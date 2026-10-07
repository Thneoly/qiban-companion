//! T11 companion voice turn: speech as a skin over the real chat chain.
//! Transcribe and synthesize reuse the lab probe's SpeechClient; the chat leg
//! stays inside chat_generate so memory injection, history and cancellation
//! are inherited untouched. Credentials never cross IPC.
use crate::model_settings::{identifier, ModelState, ModelStore, VoiceConfig};
use crate::voice::{wav_format, wav_seconds, SpeechClient};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, ipc::Response, State};
use tokio::sync::watch;

// Server "du dudu" tone signature, measured 2026-10-07
// (docs/status/voice-real-chain.md): three constant-amplitude segments around
// 0-120 / 280-660 / 1340-1480 ms separated by silence, prefixing the speech.
// Every threshold lives here; if the provider signature changes, trimming
// simply stops matching and the audio is returned untouched.
const FRAME_MS: usize = 20;
const WINDOW_FRAMES: usize = 130; // analyse at most the first 2.6 s
const SPAN_FRAMES: usize = 85; // the three segments end within 1.7 s
const HEAD_FRAMES: usize = 2; // allowed leading silence before segment one
const SILENCE_RMS: f64 = 3e-3;
const MIN_SEGMENTS: [usize; 3] = [4, 15, 5];
const MAX_SEGMENTS: [usize; 3] = [10, 25, 12];
const MIN_GAP_FRAMES: usize = 3;
const CONSTANT_CV: f64 = 0.15; // steady tone: frame-RMS variation below this
const SPEECH_CV: f64 = 0.3; // natural speech varies more than this
const SPEECH_PAD_FRAMES: usize = 3; // keep 60 ms before detected speech

/// In-flight voice requests grouped by conversation turn. Unlike the lab's
/// single-slot VoiceState, a turn may hold several TTS requests at once; the
/// registry bounds total concurrency and cancels per turn.
type TurnMap = HashMap<String, Vec<(String, watch::Sender<bool>)>>;
#[derive(Default)]
pub struct VoiceTurnState(Mutex<TurnMap>);
const MAX_INFLIGHT: usize = 12;

/// Removes exactly one registration; a stale guard dropping late must not
/// clear a newer entry under the same turn.
struct Inflight<'a> {
    state: &'a VoiceTurnState,
    turn_id: String,
    request_id: String,
}
impl Drop for Inflight<'_> {
    fn drop(&mut self) {
        if let Ok(mut map) = self.state.0.lock() {
            if let Some(list) = map.get_mut(&self.turn_id) {
                if let Some(position) = list.iter().position(|(id, _)| id == &self.request_id) {
                    list.swap_remove(position);
                }
            }
            if map.get(&self.turn_id).is_some_and(|list| list.is_empty()) {
                map.remove(&self.turn_id);
            }
        }
    }
}
impl VoiceTurnState {
    fn register(
        &self,
        turn_id: &str,
        request_id: &str,
    ) -> Result<(Inflight<'_>, watch::Receiver<bool>), String> {
        let mut map = self.0.lock().map_err(|_| "语音状态不可用")?;
        if map.values().map(Vec::len).sum::<usize>() >= MAX_INFLIGHT {
            return Err("语音请求过多，请先停止当前朗读".into());
        }
        let (signal, cancelled) = watch::channel(false);
        map.entry(turn_id.to_string())
            .or_default()
            .push((request_id.to_string(), signal));
        Ok((
            Inflight {
                state: self,
                turn_id: turn_id.to_string(),
                request_id: request_id.to_string(),
            },
            cancelled,
        ))
    }
}

/// Trim the provider's leading tone only when the measured three-segment
/// signature matches exactly; any doubt keeps the original bytes.
/// `first_sentence` keeps the tone on purpose: on the first clip it works as
/// the "about to speak" notification instead of masking latency.
fn trim_leading_tone(wav: &[u8], first_sentence: bool) -> (Cow<'_, [u8]>, bool) {
    let keep = || (Cow::Borrowed(wav), false);
    if first_sentence {
        return keep();
    }
    let format = match wav_format(wav) {
        Ok(format) => format,
        Err(_) => return keep(),
    };
    let frame_bytes = format.byte_rate * FRAME_MS / 1000;
    if frame_bytes == 0 || frame_bytes % format.block != 0 {
        return keep();
    }
    let data_end = format.data_offset + format.data_len;
    let data = &wav[format.data_offset..data_end];
    let frames = (data.len() / frame_bytes).min(WINDOW_FRAMES);
    if frames < MIN_SEGMENTS.iter().sum::<usize>() + MIN_GAP_FRAMES * 2 {
        return keep();
    }
    // Per-frame RMS normalised to 0..1.
    let mut rms = Vec::with_capacity(frames);
    for frame in 0..frames {
        let chunk = &data[frame * frame_bytes..(frame + 1) * frame_bytes];
        let (sum, count) = chunk.chunks_exact(2).fold((0.0f64, 0.0f64), |(s, n), p| {
            let sample = i16::from_le_bytes([p[0], p[1]]) as f64 / 32768.0;
            (s + sample * sample, n + 1.0)
        });
        rms.push((sum / count).sqrt());
    }
    // Maximal runs of silent / non-silent frames.
    let mut runs: Vec<(usize, usize, bool)> = Vec::new();
    for (index, value) in rms.iter().enumerate() {
        let silent = *value < SILENCE_RMS;
        match runs.last_mut() {
            Some((_, end, kind)) if *kind == silent => *end = index + 1,
            _ => runs.push((index, index + 1, silent)),
        }
    }
    let kind_of = |run: (usize, usize, bool)| -> u8 {
        if run.2 {
            return 0; // silence
        }
        let values = &rms[run.0..run.1];
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        if mean <= 0.0 {
            return 3; // ambiguous
        }
        let variance =
            values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / values.len() as f64;
        let cv = variance.sqrt() / mean;
        if cv < CONSTANT_CV {
            1 // constant tone
        } else if cv > SPEECH_CV {
            2 // natural speech
        } else {
            3
        }
    };
    let mut cursor = 0;
    if runs
        .first()
        .is_some_and(|run| run.2 && run.1 - run.0 > HEAD_FRAMES)
    {
        return keep();
    }
    if runs.first().is_some_and(|run| run.2) {
        cursor = 1;
    }
    // The three tone segments, each followed by a real silent gap.
    let mut segment_end = 0;
    for (index, (min, max)) in MIN_SEGMENTS.iter().zip(MAX_SEGMENTS.iter()).enumerate() {
        let Some(run) = runs.get(cursor) else {
            return keep();
        };
        let length = run.1 - run.0;
        if kind_of(*run) != 1 || length < *min || length > *max {
            return keep();
        }
        segment_end = run.1;
        if index == 2 {
            break;
        }
        let Some(gap) = runs.get(cursor + 1) else {
            return keep();
        };
        let Some(next) = runs.get(cursor + 2) else {
            return keep();
        };
        if kind_of(*gap) != 0 || gap.1 - gap.0 < MIN_GAP_FRAMES || kind_of(*next) != 1 {
            return keep();
        }
        cursor += 2;
    }
    if segment_end > SPAN_FRAMES {
        return keep();
    }
    // After the tone: silence is fine, but the first sound must look like
    // speech (variable envelope); another steady segment means the signature
    // does not match this clip.
    cursor += 1;
    let mut speech_start = None;
    while let Some(run) = runs.get(cursor) {
        if !run.2 {
            if kind_of(*run) == 2 {
                speech_start = Some(run.0);
            }
            break;
        }
        cursor += 1;
    }
    let Some(speech_start) = speech_start else {
        return keep();
    };
    let trim_frame = speech_start
        .saturating_sub(SPEECH_PAD_FRAMES)
        .max(segment_end);
    let trim_offset = format.data_offset + trim_frame * frame_bytes;
    if trim_offset <= format.data_offset || trim_offset >= data_end {
        return keep();
    }
    // Rebuild: original header and any ancillary chunks stay, the data chunk
    // size field is rewritten, payload starts at the trim point.
    let mut out = Vec::with_capacity(wav.len() - (trim_offset - format.data_offset));
    out.extend_from_slice(&wav[..format.data_offset - 4]);
    out.extend_from_slice(&((data_end - trim_offset) as u32).to_le_bytes());
    out.extend_from_slice(&wav[trim_offset..]);
    let total = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&total.to_le_bytes());
    if wav_seconds(&out, 60.0).is_err() {
        return keep();
    }
    (Cow::Owned(out), true)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscribeRequest {
    request_id: String,
    turn_id: String,
    expected_base_url: String,
    wav: Vec<u8>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeResult {
    request_id: String,
    transcript: String,
    recognition_ms: u128,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeakRequest {
    request_id: String,
    turn_id: String,
    expected_base_url: String,
    seq: u32,
    text: String,
    first: bool,
}
/// `None` means the first clip (tone kept on purpose); `Some(trimmed)` reports
/// whether the trim matched for later clips.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SpeakProgress {
    request_id: String,
    trimmed: Option<bool>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
/// The pet window has no form: the saved voice config is the only source of
/// truth, and the caller's expected base URL guards mid-turn config changes.
fn speech_client(
    settings: &Mutex<ModelStore>,
    expected_base_url: &str,
) -> Result<(SpeechClient, VoiceConfig), String> {
    let voice = settings
        .lock()
        .map_err(|_| "模型设置不可用")?
        .voice_config();
    if voice.voice_base_url.is_empty()
        || !identifier(&voice.asr_model)
        || !identifier(&voice.tts_model)
        || !identifier(&voice.voice)
    {
        return Err("语音服务未配置，请在任务面板的语音实验里保存语音设置".into());
    }
    if voice.voice_base_url != expected_base_url {
        return Err("语音配置已变化，请重试本轮".into());
    }
    let key = if voice.use_voice_key {
        crate::credentials::read(&voice.voice_base_url)?
            .ok_or("请先在任务面板的语音实验里设置语音密钥")?
            .to_string()
    } else {
        String::new()
    };
    let client = SpeechClient::new(voice.voice_base_url.clone(), key)?;
    Ok((client, voice))
}

async fn run_transcribe(
    state: &VoiceTurnState,
    settings: &Mutex<ModelStore>,
    request: TranscribeRequest,
) -> Result<TranscribeResult, String> {
    if !valid_id(&request.request_id) || !valid_id(&request.turn_id) {
        return Err("语音请求标识无效".into());
    }
    wav_seconds(&request.wav, 30.0)?;
    let (client, voice) = speech_client(settings, &request.expected_base_url)?;
    let inflight = state.register(&request.turn_id, &request.request_id)?;
    let (_guard, mut cancelled) = inflight;
    let asr_model = voice.asr_model.clone();
    let request_id = request.request_id.clone();
    let work = async {
        let started = Instant::now();
        let transcript = client.transcribe(&asr_model, request.wav).await?;
        Ok(TranscribeResult {
            request_id,
            transcript,
            recognition_ms: started.elapsed().as_millis(),
        })
    };
    tokio::select! {
        biased;
        _ = cancelled.changed() => Err("语音已停止；在途调用可能计费".into()),
        result = tokio::time::timeout(Duration::from_secs(60), work) => {
            result.map_err(|_| "语音识别超过60秒时限".to_string())?
        }
    }
}

async fn run_speak<P>(
    state: &VoiceTurnState,
    settings: &Mutex<ModelStore>,
    request: SpeakRequest,
    progress: P,
) -> Result<Vec<u8>, String>
where
    P: Fn(Option<bool>) -> Result<(), String>,
{
    if !valid_id(&request.request_id) || !valid_id(&request.turn_id) {
        return Err("语音请求标识无效".into());
    }
    if request.seq > 64 {
        return Err("语音句子序号超出范围".into());
    }
    let trimmed_text = request.text.trim();
    if trimmed_text.is_empty() || trimmed_text.chars().count() > 500 {
        return Err("语音句子长度需在1～500字之间".into());
    }
    let (client, voice) = speech_client(settings, &request.expected_base_url)?;
    let inflight = state.register(&request.turn_id, &request.request_id)?;
    let (_guard, mut cancelled) = inflight;
    let tts_model = voice.tts_model.clone();
    let speaker = voice.voice.clone();
    let first = request.first;
    let text = trimmed_text.to_string();
    let work = async {
        let (wav, _) = client.synthesize(&tts_model, &speaker, &text).await?;
        let (wav, trimmed) = trim_leading_tone(&wav, first);
        progress(if first { None } else { Some(trimmed) })?;
        Ok(wav.into_owned())
    };
    tokio::select! {
        biased;
        _ = cancelled.changed() => Err("已停止；在途调用可能计费".into()),
        result = tokio::time::timeout(Duration::from_secs(60), work) => {
            result.map_err(|_| "语音合成超过60秒时限".to_string())?
        }
    }
}

#[tauri::command]
pub async fn voice_transcribe(
    state: State<'_, VoiceTurnState>,
    settings: State<'_, ModelState>,
    request: TranscribeRequest,
) -> Result<TranscribeResult, String> {
    run_transcribe(state.inner(), settings.inner(), request).await
}
#[tauri::command]
pub async fn voice_speak(
    state: State<'_, VoiceTurnState>,
    settings: State<'_, ModelState>,
    request: SpeakRequest,
    on_progress: Channel<SpeakProgress>,
) -> Result<Response, String> {
    let request_id = request.request_id.clone();
    run_speak(state.inner(), settings.inner(), request, |trimmed| {
        on_progress
            .send(SpeakProgress {
                request_id: request_id.clone(),
                trimmed,
            })
            .map_err(|_| "语音窗口已断开".into())
    })
    .await
    .map(Response::new)
}
/// Signals every in-flight request of one turn; entry removal stays with the
/// RAII guards, so late drops of cancelled requests still clean up correctly.
fn cancel_all(state: &VoiceTurnState, turn_id: &str) -> Result<(), String> {
    let map = state.0.lock().map_err(|_| "语音状态不可用")?;
    if let Some(entries) = map.get(turn_id) {
        for (_, signal) in entries {
            let _ = signal.send(true);
        }
    }
    Ok(())
}
#[tauri::command]
pub fn voice_turn_cancel(state: State<'_, VoiceTurnState>, turn_id: String) -> Result<(), String> {
    cancel_all(state.inner(), &turn_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug)]
    enum Seg {
        Silence,
        Tone,
        Speech,
    }
    /// Synthetic 16 kHz mono PCM16 WAV: steady tone segments have a constant
    /// frame RMS, speech varies its amplitude across frames (CV above 0.3).
    fn build_wav(segments: &[(Seg, usize)], ancillary: bool) -> Vec<u8> {
        const RATE: usize = 16000;
        let frame = RATE / 50;
        let mut samples: Vec<i16> = Vec::new();
        for (kind, frames) in segments {
            for f in 0..*frames {
                let amp = match kind {
                    Seg::Silence => 0.0,
                    Seg::Tone => 0.5,
                    Seg::Speech => 0.2 + 0.8 * ((f % 5) as f64 / 4.0),
                };
                for n in 0..frame {
                    let phase = (n as f64 / frame as f64) * std::f64::consts::TAU;
                    samples.push((amp * 0.9 * phase.sin() * 32767.0) as i16);
                }
            }
        }
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        // "AIGC" + size field + 2-byte payload = 10 extra bytes over the base 36.
        let anc = if ancillary { 10 } else { 0 };
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data.len() + anc) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(RATE as u32).to_le_bytes());
        out.extend_from_slice(&((RATE * 2) as u32).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        if ancillary {
            out.extend_from_slice(b"AIGC");
            out.extend_from_slice(&2u32.to_le_bytes());
            out.extend_from_slice(b"{}");
        }
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }
    fn layout(s1: usize, s2: usize, s3: usize, gap_tail: usize) -> Vec<(Seg, usize)> {
        vec![
            (Seg::Tone, s1),
            (Seg::Silence, 8),
            (Seg::Tone, s2),
            (Seg::Silence, 34),
            (Seg::Tone, s3),
            (Seg::Silence, gap_tail),
            (Seg::Speech, 30),
        ]
    }

    #[test]
    fn trims_three_tone_prefix_and_rewrites_sizes() {
        // Measured signature: 6/19/7 frames at 20 ms → third segment ends at
        // frame 74; speech starts at 77, so the trim keeps 60 ms of pad.
        let wav = build_wav(&layout(6, 19, 7, 3), false);
        let (trimmed, did) = trim_leading_tone(&wav, false);
        assert!(did);
        let trimmed = trimmed.into_owned();
        assert!(wav_seconds(&trimmed, 60.0).is_ok());
        assert_eq!(trimmed.len(), wav.len() - 74 * 640);
        // Ancillary chunks before data survive the rewrite untouched; only
        // the RIFF size field (bytes 4..8) legitimately changes.
        let marked = build_wav(&layout(6, 19, 7, 3), true);
        let (trimmed, did) = trim_leading_tone(&marked, false);
        assert!(did);
        let trimmed = trimmed.into_owned();
        assert_eq!(&trimmed[..4], &marked[..4]);
        assert_eq!(&trimmed[8..40], &marked[8..40]);
        let size = u32::from_le_bytes(trimmed[4..8].try_into().unwrap()) as usize;
        assert_eq!(size, trimmed.len() - 8);
        assert!(trimmed.windows(4).any(|w| w == b"AIGC"));
        assert!(wav_seconds(&trimmed, 60.0).is_ok());
    }
    #[test]
    fn keeps_audio_when_the_signature_does_not_match() {
        let mut long_gap = Vec::new();
        long_gap.extend_from_slice(&layout(6, 19, 7, 3));
        // Third segment would end at frame 86, past the 1.7 s span bound.
        long_gap[3] = (Seg::Silence, 46);
        let mut leading_silence = vec![(Seg::Silence, 3)];
        leading_silence.extend(layout(6, 19, 7, 3));
        let mut fourth_steady = Vec::new();
        fourth_steady.extend(layout(6, 19, 7, 3).into_iter().take(6));
        fourth_steady.extend_from_slice(&[(Seg::Tone, 6), (Seg::Silence, 3), (Seg::Speech, 30)]);
        for segments in [
            vec![(Seg::Speech, 30)], // speech from the start
            layout(6, 19, 7, 3).into_iter().take(5).collect::<Vec<_>>(), // only two tone segments
            vec![(Seg::Speech, 3)],  // shorter than any window
            layout(3, 19, 7, 3),     // first segment too short
            layout(11, 19, 7, 3),    // first segment too long
            layout(6, 14, 7, 3),     // second segment too short
            layout(6, 26, 7, 3),     // second segment too long
            layout(6, 19, 13, 3),    // third segment too long
            long_gap,                // third segment ends past 1.7 s
            leading_silence,         // more than two leading silence frames
            fourth_steady,           // a steady segment where speech should start
        ] {
            let wav = build_wav(&segments, false);
            let (trimmed, did) = trim_leading_tone(&wav, false);
            assert!(!did, "{segments:?}");
            assert_eq!(trimmed.into_owned(), wav);
        }
    }
    #[test]
    fn first_sentence_never_trims() {
        let wav = build_wav(&layout(6, 19, 7, 3), false);
        let (trimmed, did) = trim_leading_tone(&wav, true);
        assert!(!did);
        assert_eq!(trimmed.into_owned(), wav);
    }
    #[test]
    fn trims_at_tolerance_bounds() {
        let wav = build_wav(&layout(4, 15, 5, 3), false);
        let (trimmed, did) = trim_leading_tone(&wav, false);
        assert!(did);
        assert!(wav_seconds(&trimmed, 60.0).is_ok());
        // Up to two leading silence frames are tolerated before segment one.
        let mut with_head = vec![(Seg::Silence, 2)];
        with_head.extend(layout(6, 19, 7, 3));
        let (_, did) = trim_leading_tone(&build_wav(&with_head, false), false);
        assert!(did);
    }

    #[tokio::test]
    async fn registry_cancels_only_its_turn_and_caps_inflight() {
        let state = VoiceTurnState::default();
        let mut guards = Vec::new();
        for index in 0..MAX_INFLIGHT {
            guards.push(state.register("t", &format!("r{index}")).unwrap());
        }
        assert!(state.register("t", "overflow").is_err());
        drop(guards);
        assert!(state.0.lock().unwrap().is_empty());
        let (_a, mut cancelled_a) = state.register("a", "x").unwrap();
        let (guard_b, mut cancelled_b) = state.register("b", "y").unwrap();
        cancel_all(&state, "a").unwrap();
        assert!(cancelled_a.changed().await.is_ok());
        assert!(*cancelled_a.borrow_and_update());
        // Turn b is untouched; its receiver only ends once its guard drops.
        drop(guard_b);
        assert!(cancelled_b.changed().await.is_err());
        assert_eq!(state.0.lock().unwrap().len(), 1);
    }
    #[test]
    fn guard_cleanup_cannot_remove_newer_entry() {
        let state = VoiceTurnState::default();
        let (older, _) = state.register("t", "x").unwrap();
        let (newer, _) = state.register("t", "y").unwrap();
        drop(older);
        {
            let map = state.0.lock().unwrap();
            let list = map.get("t").unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].0, "y");
        }
        drop(newer);
        assert!(state.0.lock().unwrap().is_empty());
    }

    fn store_with_voice(base: &str) -> (std::path::PathBuf, Mutex<ModelStore>) {
        let path =
            std::env::temp_dir().join(format!("voice-companion-{}.db", uuid::Uuid::new_v4()));
        let mut store = ModelStore::open(&path).unwrap();
        store
            .save_voice_config(VoiceConfig {
                voice_base_url: base.into(),
                use_voice_key: false,
                asr_model: "custom-asr".into(),
                tts_model: "custom-tts".into(),
                voice: "custom-voice".into(),
            })
            .unwrap();
        (path, Mutex::new(store))
    }
    /// Reads one full HTTP request (headers + content-length body) from the socket.
    fn read_request(socket: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let size = socket.read(&mut buffer).unwrap();
            assert!(size > 0);
            request.extend_from_slice(&buffer[..size]);
            if let Some(end) = request.windows(4).position(|s| s == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..end]);
                if let Some(length) = header.lines().find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse::<usize>().unwrap())
                }) {
                    if request.len() >= end + 4 + length {
                        return String::from_utf8_lossy(&request).into_owned();
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn transcribe_over_http_uses_saved_config_and_maps_errors() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fixture = build_wav(&[(Seg::Speech, 30)], false);
        let server = std::thread::spawn(move || {
            for step in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                let raw = read_request(&mut socket);
                assert!(raw.starts_with("POST /audio/transcriptions "));
                assert!(raw.contains("custom-asr"));
                assert!(!raw.contains("Bearer"));
                let body = if step == 0 {
                    r#"{"text":"你好栖栖"}"#.as_bytes().to_vec()
                } else {
                    Vec::new()
                };
                let status = if step == 0 {
                    "200 OK"
                } else {
                    "401 Unauthorized"
                };
                let extra = if step == 0 {
                    String::new()
                } else {
                    format!("Content-Length: {}\r\n", body.len())
                };
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\n{extra}Connection: close\r\n\r\n"
                )
                .unwrap();
                if step == 0 {
                    socket.write_all(&body).unwrap();
                }
            }
        });
        let (path, settings) = store_with_voice(&base);
        let state = VoiceTurnState::default();
        let result = run_transcribe(
            &state,
            &settings,
            TranscribeRequest {
                request_id: "turn-1".into(),
                turn_id: "turn".into(),
                expected_base_url: base.clone(),
                wav: fixture,
            },
        )
        .await
        .unwrap();
        assert_eq!(result.transcript, "你好栖栖");
        let error = run_transcribe(
            &state,
            &settings,
            TranscribeRequest {
                request_id: "turn-2".into(),
                turn_id: "turn".into(),
                expected_base_url: base.clone(),
                wav: build_wav(&[(Seg::Speech, 10)], false),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("HTTP 401"));
        assert!(!error.contains("secret"));
        server.join().unwrap();
        drop(settings); // close SQLite before the Windows file delete
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn speak_returns_trimmed_later_clips_and_reports_progress() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fixture = build_wav(&layout(6, 19, 7, 3), false);
        let expected = fixture.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                let raw = read_request(&mut socket);
                assert!(raw.starts_with("POST /audio/speech "));
                assert!(raw.contains("custom-tts"));
                assert!(raw.contains("custom-voice"));
                assert!(raw.contains("第一句话"));
                let body = expected.clone();
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                socket.write_all(&body).unwrap();
            }
        });
        let (path, settings) = store_with_voice(&base);
        let state = VoiceTurnState::default();
        let seen = Mutex::new(Vec::new());
        let first = run_speak(
            &state,
            &settings,
            SpeakRequest {
                request_id: "turn-0".into(),
                turn_id: "turn".into(),
                expected_base_url: base.clone(),
                seq: 0,
                text: "第一句话".into(),
                first: true,
            },
            |flag| {
                seen.lock().unwrap().push(flag);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(first, fixture);
        let later = run_speak(
            &state,
            &settings,
            SpeakRequest {
                request_id: "turn-1".into(),
                turn_id: "turn".into(),
                expected_base_url: base,
                seq: 1,
                text: "第一句话".into(),
                first: false,
            },
            |flag| {
                seen.lock().unwrap().push(flag);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(later.len(), fixture.len() - 74 * 640);
        assert!(later.len() < first.len());
        assert_eq!(*seen.lock().unwrap(), vec![None, Some(true)]);
        server.join().unwrap();
        drop(settings); // close SQLite before the Windows file delete
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn turn_cancel_stops_inflight_requests_and_the_registry_drains() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let wave = build_wav(&[(Seg::Speech, 5)], false);
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                let _ = read_request(&mut socket);
                // Hold the response long enough for the test to cancel first.
                std::thread::sleep(Duration::from_millis(800));
                let _ = write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    wave.len()
                );
                let _ = socket.write_all(&wave);
            }
        });
        let (path, settings) = store_with_voice(&base);
        let state = VoiceTurnState::default();
        async fn speak_turn(
            state: &VoiceTurnState,
            settings: &Mutex<ModelStore>,
            base: &str,
            turn: &str,
        ) -> Result<Vec<u8>, String> {
            run_speak(
                state,
                settings,
                SpeakRequest {
                    request_id: format!("{turn}-0"),
                    turn_id: turn.into(),
                    expected_base_url: base.to_string(),
                    seq: 0,
                    text: "被取消的句子".into(),
                    first: true,
                },
                |_| Ok(()),
            )
            .await
        }
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            cancel_all(&state, "a").unwrap();
            cancel_all(&state, "b").unwrap();
        };
        let (a, b, ()) = tokio::join!(
            speak_turn(&state, &settings, &base, "a"),
            speak_turn(&state, &settings, &base, "b"),
            cancel
        );
        assert!(a.unwrap_err().contains("已停止"));
        assert!(b.unwrap_err().contains("已停止"));
        for _ in 0..50 {
            if state.0.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(state.0.lock().unwrap().is_empty());
        server.join().unwrap();
        drop(settings); // close SQLite before the Windows file delete
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn speak_rejects_invalid_payloads_without_any_request() {
        let (path, settings) = store_with_voice("https://voice.example.invalid");
        let state = VoiceTurnState::default();
        let cases = [
            SpeakRequest {
                request_id: "t-0".into(),
                turn_id: "t".into(),
                expected_base_url: "https://voice.example.invalid".into(),
                seq: 0,
                text: "x".repeat(501),
                first: true,
            },
            SpeakRequest {
                request_id: "t-1".into(),
                turn_id: "t".into(),
                expected_base_url: "https://voice.example.invalid".into(),
                seq: 0,
                text: "   ".into(),
                first: true,
            },
            SpeakRequest {
                request_id: "t-2".into(),
                turn_id: "t".into(),
                expected_base_url: "https://voice.example.invalid".into(),
                seq: 65,
                text: "句子".into(),
                first: false,
            },
            SpeakRequest {
                request_id: "t-3".into(),
                turn_id: "t".into(),
                expected_base_url: "https://changed.example".into(),
                seq: 0,
                text: "句子".into(),
                first: false,
            },
        ];
        for case in cases {
            assert!(run_speak(&state, &settings, case, |_| Ok(()))
                .await
                .is_err());
        }
        assert!(state.0.lock().unwrap().is_empty());
        drop(settings); // close SQLite before the Windows file delete
        std::fs::remove_file(path).unwrap();
    }
}

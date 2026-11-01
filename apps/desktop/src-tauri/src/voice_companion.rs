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
const SILENCE_RMS: f64 = 3e-3;
const MIN_SEGMENTS: [usize; 3] = [4, 15, 5];
const MAX_SEGMENTS: [usize; 3] = [10, 25, 12];
const MIN_GAP_FRAMES: usize = 3;
const CONSTANT_CV: f64 = 0.15; // steady tone: frame-RMS variation below this
const SPEECH_CV: f64 = 0.3; // natural speech varies more than this
const SPEECH_PAD_FRAMES: usize = 3; // keep 60 ms before detected speech

// Energy bands shared by the burst pass and the tone-present verdict: real
// speech bursts clearly above BURST_RMS; the provider's tone (steady or
// wobbling) lives inside TONE_BAND, disjoint from the burst band.
const BURST_RMS: f64 = 0.045;
const TONE_BAND: (f64, f64) = (0.015, 0.042);

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

/// What the leading-tone analysis concluded about one clip. The distinction
/// feeds the frontend's single-shot fallback (TUWE-6): a provider that no
/// longer prefixes a tone returns `Absent` on every clip, which is NOT a
/// trim failure — only `Missed` (tone-like material heard, no pass could
/// safely remove it) may count against per-sentence synthesis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneVerdict {
    /// Tone found and removed.
    Trimmed,
    /// Tone-like leading material detected but untrimmable.
    Missed,
    /// No leading tone detected — nothing to trim, not a failure.
    Absent,
}

/// Trim the provider's leading tone on EVERY clip. The tone once doubled as
/// an "about to speak" notification on the first clip, but trial feedback
/// (twice) was that users hear it as noise — so the exemption is gone and
/// only the must-find-speech guard remains: any doubt keeps the original
/// bytes, we never trim into actual speech.
fn trim_leading_tone(wav: &[u8]) -> (Cow<'_, [u8]>, ToneVerdict) {
    let keep = || (Cow::Borrowed(wav), ToneVerdict::Absent);
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
    // Leading silence (any length) simply precedes whatever comes next; the
    // passes below decide on tone/speech shape alone. An early "too much
    // leading silence" bail used to shield the exact signature — it also
    // blocked the loose pass on clips that merely started quietly.
    let mut cursor = usize::from(runs.first().is_some_and(|run| run.2));
    // Exact pass: the three known tone segments, each followed by a real
    // silent gap. Any mismatch falls through to the looser second pass —
    // the provider's tone shape has drifted on real machines before (every
    // sentence beeped again), so an exact-signature-only trim is a
    // regression waiting for the next drift.
    let mut segment_end = 0;
    let mut exact = true;
    'exact: for (index, (min, max)) in MIN_SEGMENTS.iter().zip(MAX_SEGMENTS.iter()).enumerate() {
        let Some(run) = runs.get(cursor) else {
            exact = false;
            break 'exact;
        };
        let length = run.1 - run.0;
        if kind_of(*run) != 1 || length < *min || length > *max {
            exact = false;
            break 'exact;
        }
        segment_end = run.1;
        if index == 2 {
            break;
        }
        let (Some(gap), Some(next)) = (runs.get(cursor + 1), runs.get(cursor + 2)) else {
            exact = false;
            break 'exact;
        };
        if kind_of(*gap) != 0 || gap.1 - gap.0 < MIN_GAP_FRAMES || kind_of(*next) != 1 {
            exact = false;
            break 'exact;
        }
        cursor += 2;
    }
    let mut trim_frame = None;
    if exact && segment_end <= SPAN_FRAMES {
        // After the tone: silence is fine, but the first sound must look
        // like speech (variable envelope); another steady segment means the
        // signature does not match this clip.
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
        if let Some(speech_start) = speech_start {
            trim_frame = Some(
                speech_start
                    .saturating_sub(SPEECH_PAD_FRAMES)
                    .max(segment_end),
            );
        }
    }
    // Loose pass: a leading run of constant-tone segments totalling 60 ms–1 s
    // ms with real energy, then a silent gap, then natural speech. The
    // must-find-speech guard keeps a pure-tone clip (or a shouted lead-in
    // with no gap after it) untouched.
    if trim_frame.is_none() {
        let mut cursor = usize::from(runs.first().is_some_and(|run| run.2));
        let mut tone_frames = 0usize;
        let mut tone_end = 0usize;
        let mut energetic = false;
        while let Some(run) = runs.get(cursor) {
            if kind_of(*run) != 1 {
                break;
            }
            let length = run.1 - run.0;
            let mean = rms[run.0..run.1].iter().sum::<f64>() / length as f64;
            energetic |= mean >= 0.01;
            tone_frames += length;
            tone_end = run.1;
            cursor += 1;
            if let Some(gap) = runs.get(cursor) {
                if gap.2 && gap.1 - gap.0 >= MIN_GAP_FRAMES {
                    cursor += 1;
                    continue;
                }
            }
            break;
        }
        if energetic && (3..=50).contains(&tone_frames) {
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
            if let Some(speech_start) = speech_start {
                trim_frame = Some(speech_start.saturating_sub(SPEECH_PAD_FRAMES).max(tone_end));
            }
        }
        // Diagnosis: an energetic steady segment survived the passes —
        // dump the frame kinds so the next report ships the provider's
        // actual tone shape instead of another guess.
        if trim_frame.is_none() && runs.iter().take(30).any(|run| kind_of(*run) == 1) {
            let kinds: Vec<u8> = runs.iter().take(24).map(|run| kind_of(*run)).collect();
            eprintln!("qiban voice: leading tone survived both passes; frame kinds {kinds:?}");
        }
    }
    // Burst pass (2026-10-24, second drift): the provider's tone now comes
    // as FOUR mid-energy segments whose CV can exceed the speech threshold
    // (measured 0.37 on segments one and four), so shape-based passes miss
    // it. The stable discriminator is ENERGY: real speech bursts clearly
    // above the tone band; every tone segment measured 0.033-0.038. Find
    // the first substantial burst; tone-band material before it reads as
    // the leading tone, whatever its shape. 2026-10-25: the burst bar
    // dropped 0.06 -> 0.045 because a LONG clip's first speech run can
    // start quietly (measured just under 0.06), which pushed the "first
    // burst" deep past the lead cap — that clip's tone survived and was
    // heard as a beep in the MIDDLE of a turn. The lead-band window
    // (0.015..=0.042) keeps burst and tone bands disjoint.
    if trim_frame.is_none() {
        let burst = runs.iter().find(|run| {
            let length = run.1 - run.0;
            if run.2 || length < 4 {
                return false;
            }
            let values = &rms[run.0..run.1];
            let mean = values.iter().sum::<f64>() / length as f64;
            mean >= BURST_RMS
        });
        if let Some(burst) = burst {
            let lead = runs.iter().take_while(|run| run.0 < burst.0).any(|run| {
                let length = run.1 - run.0;
                if run.2 || length < 2 {
                    return false;
                }
                let mean = rms[run.0..run.1].iter().sum::<f64>() / length as f64;
                (TONE_BAND.0..=TONE_BAND.1).contains(&mean)
            });
            if lead && burst.0 <= 110 {
                trim_frame = Some(burst.0.saturating_sub(SPEECH_PAD_FRAMES).max(1));
            }
        }
    }
    let Some(trim_frame) = trim_frame else {
        // Every pass refused to trim. Split "tone survived" from "no tone":
        // the fallback verdict is only evidence against per-sentence
        // synthesis when tone-like material is actually present. Two signals,
        // complementary by construction — a constant-shape (kind 1) run early
        // in the clip catches steady tones at any energy (the loud original
        // signature is itself a "burst", so the band check cannot see it),
        // while tone-band material before the first speech burst catches the
        // wobbly mid-energy drift whose shape defeats the CV classes.
        let run_mean = |run: (usize, usize, bool)| {
            rms[run.0..run.1].iter().sum::<f64>() / (run.1 - run.0) as f64
        };
        let first_burst = runs
            .iter()
            .find(|run| !run.2 && run.1 - run.0 >= 4 && run_mean(**run) >= BURST_RMS);
        let tone_band_lead = first_burst.is_some_and(|burst| {
            runs.iter().take_while(|run| run.0 < burst.0).any(|run| {
                !run.2
                    && run.1 - run.0 >= 2
                    && (TONE_BAND.0..=TONE_BAND.1).contains(&run_mean(*run))
            })
        });
        let tone_present = tone_band_lead || runs.iter().take(30).any(|run| kind_of(*run) == 1);
        return (
            Cow::Borrowed(wav),
            if tone_present {
                ToneVerdict::Missed
            } else {
                ToneVerdict::Absent
            },
        );
    };
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
    (Cow::Owned(out), ToneVerdict::Trimmed)
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
}
/// Reports the leading-tone verdict for this clip. The old
/// `None` variant meant "first clip, tone kept on purpose" — that exemption
/// was removed after trial feedback. `tone_detected` distinguishes a trim
/// failure (tone present, `trimmed=false`) from a tone-free clip (nothing to
/// trim), so the frontend fallback only engages on real misses (TUWE-6).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SpeakProgress {
    request_id: String,
    trimmed: bool,
    tone_detected: bool,
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
    P: Fn(ToneVerdict) -> Result<(), String>,
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
    let text = trimmed_text.to_string();
    let work = async {
        let (wav, _) = client.synthesize(&tts_model, &speaker, &text).await?;
        let (wav, trimmed) = trim_leading_tone(&wav);
        progress(trimmed)?;
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
    run_speak(state.inner(), settings.inner(), request, |verdict| {
        on_progress
            .send(SpeakProgress {
                request_id: request_id.clone(),
                trimmed: verdict == ToneVerdict::Trimmed,
                tone_detected: verdict != ToneVerdict::Absent,
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
    if !valid_id(&turn_id) {
        return Err("语音轮次标识无效".into());
    }
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
        /// 2026-10-24 drift: mid-energy tone (~0.035 frame RMS), steady.
        SoftTone,
        /// Same energy but wobbling — its measured CV reached 0.37, past the
        /// speech threshold, which is exactly how the shape passes beat it.
        WobblyTone,
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
                    Seg::SoftTone => 0.055,
                    Seg::WobblyTone => 0.03 + 0.05 * ((f % 4) as f64 / 3.0),
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
        let (trimmed, did) = trim_leading_tone(&wav);
        assert_eq!(did, ToneVerdict::Trimmed);
        let trimmed = trimmed.into_owned();
        assert!(wav_seconds(&trimmed, 60.0).is_ok());
        assert_eq!(trimmed.len(), wav.len() - 74 * 640);
        // Ancillary chunks before data survive the rewrite untouched; only
        // the RIFF size field (bytes 4..8) legitimately changes.
        let marked = build_wav(&layout(6, 19, 7, 3), true);
        let (trimmed, did) = trim_leading_tone(&marked);
        assert_eq!(did, ToneVerdict::Trimmed);
        let trimmed = trimmed.into_owned();
        assert_eq!(&trimmed[..4], &marked[..4]);
        assert_eq!(&trimmed[8..40], &marked[8..40]);
        let size = u32::from_le_bytes(trimmed[4..8].try_into().unwrap()) as usize;
        assert_eq!(size, trimmed.len() - 8);
        assert!(trimmed.windows(4).any(|w| w == b"AIGC"));
        assert!(wav_seconds(&trimmed, 60.0).is_ok());
    }
    #[test]
    fn four_mid_energy_wobbly_tone_segments_trim_via_the_burst_pass() {
        // The measured 2026-10-24 drift: four tone segments at ~0.035 frame
        // RMS (3x quieter than real speech), two of them wobbling past the
        // speech CV threshold (0.37) — every shape-based pass misses this;
        // the burst pass discriminates on energy alone.
        let wav = build_wav(
            &[
                (Seg::WobblyTone, 7),
                (Seg::Silence, 7),
                (Seg::SoftTone, 19),
                (Seg::Silence, 34),
                (Seg::SoftTone, 7),
                (Seg::Silence, 8),
                (Seg::WobblyTone, 7),
                (Seg::Silence, 8),
                (Seg::Speech, 24),
            ],
            false,
        );
        let (trimmed, did) = trim_leading_tone(&wav);
        assert_eq!(did, ToneVerdict::Trimmed, "the drifted shape must trim");
        // Everything before the speech burst (~97 frames) is gone, landing a
        // 3-frame pad before it — never mid-speech.
        let removed = (wav.len() - trimmed.len()) / 640;
        assert!((92..=97).contains(&removed), "removed {removed} frames");
        // Plain mid-energy lead-ins (no wobble) also take the burst path
        // when the shape passes refuse them.
        let wav = build_wav(
            &[(Seg::SoftTone, 10), (Seg::Silence, 10), (Seg::Speech, 24)],
            false,
        );
        let (_, did) = trim_leading_tone(&wav);
        assert_eq!(did, ToneVerdict::Trimmed, "soft steady lead-in trims too");
    }

    #[test]
    fn burst_pass_never_trims_into_speech_or_tone_only_clips() {
        // Speech from frame zero: the burst IS the first run — no lead-in,
        // nothing to trim, and no tone either: the verdict is Absent, the
        // state a tone-free provider (glm-tts since 2026-10-30) returns for
        // every clip. Absent must never read as a trim failure.
        let wav = build_wav(&[(Seg::Speech, 30)], false);
        let (trimmed, did) = trim_leading_tone(&wav);
        assert_eq!(did, ToneVerdict::Absent);
        assert_eq!(trimmed.into_owned(), wav);
        // Tone-only clip (all quiet-ish tone, no burst anywhere): the
        // must-find-speech guard holds for the burst pass too — untrimmed,
        // but the tone is unmistakably there, so the verdict is Missed.
        // Long enough to clear the 30-frame analysis floor.
        let wav = build_wav(
            &[(Seg::SoftTone, 20), (Seg::Silence, 6), (Seg::SoftTone, 20)],
            false,
        );
        let (trimmed, did) = trim_leading_tone(&wav);
        assert_eq!(did, ToneVerdict::Missed);
        assert_eq!(trimmed.into_owned(), wav);
    }

    #[test]
    fn keeps_audio_without_a_leading_tone() {
        let mut tone_without_speech = Vec::new();
        tone_without_speech.extend(layout(6, 19, 7, 3).into_iter().take(6));
        // Tone present but nothing speech-like follows: never trim (the
        // must-find-speech guard is what keeps pure-tone clips intact).
        tone_without_speech.extend_from_slice(&[
            (Seg::Tone, 6),
            (Seg::Silence, 3),
            (Seg::Silence, 40),
        ]);
        let mut tone_past_the_budget = Vec::new();
        tone_past_the_budget.extend(layout(6, 19, 7, 3).into_iter().take(6));
        // Total tone ≈ 58 frames ≈ 1.16 s: past the loose pass's 1 s
        // budget, and a steady segment sits where speech should start.
        tone_past_the_budget.extend_from_slice(&[
            (Seg::Tone, 26),
            (Seg::Silence, 3),
            (Seg::Speech, 30),
        ]);
        // Untrimmable clips split into tone-present (Missed — fallback
        // evidence) and tone-free (Absent — no evidence at all).
        for (segments, expected) in [
            (vec![(Seg::Speech, 30)], ToneVerdict::Absent), // speech from the start
            (
                layout(6, 19, 7, 3).into_iter().take(5).collect::<Vec<_>>(),
                ToneVerdict::Missed,
            ), // tone, then the clip ends
            (vec![(Seg::Speech, 3)], ToneVerdict::Absent),  // shorter than any window
            (tone_without_speech, ToneVerdict::Missed),
            (tone_past_the_budget, ToneVerdict::Missed),
        ] {
            let wav = build_wav(&segments, false);
            let (trimmed, did) = trim_leading_tone(&wav);
            assert_eq!(did, expected, "{segments:?}");
            assert_eq!(trimmed.into_owned(), wav);
        }
    }
    #[test]
    fn drifted_tone_shapes_still_trim_through_the_loose_pass() {
        // The provider's tone has drifted on real machines: segment lengths
        // outside the exact signature made EVERY sentence beep again. The
        // loose pass trims any energetic constant-tone lead-in within
        // 60–600 ms that is followed by a gap and natural speech.
        let mut leading_silence = vec![(Seg::Silence, 3)];
        leading_silence.extend(layout(6, 19, 7, 3));
        for segments in [
            layout(3, 19, 7, 3),  // first segment shorter than the signature
            layout(11, 19, 7, 3), // first segment longer
            layout(6, 14, 7, 3),  // second segment shorter
            layout(6, 26, 7, 3),  // second segment longer
            layout(6, 19, 13, 3), // third segment longer
            leading_silence,      // a little leading silence before the tone
        ] {
            let wav = build_wav(&segments, false);
            let (trimmed, did) = trim_leading_tone(&wav);
            assert_eq!(did, ToneVerdict::Trimmed, "{segments:?}");
            assert!(trimmed.len() < wav.len(), "{segments:?}");
        }
    }
    #[test]
    fn every_clip_trims_including_the_first() {
        // The first-clip exemption ("about to speak" notification) was
        // removed after trial feedback heard the tone as noise twice.
        let wav = build_wav(&layout(6, 19, 7, 3), false);
        let (trimmed, did) = trim_leading_tone(&wav);
        assert_eq!(did, ToneVerdict::Trimmed);
        assert!(trimmed.len() < wav.len());
    }
    #[test]
    fn trims_at_tolerance_bounds() {
        let wav = build_wav(&layout(4, 15, 5, 3), false);
        let (trimmed, did) = trim_leading_tone(&wav);
        assert_eq!(did, ToneVerdict::Trimmed);
        assert!(wav_seconds(&trimmed, 60.0).is_ok());
        // Up to two leading silence frames are tolerated before segment one.
        let mut with_head = vec![(Seg::Silence, 2)];
        with_head.extend(layout(6, 19, 7, 3));
        let (_, did) = trim_leading_tone(&build_wav(&with_head, false));
        assert_eq!(did, ToneVerdict::Trimmed);
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
        let plain = build_wav(&[(Seg::Speech, 30)], false); // a tone-free clip
        let expected = fixture.clone();
        let bare = plain.clone();
        let server = std::thread::spawn(move || {
            for step in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
                let raw = read_request(&mut socket);
                assert!(raw.starts_with("POST /audio/speech "));
                assert!(raw.contains("custom-tts"));
                assert!(raw.contains("custom-voice"));
                assert!(raw.contains("第一句话"));
                let body = if step < 2 {
                    expected.clone()
                } else {
                    bare.clone()
                };
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
        let clip0 = run_speak(
            &state,
            &settings,
            SpeakRequest {
                request_id: "turn-0".into(),
                turn_id: "turn".into(),
                expected_base_url: base.clone(),
                seq: 0,
                text: "第一句话".into(),
            },
            |verdict| {
                seen.lock().unwrap().push(verdict);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert!(clip0.len() < fixture.len()); // the first clip trims too now
        let later = run_speak(
            &state,
            &settings,
            SpeakRequest {
                request_id: "turn-1".into(),
                turn_id: "turn".into(),
                expected_base_url: base.clone(),
                seq: 1,
                text: "第一句话".into(),
            },
            |verdict| {
                seen.lock().unwrap().push(verdict);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(later.len(), fixture.len() - 74 * 640);
        assert!(later.len() <= clip0.len());
        // A tone-free clip comes back untouched and reports Absent — the
        // verdict that must never engage the frontend's single-shot fallback.
        let bare_clip = run_speak(
            &state,
            &settings,
            SpeakRequest {
                request_id: "turn-2".into(),
                turn_id: "turn".into(),
                expected_base_url: base,
                seq: 2,
                text: "第一句话".into(),
            },
            |verdict| {
                seen.lock().unwrap().push(verdict);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(bare_clip, plain);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                ToneVerdict::Trimmed,
                ToneVerdict::Trimmed,
                ToneVerdict::Absent
            ]
        );
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
        // A local listener proves the "without any request" claim: any stray
        // network attempt would surface as an accepted connection.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (path, settings) = store_with_voice(&base);
        let state = VoiceTurnState::default();
        let cases = [
            SpeakRequest {
                request_id: "t-0".into(),
                turn_id: "t".into(),
                expected_base_url: base.clone(),
                seq: 0,
                text: "x".repeat(501),
            },
            SpeakRequest {
                request_id: "t-1".into(),
                turn_id: "t".into(),
                expected_base_url: base.clone(),
                seq: 0,
                text: "   ".into(),
            },
            SpeakRequest {
                request_id: "t-2".into(),
                turn_id: "t".into(),
                expected_base_url: base.clone(),
                seq: 65,
                text: "句子".into(),
            },
            SpeakRequest {
                request_id: "t-3".into(),
                turn_id: "t".into(),
                expected_base_url: "https://changed.example".into(),
                seq: 0,
                text: "句子".into(),
            },
        ];
        for case in cases {
            assert!(run_speak(&state, &settings, case, |_| Ok(()))
                .await
                .is_err());
        }
        assert!(state.0.lock().unwrap().is_empty());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "validation failures must never reach the network"
        );
        drop(settings); // close SQLite before the Windows file delete
        std::fs::remove_file(path).unwrap();
    }

    /// Real-service diagnostic (run explicitly: `cargo test diagnose_real -- --ignored --nocapture`).
    /// Synthesizes one sentence through the user's configured voice service,
    /// saves the WAV, runs both trim passes and prints per-frame RMS so a
    /// survived tone can be diagnosed from the provider's actual shape.
    #[tokio::test]
    #[ignore = "uses the real configured voice service and credentials"]
    async fn diagnose_real_tts_tone_shape() {
        let data_dir = std::env::var("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .expect("LOCALAPPDATA")
            .join("dev.qiban.companion");
        let store =
            crate::model_settings::ModelStore::open(&data_dir.join("model-settings.db")).unwrap();
        let voice = store.voice_config();
        assert!(!voice.voice_base_url.is_empty(), "voice not configured");
        let key = if voice.use_voice_key {
            crate::credentials::read(&voice.voice_base_url)
                .unwrap()
                .expect("voice key missing")
                .to_string()
        } else {
            String::new()
        };
        let client = SpeechClient::new(voice.voice_base_url.clone(), key.clone()).unwrap();
        // Multiple clips like a real turn: later sentences are their own
        // synth requests — the mid-sentence beeps live there. The sustained
        // vowel openings (2026-10-30) probe the documented mis-trim risk:
        // steady lead-ins that are CONTENT, not the provider tone.
        let texts = [
            "我先说第一句。",
            "好的，明白。",
            "这是中间的一句。",
            "啊——今天的天气真好呀。",
            "嗯——让我想一想该怎么回答你。",
            "啊——嗯——哦——今天想听点什么呢？",
        ];
        for (index, text) in texts.iter().enumerate() {
            let (wav, _) = client
                .synthesize(&voice.tts_model, &voice.voice, text)
                .await
                .unwrap();
            let (trimmed, did) = trim_leading_tone(&wav);
            eprintln!(
                "[{text}] trim did={did:?}, {} -> {} bytes",
                wav.len(),
                trimmed.len()
            );
            std::fs::write(format!("D:/Game/.cache/diag-tts-{index}.wav"), &wav).unwrap();
            let format = wav_format(&wav).unwrap();
            let frame_bytes = format.byte_rate * FRAME_MS / 1000;
            let data = &wav[format.data_offset..format.data_offset + format.data_len];
            let frames = (data.len() / frame_bytes).min(WINDOW_FRAMES);
            let mut line = String::new();
            for frame in 0..frames {
                let chunk = &data[frame * frame_bytes..(frame + 1) * frame_bytes];
                let (sum, count) = chunk.chunks_exact(2).fold((0.0f64, 0.0f64), |(s, n), p| {
                    let sample = i16::from_le_bytes([p[0], p[1]]) as f64 / 32768.0;
                    (s + sample * sample, n + 1.0)
                });
                let rms = (sum / count).sqrt();
                let level = match rms {
                    v if v < SILENCE_RMS => '-',
                    v if v < 0.01 => '.',
                    v if v < 0.06 => 'x',
                    _ => 'X',
                };
                line.push(level);
            }
            eprintln!("  levels: {line}");
        }
        let long = "这是一段没有句号的长句子用来测试服务端会不会在音频中段插入提示音我们需要看得更远一些超过分析窗口之外的形态这样才知道中间的嘟嘟声到底来自哪里";
        let (wav, _) = client
            .synthesize(&voice.tts_model, &voice.voice, long)
            .await
            .unwrap();
        // A/B against the 2026-10-07 baseline (no watermark_enabled field):
        // if the tone returns here, its disappearance is driven by OUR request
        // param + the console switch; if not, the provider changed globally.
        {
            let raw = reqwest::Client::new();
            let mut request = raw
                .post(format!("{}/audio/speech", voice.voice_base_url))
                .json(&serde_json::json!({
                    "model": voice.tts_model,
                    "input": "我先说第一句。",
                    "voice": voice.voice,
                    "response_format": "wav",
                    "stream": false,
                }));
            if !key.is_empty() {
                request = request.bearer_auth(&key);
            }
            let bytes = request.send().await.unwrap().bytes().await.unwrap();
            std::fs::write("D:/Game/.cache/diag-tts-baseline.wav", &bytes).unwrap();
            let (trimmed, did) = trim_leading_tone(&bytes);
            eprintln!(
                "[baseline 无 watermark 字段] {}字节 trim did={did:?} -> {} 字节",
                bytes.len(),
                trimmed.len()
            );
            if let Ok(format) = wav_format(&bytes) {
                let frame_bytes = format.byte_rate * FRAME_MS / 1000;
                let data = &bytes[format.data_offset..format.data_offset + format.data_len];
                let frames = (data.len() / frame_bytes).min(WINDOW_FRAMES);
                let mut line = String::new();
                for frame in 0..frames {
                    let chunk = &data[frame * frame_bytes..(frame + 1) * frame_bytes];
                    let (sum, count) = chunk.chunks_exact(2).fold((0.0f64, 0.0f64), |(s, n), p| {
                        let sample = i16::from_le_bytes([p[0], p[1]]) as f64 / 32768.0;
                        (s + sample * sample, n + 1.0)
                    });
                    let rms = (sum / count).sqrt();
                    let level = match rms {
                        v if v < SILENCE_RMS => '-',
                        v if v < 0.01 => '.',
                        v if v < 0.06 => 'x',
                        _ => 'X',
                    };
                    line.push(level);
                }
                eprintln!("  baseline levels: {line}");
            }
        }
        std::fs::write("D:/Game/.cache/diag-tts-long.wav", &wav).unwrap();
        let (trimmed, did) = trim_leading_tone(&wav);
        eprintln!(
            "[long {long:.12}…] trim did={did:?}, {} -> {} bytes",
            wav.len(),
            trimmed.len()
        );
        // FULL-length level dump (not just the 2.6s window) so mid-clip
        // insertions become visible.
        let format = wav_format(&wav).unwrap();
        let frame_bytes = format.byte_rate * FRAME_MS / 1000;
        let data = &wav[format.data_offset..format.data_offset + format.data_len];
        let total_frames = data.len() / frame_bytes;
        let mut line = String::new();
        for frame in 0..total_frames {
            let chunk = &data[frame * frame_bytes..(frame + 1) * frame_bytes];
            let (sum, count) = chunk.chunks_exact(2).fold((0.0f64, 0.0f64), |(s, n), p| {
                let sample = i16::from_le_bytes([p[0], p[1]]) as f64 / 32768.0;
                (s + sample * sample, n + 1.0)
            });
            let rms = (sum / count).sqrt();
            let level = match rms {
                v if v < SILENCE_RMS => '-',
                v if v < 0.01 => '.',
                v if v < 0.06 => 'x',
                _ => 'X',
            };
            line.push(level);
        }
        eprintln!("  long levels ({total_frames} frames): {line}");
        // Per-frame RMS for the analysis window, quantized for readability.
        let format = wav_format(&wav).unwrap();
        let frame_bytes = format.byte_rate * FRAME_MS / 1000;
        let data = &wav[format.data_offset..format.data_offset + format.data_len];
        let frames = (data.len() / frame_bytes).min(WINDOW_FRAMES);
        let mut line = String::new();
        for frame in 0..frames {
            let chunk = &data[frame * frame_bytes..(frame + 1) * frame_bytes];
            let (sum, count) = chunk.chunks_exact(2).fold((0.0f64, 0.0f64), |(s, n), p| {
                let sample = i16::from_le_bytes([p[0], p[1]]) as f64 / 32768.0;
                (s + sample * sample, n + 1.0)
            });
            let rms = (sum / count).sqrt();
            let level = match rms {
                v if v < SILENCE_RMS => '-',
                v if v < 0.01 => '.',
                v if v < 0.05 => 'x',
                _ => 'X',
            };
            line.push(level);
        }
        eprintln!("frame levels (first {frames} frames, 20ms each): {line}");
    }
}

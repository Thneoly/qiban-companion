//! T04 bounded, non-streaming speech probe. Credentials never cross IPC.
use crate::model_settings::{ModelConfig, ModelState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, State, WebviewWindow};
use tokio::sync::watch;

#[derive(Default)]
pub struct VoiceState(Mutex<Option<(String, watch::Sender<bool>)>>);
struct Active<'a>(&'a VoiceState);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0 .0.lock() {
            *active = None;
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRequest {
    request_id: String,
    expected_base_url: String,
    voice_base_url: String,
    use_voice_key: bool,
    asr_model: String,
    tts_model: String,
    voice: String,
    wav: Vec<u8>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStage {
    request_id: String,
    stage: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceResult {
    request_id: String,
    transcript: String,
    reply: String,
    wav: Vec<u8>,
    recognition_ms: u128,
    generation_ms: u128,
    synthesis_ms: u128,
    input_seconds: f64,
    output_seconds: f64,
    model_total_tokens: Option<u64>,
    // Audio endpoints have no standardized usage response; never invent a price.
    audio_cost: Option<f64>,
}

fn identifier(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 160 && !value.chars().any(char::is_control)
}
/// Accept PCM16 RIFF/WAVE only, including ancillary chunks; bound decoded duration.
fn wav_seconds(bytes: &[u8], limit: f64) -> Result<f64, String> {
    let invalid = || "需要完整PCM16 WAV音频，时长或格式超出实验范围".to_string();
    if bytes.len() < 44
        || bytes.len() > 8 * 1024 * 1024
        || &bytes[..4] != b"RIFF"
        || &bytes[8..12] != b"WAVE"
    {
        return Err(invalid());
    }
    let u32_at = |p| u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()) as usize;
    // The RIFF size field is not trustworthy on server-generated WAVs (Zhipu's
    // glm-tts writes len-12, real capture 2026-10-07); chunk walking below is
    // the authority for length consistency, so no equality check here.
    let mut offset = 12;
    let mut format = None;
    let mut data_len = None;
    while offset + 8 <= bytes.len() {
        let size = u32_at(offset + 4);
        let start = offset + 8;
        let end = start.checked_add(size).ok_or_else(invalid)?;
        if end > bytes.len() {
            return Err(invalid());
        }
        match &bytes[offset..offset + 4] {
            b"fmt " => {
                if size < 16 || format.is_some() {
                    return Err(invalid());
                }
                let u16_at = |p| u16::from_le_bytes(bytes[p..p + 2].try_into().unwrap()) as usize;
                let channels = u16_at(start + 2);
                let rate = u32_at(start + 4);
                if u16_at(start) != 1
                    || !(1..=2).contains(&channels)
                    || !(8000..=48000).contains(&rate)
                    || u16_at(start + 14) != 16
                    || u16_at(start + 12) != channels * 2
                    || u32_at(start + 8) != rate * channels * 2
                {
                    return Err(invalid());
                }
                format = Some((rate * channels * 2, channels * 2));
            }
            b"data" if data_len.is_some() => return Err(invalid()),
            b"data" => data_len = Some(size),
            _ => {}
        }
        offset = end.checked_add(size % 2).ok_or_else(invalid)?;
    }
    if offset != bytes.len() {
        return Err(invalid());
    }
    let (rate, block) = format.ok_or_else(invalid)?;
    let size = data_len.ok_or_else(invalid)?;
    let seconds = size as f64 / rate as f64;
    if size % block != 0 || seconds <= 0.0 || seconds > limit {
        return Err(invalid());
    }
    Ok(seconds)
}
async fn bounded(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        return Err(format!(
            "语音服务请求失败（HTTP {}）；请核对模型、权限或额度",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "语音响应读取失败")? {
        if bytes.len() + chunk.len() > max {
            return Err("语音响应超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn run(
    config: &ModelConfig,
    key: &str,
    speech: &ModelConfig,
    speech_key: &str,
    request: VoiceRequest,
    stage: impl Fn(&'static str) -> Result<(), String>,
) -> Result<VoiceResult, String> {
    let input_seconds = wav_seconds(&request.wav, 30.0)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|_| "语音网络初始化失败")?;
    let authorized = |path: &str| {
        let builder = client.post(format!("{}{path}", speech.base_url));
        if speech_key.is_empty() {
            builder
        } else {
            builder.bearer_auth(speech_key)
        }
    };
    stage("recognizing")?;
    let started = Instant::now();
    let part = reqwest::multipart::Part::bytes(request.wav)
        .file_name("recording.wav")
        .mime_str("audio/wav")
        .map_err(|_| "音频编码失败")?;
    let form = reqwest::multipart::Form::new()
        .text("model", request.asr_model)
        .text("stream", "false")
        .part("file", part);
    let response = authorized("/audio/transcriptions")
        .multipart(form)
        .send()
        .await
        .map_err(|_| "语音识别连接失败")?;
    let value: Value = serde_json::from_slice(&bounded(response, 65536).await?)
        .map_err(|_| "语音识别响应格式不兼容")?;
    let transcript = value["text"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.chars().count() <= 1800)
        .ok_or("没有可用的识别文本，或内容过长")?
        .to_string();
    let recognition_ms = started.elapsed().as_millis();
    stage("generating")?;
    let started = Instant::now();
    let (model_usage, reply) = crate::chat::stream(
        &config.endpoint(),
        key,
        &config.model,
        config.max_output_tokens,
        &[],
        &format!("请用不超过60个中文字简短回应以下话语，不要调用工具：\n{transcript}"),
        |_| Ok(()),
    )
    .await?;
    let generation_ms = started.elapsed().as_millis();
    if reply.chars().count() > 500 {
        return Err("回复超过语音实验500字上限，未提交合成".into());
    }
    stage("synthesizing")?;
    let started = Instant::now();
    let response = authorized("/audio/speech").json(&json!({"model":request.tts_model,"input":reply,"voice":request.voice,"response_format":"wav","stream":false})).send().await.map_err(|_| "语音合成连接失败")?;
    let wav = bounded(response, 8 * 1024 * 1024).await?;
    // Real providers emit headers the strict RIFF checks may reject (sized-on-close
    // chunks, placeholder sizes); keep the rejected bytes once for diagnosis.
    let output_seconds = match wav_seconds(&wav, 60.0) {
        Ok(seconds) => seconds,
        Err(reason) => {
            let dump = std::env::temp_dir().join("qiban-voice-tts-rejected.wav");
            let _ = std::fs::write(&dump, &wav);
            let head: String = wav.iter().take(48).map(|b| format!("{b:02x}")).collect();
            return Err(format!(
                "合成音频校验失败（{reason}；{}字节，头48hex={head}）；诊断副本已写入 {}",
                wav.len(),
                dump.display()
            ));
        }
    };
    Ok(VoiceResult {
        request_id: request.request_id,
        transcript,
        reply,
        wav,
        recognition_ms,
        generation_ms,
        synthesis_ms: started.elapsed().as_millis(),
        input_seconds,
        output_seconds,
        model_total_tokens: model_usage.and_then(|u| u["total_tokens"].as_u64()),
        audio_cost: None,
    })
}
#[tauri::command]
pub fn voice_cancel(state: State<'_, VoiceState>, request_id: String) -> Result<(), String> {
    let active = state.0.lock().map_err(|_| "语音状态不可用")?;
    if let Some((id, signal)) = active.as_ref() {
        if id == &request_id {
            let _ = signal.send(true);
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn voice_key_set(window: WebviewWindow, base_url: String) -> Result<(), String> {
    let config = ModelConfig {
        base_url,
        ..Default::default()
    }
    .validated()?;
    #[cfg(windows)]
    {
        let hwnd = window.hwnd().map_err(|_| "语音设置窗口不可用")?.0 as usize;
        tauri::async_runtime::spawn_blocking(move || {
            crate::credentials::prompt(&config.base_url, hwnd)
        })
        .await
        .map_err(|_| "密钥窗口异常")?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, config);
        Err("当前仅在Windows支持系统凭据配置".into())
    }
}
#[tauri::command]
pub async fn voice_probe(
    state: State<'_, VoiceState>,
    settings: State<'_, ModelState>,
    request: VoiceRequest,
    on_stage: Channel<VoiceStage>,
) -> Result<VoiceResult, String> {
    if request.request_id.is_empty()
        || request.request_id.len() > 80
        || !request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !identifier(&request.asr_model)
        || !identifier(&request.tts_model)
        || !identifier(&request.voice)
    {
        return Err("请填写有效的语音模型与音色编码".into());
    }
    wav_seconds(&request.wav, 30.0)?;
    let config = settings
        .lock()
        .map_err(|_| "模型设置不可用")?
        .config
        .clone();
    if config.base_url != request.expected_base_url {
        return Err("模型服务地址已改变，请关闭后重新打开语音实验".into());
    }
    let speech = ModelConfig {
        base_url: request.voice_base_url.clone(),
        model: request.asr_model.clone(),
        use_api_key: request.use_voice_key,
        ..Default::default()
    }
    .validated()?;
    let speech_key = if speech.use_api_key {
        Some(
            crate::credentials::read(&speech.base_url)?
                .ok_or("请先为语音API地址设置密钥（不会自动复用其他地址的密钥）")?,
        )
    } else {
        None
    };
    let key = if config.use_api_key {
        Some(crate::credentials::read(&config.base_url)?.ok_or("请先配置当前API地址的密钥")?)
    } else {
        None
    };
    let (signal, mut cancelled) = watch::channel(false);
    let id = request.request_id.clone();
    {
        let mut active = state.0.lock().map_err(|_| "语音状态不可用")?;
        if active.is_some() {
            return Err("上一轮语音仍在结束".into());
        }
        *active = Some((id.clone(), signal));
    }
    let _active = Active(&state);
    tokio::select! {
        biased;
        _ = cancelled.changed() => Err("语音实验已停止；在途调用可能仍计费".into()),
        result = tokio::time::timeout(Duration::from_secs(120), run(&config, key.as_deref().map(|s|s.as_str()).unwrap_or(""), &speech, speech_key.as_deref().map(|s|s.as_str()).unwrap_or(""), request, |stage| on_stage.send(VoiceStage { request_id:id.clone(),stage }).map_err(|_|"语音窗口已断开".into()))) => result.map_err(|_|"语音链超过120秒时限".to_string())?,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wave() -> Vec<u8> {
        let mut b = Vec::from(&b"RIFF"[..]);
        b.extend(38_u32.to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16_u32.to_le_bytes());
        b.extend(1_u16.to_le_bytes());
        b.extend(1_u16.to_le_bytes());
        b.extend(16000_u32.to_le_bytes());
        b.extend(32000_u32.to_le_bytes());
        b.extend(2_u16.to_le_bytes());
        b.extend(16_u16.to_le_bytes());
        b.extend(b"data");
        b.extend(2_u32.to_le_bytes());
        b.extend([0, 0]);
        b
    }
    #[test]
    fn validates_wave_structure_before_any_upload() {
        let valid = wave();
        assert!(wav_seconds(&valid, 30.0).is_ok());
        let mut bad = valid.clone();
        bad[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(wav_seconds(&bad, 30.0).is_err());
        let mut bad = valid.clone();
        bad[28..32].copy_from_slice(&1_u32.to_le_bytes());
        assert!(wav_seconds(&bad, 30.0).is_err());
        assert!(wav_seconds(&valid[..44], 30.0).is_err());
        assert!(wav_seconds(&valid, 0.00001).is_err());
        assert!(!identifier("\nmodel"));
        // Zhipu glm-tts writes the RIFF size field len-12 (capture 2026-10-07);
        // chunk walking, not that field, decides length consistency.
        let mut provider = valid.clone();
        let short = (provider.len() as u32 - 12).to_le_bytes();
        provider[4..8].copy_from_slice(&short);
        assert!(wav_seconds(&provider, 30.0).is_ok());
        // Ancillary watermark chunks between fmt and data are walked through.
        let mut ancillary = Vec::with_capacity(valid.len() + 14);
        ancillary.extend_from_slice(b"RIFF");
        ancillary.extend_from_slice(&(valid.len() as u32 + 6).to_le_bytes());
        ancillary.extend_from_slice(&valid[8..36]);
        ancillary.extend_from_slice(b"AIGC");
        ancillary.extend_from_slice(&2_u32.to_le_bytes());
        ancillary.extend_from_slice(b"{}");
        ancillary.extend_from_slice(&valid[36..]);
        assert!(wav_seconds(&ancillary, 30.0).is_ok());
    }

    #[tokio::test]
    async fn runs_ordered_audio_chain_over_http_without_touching_chat_history() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for step in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
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
                        let length: usize = header
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|s| s.trim().parse().unwrap())
                            })
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let raw = String::from_utf8_lossy(&request);
                if step == 1 {
                    assert!(raw.contains("Bearer fixture-key"));
                    assert!(!raw.contains("Bearer speech-key"));
                } else {
                    assert!(raw.contains("Bearer speech-key"));
                    assert!(!raw.contains("Bearer fixture-key"));
                }
                let body = match step {
                    0 => {
                        assert!(raw.starts_with("POST /voice/audio/transcriptions "));
                        assert!(raw.contains("custom-asr"));
                        assert!(raw.contains("recording.wav"));
                        br#"{"text":"hello"}"#.to_vec()
                    }
                    1 => {
                        assert!(raw.starts_with("POST /chat/completions "));
                        assert!(raw.contains("custom-chat"));
                        b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"total_tokens\":4}}\n\ndata: [DONE]\n\n".to_vec()
                    }
                    _ => {
                        assert!(raw.starts_with("POST /voice/audio/speech "));
                        assert!(raw.contains("custom-tts"));
                        assert!(raw.contains("custom-voice"));
                        wave()
                    }
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
        let config = ModelConfig {
            base_url: base.clone(),
            model: "custom-chat".into(),
            ..Default::default()
        };
        let stages = Mutex::new(Vec::new());
        let speech = ModelConfig {
            base_url: format!("{base}/voice"),
            ..Default::default()
        };
        let result = run(
            &config,
            "fixture-key",
            &speech,
            "speech-key",
            VoiceRequest {
                request_id: "probe".into(),
                expected_base_url: base,
                voice_base_url: speech.base_url.clone(),
                use_voice_key: true,
                asr_model: "custom-asr".into(),
                tts_model: "custom-tts".into(),
                voice: "custom-voice".into(),
                wav: wave(),
            },
            |s| {
                stages.lock().unwrap().push(s);
                Ok(())
            },
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert_eq!(result.transcript, "hello");
        assert_eq!(result.reply, "hi");
        assert_eq!(result.model_total_tokens, Some(4));
        assert_eq!(result.audio_cost, None);
        assert_eq!(
            *stages.lock().unwrap(),
            vec!["recognizing", "generating", "synthesizing"]
        );
    }
}

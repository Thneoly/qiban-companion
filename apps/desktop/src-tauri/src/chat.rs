//! OpenAI-compatible Chat Completions single-turn transport. Credentials stay in this process; no tool execution.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, State};
use tokio::sync::watch;

#[derive(Default)]
pub struct ChatState(Mutex<Option<(String, watch::Sender<bool>)>>);
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConfig {
    configured: bool,
    model: String,
}
#[tauri::command]
pub fn chat_config(
    settings: State<'_, crate::model_settings::ModelState>,
) -> Result<ChatConfig, String> {
    let config = settings
        .lock()
        .map_err(|_| "模型设置不可用")?
        .config
        .clone();
    let configured = !config.use_api_key || crate::credentials::read(&config.base_url)?.is_some();
    Ok(ChatConfig {
        configured,
        model: config.model,
    })
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChatDelta {
    request_id: String,
    text: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResult {
    request_id: String,
    elapsed_ms: u128,
    usage: Option<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    request_id: String,
    prompt: String,
}

struct ActiveGuard<'a>(&'a ChatState);
impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0 .0.lock() {
            *state = None;
        }
    }
}
#[tauri::command]
pub fn chat_cancel(state: State<'_, ChatState>, request_id: String) -> Result<(), String> {
    let active = state.0.lock().map_err(|_| "对话状态不可用")?;
    if let Some((id, signal)) = active.as_ref() {
        if *id == request_id {
            let _ = signal.send(true);
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn chat_generate(
    state: State<'_, ChatState>,
    settings: State<'_, crate::model_settings::ModelState>,
    request: ChatRequest,
    on_delta: Channel<ChatDelta>,
) -> Result<ChatResult, String> {
    if request.request_id.len() > 80
        || request.request_id.is_empty()
        || !request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || request.prompt.trim().is_empty()
        || request.prompt.chars().count() > 2000
    {
        return Err("请输入1～2000字的内容".into());
    }
    let config = settings
        .lock()
        .map_err(|_| "模型设置不可用")?
        .config
        .clone();
    let key = if config.use_api_key {
        Some(crate::credentials::read(&config.base_url)?.ok_or("请先在模型设置中配置 API Key")?)
    } else {
        None
    };
    let selected = config.model.clone();
    let endpoint = config.endpoint();
    let (signal, mut cancelled) = watch::channel(false);
    {
        let mut active = state.0.lock().map_err(|_| "对话状态不可用")?;
        if active.is_some() {
            return Err("上一条回复仍在结束，请稍后再试".into());
        }
        *active = Some((request.request_id.clone(), signal));
    }
    let _guard = ActiveGuard(&state);
    let started = Instant::now();
    let run = stream(
        &endpoint,
        key.as_deref().map(|s| s.as_str()).unwrap_or(""),
        &selected,
        &request.prompt,
        |text| {
            on_delta
                .send(ChatDelta {
                    request_id: request.request_id.clone(),
                    text,
                })
                .map_err(|_| "对话窗口已断开".to_string())
        },
    );
    tokio::select! {
        biased;
        _ = cancelled.changed() => Err("已停止生成；已产生的服务用量仍可能计费".into()),
        result = tokio::time::timeout(Duration::from_secs(90),run) => {
            let usage=result.map_err(|_| "回复超时，请稍后重试")??;
            Ok(ChatResult { request_id:request.request_id, elapsed_ms:started.elapsed().as_millis(), usage })
        }
    }
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
    data: String,
}
impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, String> {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > 262144 {
            return Err("模型响应片段过大".into());
        }
        let mut events = Vec::new();
        while let Some(index) = self.buffer.iter().position(|b| *b == b'\n') {
            let bytes: Vec<u8> = self.buffer.drain(..=index).collect();
            let line = std::str::from_utf8(&bytes)
                .map_err(|_| "模型响应编码错误")?
                .trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    if self.data.trim() == "[DONE]" {
                        events.push(json!({"done":true}));
                    } else {
                        events.push(
                            serde_json::from_str(&self.data).map_err(|_| "模型响应格式错误")?,
                        );
                    }
                    self.data.clear();
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
                if self.data.len() > 262144 {
                    return Err("模型响应片段过大".into());
                }
            }
        }
        Ok(events)
    }
}
async fn stream(
    endpoint: &str,
    key: &str,
    selected: &str,
    prompt: &str,
    mut emit: impl FnMut(String) -> Result<(), String>,
) -> Result<Option<Value>, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法初始化模型连接")?;
    let request = client.post(endpoint);
    let request = if key.is_empty() {
        request
    } else {
        request.bearer_auth(key)
    };
    let mut response=request.json(&json!({
        "model":selected,"stream":true,"max_tokens":1024,

        "messages":[{"role":"system","content":"你是栖栖，一个温和、诚实的桌面AI伙伴。用简洁中文交流。这是单轮对话，你不能访问电脑、文件或执行任务，也没有长期记忆。不要声称已经做过未执行的事情。"},
        {"role":"user","content":prompt}]
    })).send().await.map_err(|_| "无法连接模型服务，请检查网络后重试")?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => "鉴权失败，请检查API Key 和模型权限",
            404 => "模型或接口不可用，请核对模型编码",
            429 => "请求受限或额度不足，请检查账号后重试",
            400 => "请求未被接受，请核对模型编码与参数支持",
            _ => "模型服务暂时不可用，请稍后重试",
        }
        .into());
    }
    let mut decoder = SseDecoder::default();
    let mut usage = None;
    let mut bytes = 0usize;
    let mut text_len = 0usize;
    let mut finished = false;
    while let Some(chunk) = response.chunk().await.map_err(|_| "回复连接中断，请重试")? {
        bytes += chunk.len();
        if bytes > 4 * 1024 * 1024 {
            return Err("响应达到本机大小上限，已停止".into());
        }
        for event in decoder.push(&chunk)? {
            if event.get("error").is_some() {
                return Err("模型返回错误，请检查账号或稍后重试".into());
            }
            if event.get("done") == Some(&Value::Bool(true)) {
                return if finished && text_len > 0 {
                    Ok(usage)
                } else {
                    Err("回复未正常完成，请重试".into())
                };
            }
            if let Some(value) = event.get("usage").filter(|v| v.is_object()) {
                // Only retain numeric counters, never an arbitrary provider payload.
                usage = Some(json!({
                    "prompt_tokens":value.get("prompt_tokens").and_then(Value::as_u64),
                    "completion_tokens":value.get("completion_tokens").and_then(Value::as_u64),
                    "total_tokens":value.get("total_tokens").and_then(Value::as_u64)
                }));
            }
            if let Some(text) = event
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
            {
                text_len += text.len();
                if text_len > 65536 {
                    return Err("回复达到显示上限，已停止".into());
                }
                if !text.is_empty() {
                    emit(text.into())?;
                }
            }
            if let Some(reason) = event
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
            {
                if reason != "stop" {
                    return Err(if reason == "length" {
                        "回复达到长度上限，可缩短问题后重试"
                    } else {
                        "回复未正常完成，请调整问题后重试"
                    }
                    .into());
                }
                finished = true;
            }
        }
    }
    if finished && text_len > 0 {
        Ok(usage)
    } else {
        Err("回复连接提前结束，请重试".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_handles_split_utf8_crlf_and_multiple_events() {
        let input=": ping\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\r\n\r\ndata: [DONE]\n\n".as_bytes();
        let mut decoder = SseDecoder::default();
        let mut out = Vec::new();
        for byte in input {
            out.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(out[0].pointer("/choices/0/delta/content").unwrap(), "你好");
        assert_eq!(out[1]["done"], true);
    }
    #[test]
    fn rejects_bad_and_oversized_sse() {
        assert!(SseDecoder::default().push(b"data: bad\n\n").is_err());
        assert!(SseDecoder::default().push(&vec![b'a'; 262145]).is_err());
    }

    fn fixture(status: &str, body: &str) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response=format!("HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = socket.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|p| p == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let size = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            socket.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}")
    }
    #[tokio::test]
    async fn streams_over_http_and_requires_completion() {
        let endpoint=fixture("200 OK","data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\ndata: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"total_tokens\":7}}\n\ndata: [DONE]\n\n");
        let mut text = String::new();
        let usage = stream(&endpoint, "test-only", "fixture", "hello", |part| {
            text.push_str(&part);
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(text, "你好");
        assert_eq!(usage.unwrap()["total_tokens"], 7);
        let endpoint = fixture(
            "200 OK",
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
        );
        assert!(
            stream(&endpoint, "test-only", "fixture", "hello", |_| Ok(()))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn provider_errors_never_forward_response_body_or_key() {
        let endpoint = fixture("401 Unauthorized", "fixture-private-provider-body");
        let error = stream(&endpoint, "fixture-private-key", "fixture", "hello", |_| {
            Ok(())
        })
        .await
        .unwrap_err();
        assert!(error.contains("鉴权失败"));
        assert!(!error.contains("fixture-private"));
    }
}

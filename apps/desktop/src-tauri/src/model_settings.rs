//! Non-secret settings in SQLite; API keys are scoped by base URL in Windows Credential Manager.
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State, WebviewWindow};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    pub base_url: String,
    pub model: String,
    pub use_api_key: bool,
    #[serde(default = "default_output_tokens")]
    pub max_output_tokens: u32,
}
fn default_output_tokens() -> u32 {
    1024
}
impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            model: "glm-5.3".into(),
            use_api_key: true,
            max_output_tokens: default_output_tokens(),
        }
    }
}
impl ModelConfig {
    pub(crate) fn validated(mut self) -> Result<Self, String> {
        if !(128..=8192).contains(&self.max_output_tokens) {
            return Err("最大输出 tokens 需要是128～8192之间的整数".into());
        }
        self.base_url = self.base_url.trim().trim_end_matches('/').into();
        self.model = self.model.trim().into();
        let url = reqwest::Url::parse(&self.base_url).map_err(|_| "API 地址无效")?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if self.base_url.len() > 512
            || (url.scheme() != "https" && !(url.scheme() == "http" && local))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "请使用 HTTPS API 基地址；本机服务可使用 HTTP。地址不能含密码、查询参数或片段"
                    .into(),
            );
        }
        if self.model.is_empty()
            || self.model.len() > 160
            || self.model.chars().any(char::is_control)
        {
            return Err("请填写有效的模型编码".into());
        }
        if self.base_url.ends_with("/chat/completions") {
            return Err("请填写 API 基地址，不包含 /chat/completions".into());
        }
        self.base_url = url.to_string().trim_end_matches('/').into();
        Ok(self)
    }
    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}
pub struct ModelStore {
    connection: Connection,
    pub config: ModelConfig,
    voice: VoiceConfig,
}
pub type ModelState = Mutex<ModelStore>;
/// Non-empty model/voice identifier: bounded length, no control characters.
pub(crate) fn identifier(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 160 && !value.chars().any(char::is_control)
}
/// Non-secret voice lab settings. The API key itself stays in the credential
/// manager scoped to the voice base URL; only `use_voice_key` is remembered.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceConfig {
    pub voice_base_url: String,
    pub use_voice_key: bool,
    pub asr_model: String,
    pub tts_model: String,
    pub voice: String,
}
impl VoiceConfig {
    pub(crate) fn validated(mut self) -> Result<Self, String> {
        let checked = ModelConfig {
            base_url: self.voice_base_url.trim().trim_end_matches('/').into(),
            ..Default::default()
        }
        .validated()?;
        self.voice_base_url = checked.base_url;
        self.asr_model = self.asr_model.trim().into();
        self.tts_model = self.tts_model.trim().into();
        self.voice = self.voice.trim().into();
        if !identifier(&self.asr_model) || !identifier(&self.tts_model) || !identifier(&self.voice)
        {
            return Err("请填写有效的语音模型与音色编码".into());
        }
        Ok(self)
    }
}
impl ModelStore {
    pub fn open(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_millis(250))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 2 {
            return Err("模型设置来自更新版本".into());
        }
        if version < 2 {
            // v2 adds voice_config; v0 databases still get both tables in one go.
            let migration = if version == 0 {
                "CREATE TABLE model_config(id INTEGER PRIMARY KEY CHECK(id=1),body TEXT NOT NULL); \
                 CREATE TABLE voice_config(id INTEGER PRIMARY KEY CHECK(id=1),body TEXT NOT NULL); \
                 PRAGMA user_version=2;"
            } else {
                "CREATE TABLE voice_config(id INTEGER PRIMARY KEY CHECK(id=1),body TEXT NOT NULL); \
                 PRAGMA user_version=2;"
            };
            let tx = connection.transaction()?;
            tx.execute_batch(migration)?;
            tx.commit()?;
        }
        let body: Option<String> = connection
            .query_row("SELECT body FROM model_config WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let config = body
            .map(|s| serde_json::from_str::<ModelConfig>(&s))
            .transpose()?
            .unwrap_or_default()
            .validated()?;
        let voice_body: Option<String> = connection
            .query_row("SELECT body FROM voice_config WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let voice = voice_body
            .map(|s| serde_json::from_str::<VoiceConfig>(&s))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            connection,
            config,
            voice,
        })
    }
    fn save(&mut self, config: ModelConfig) -> Result<(), String> {
        let config = config.validated()?;
        let body = serde_json::to_string(&config).map_err(|_| "模型设置编码失败")?;
        self.connection.execute("INSERT INTO model_config(id,body) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET body=excluded.body",[body]).map_err(|_|"保存模型设置失败")?;
        self.config = config;
        Ok(())
    }
    pub fn voice_config(&self) -> VoiceConfig {
        self.voice.clone()
    }
    pub fn save_voice_config(&mut self, config: VoiceConfig) -> Result<(), String> {
        let config = config.validated()?;
        let body = serde_json::to_string(&config).map_err(|_| "语音设置编码失败")?;
        self.connection.execute("INSERT INTO voice_config(id,body) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET body=excluded.body",[body]).map_err(|_|"保存语音设置失败")?;
        self.voice = config;
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    #[serde(flatten)]
    config: ModelConfig,
    has_api_key: bool,
}
#[tauri::command]
pub fn model_settings_get(state: State<'_, ModelState>) -> Result<ModelSettings, String> {
    let config = state.lock().map_err(|_| "模型设置不可用")?.config.clone();
    let has_api_key = crate::credentials::read(&config.base_url)?.is_some();
    Ok(ModelSettings {
        config,
        has_api_key,
    })
}
#[tauri::command]
pub fn model_settings_save(
    app: AppHandle,
    state: State<'_, ModelState>,
    chat: State<'_, crate::chat::ChatState>,
    config: ModelConfig,
) -> Result<(), String> {
    let mut store = state.lock().map_err(|_| "模型设置不可用")?;
    let config = config.validated()?;
    if store.config.base_url != config.base_url || store.config.model != config.model {
        // Invalidate first while holding the settings lock: a save failure is safe and keeps the old config.
        let mut inner = chat.0.lock().map_err(|_| "对话状态不可用")?;
        let epoch = inner
            .store
            .as_mut()
            .ok_or("记忆存储不可用，模型未切换")?
            .invalidate_context()
            .map_err(|_| "无法使旧预览失效，模型未切换")?;
        inner.invalidate_memory(false);
        let _ = app.emit(
            "memory-changed",
            crate::memory::MemoryChanged {
                context_epoch: epoch,
                chat_cleared: false,
            },
        );
    }
    store.save(config)?;
    crate::chat::select_config(&chat, &store.config.base_url, &store.config.model)
        .map_err(|error| format!("模型设置已保存，但对话记录未就绪：{error}"))
}
#[tauri::command]
pub async fn model_key_set(
    window: WebviewWindow,
    state: State<'_, ModelState>,
) -> Result<(), String> {
    let base_url = state
        .lock()
        .map_err(|_| "模型设置不可用")?
        .config
        .base_url
        .clone();
    #[cfg(windows)]
    {
        let hwnd = window.hwnd().map_err(|_| "设置窗口不可用")?.0 as usize;
        tauri::async_runtime::spawn_blocking(move || crate::credentials::prompt(&base_url, hwnd))
            .await
            .map_err(|_| "密钥窗口异常")?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, base_url);
        Err("当前仅在Windows支持系统凭据配置".into())
    }
}
#[tauri::command]
pub fn model_key_delete(state: State<'_, ModelState>) -> Result<(), String> {
    let base = state
        .lock()
        .map_err(|_| "模型设置不可用")?
        .config
        .base_url
        .clone();
    crate::credentials::delete(&base)
}
#[tauri::command]
pub fn voice_settings_get(state: State<'_, ModelState>) -> Result<VoiceConfig, String> {
    Ok(state.lock().map_err(|_| "模型设置不可用")?.voice_config())
}
#[tauri::command]
pub fn voice_settings_save(
    state: State<'_, ModelState>,
    config: VoiceConfig,
) -> Result<(), String> {
    state
        .lock()
        .map_err(|_| "模型设置不可用")?
        .save_voice_config(config)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_settings_round_trip_without_ever_storing_a_key() {
        let path = std::env::temp_dir().join(format!("voice-config-{}.db", uuid::Uuid::new_v4()));
        {
            let mut store = super::ModelStore::open(&path).unwrap();
            assert_eq!(store.voice_config().voice_base_url, "");
            store
                .save_voice_config(super::VoiceConfig {
                    voice_base_url: "https://open.bigmodel.cn/api/paas/v4/".into(),
                    use_voice_key: true,
                    asr_model: "glm-asr-2512".into(),
                    tts_model: "glm-tts".into(),
                    voice: "tongtong".into(),
                })
                .unwrap();
        }
        let store = super::ModelStore::open(&path).unwrap();
        let voice = store.voice_config();
        assert_eq!(voice.voice_base_url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(voice.asr_model, "glm-asr-2512");
        assert_eq!(voice.tts_model, "glm-tts");
        assert_eq!(voice.voice, "tongtong");
        assert!(voice.use_voice_key);
        let body: String = store
            .connection
            .query_row("SELECT body FROM voice_config", [], |r| r.get(0))
            .unwrap();
        // `useVoiceKey` is a boolean flag, not key material; no api-key-style field exists.
        assert!(!body.to_lowercase().contains("apikey"));
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn invalid_voice_settings_keep_previous_values() {
        let path = std::env::temp_dir().join(format!("voice-invalid-{}.db", uuid::Uuid::new_v4()));
        let mut store = super::ModelStore::open(&path).unwrap();
        let valid = super::VoiceConfig {
            voice_base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            use_voice_key: true,
            asr_model: "glm-asr-2512".into(),
            tts_model: "glm-tts".into(),
            voice: "tongtong".into(),
        };
        store.save_voice_config(valid.clone()).unwrap();
        for bad in [
            super::VoiceConfig {
                voice_base_url: "http://example.com/v1".into(),
                asr_model: " ".into(),
                ..valid.clone()
            },
            super::VoiceConfig {
                tts_model: "x\ny".into(),
                ..valid.clone()
            },
            super::VoiceConfig {
                voice: String::new(),
                ..valid.clone()
            },
            super::VoiceConfig {
                voice_base_url: "https://example.com/v1?key=x".into(),
                ..valid.clone()
            },
        ] {
            assert!(store.save_voice_config(bad).is_err());
            assert_eq!(store.voice_config().voice, valid.voice);
            assert_eq!(store.voice_config().voice_base_url, valid.voice_base_url);
        }
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn version_one_database_migrates_to_voice_table_without_touching_model_config() {
        let path = std::env::temp_dir().join(format!("voice-migrate-{}.db", uuid::Uuid::new_v4()));
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE model_config(id INTEGER PRIMARY KEY CHECK(id=1),body TEXT NOT NULL); \
                     INSERT INTO model_config VALUES(1,'{\"baseUrl\":\"https://example.com/v1\",\"model\":\"custom\",\"useApiKey\":false}'); \
                     PRAGMA user_version=1;",
                )
                .unwrap();
        }
        let store = super::ModelStore::open(&path).unwrap();
        assert_eq!(store.config.model, "custom");
        assert_eq!(store.voice_config().voice_base_url, "");
        let version: u32 = store
            .connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn validates_https_and_loopback_without_embedded_credentials() {
        for url in [
            "https://example.com/v1",
            "http://127.0.0.1:11434/v1",
            "http://[::1]:1234/v1",
        ] {
            assert!(ModelConfig {
                base_url: url.into(),
                ..Default::default()
            }
            .validated()
            .is_ok());
        }
        for url in [
            "http://example.com/v1",
            "https://key@example.com/v1",
            "https://example.com/v1?key=x",
            "https://example.com/v1/chat/completions",
        ] {
            assert!(ModelConfig {
                base_url: url.into(),
                ..Default::default()
            }
            .validated()
            .is_err());
        }
    }
    #[test]
    fn legacy_config_defaults_without_rewriting_and_invalid_budget_is_not_saved() {
        let path = std::env::temp_dir().join(format!("legacy-model-{}.db", uuid::Uuid::new_v4()));
        let store = ModelStore::open(&path).unwrap();
        let legacy = r#"{"baseUrl":"https://example.com/v1","model":"custom","useApiKey":false}"#;
        store
            .connection
            .execute("INSERT INTO model_config VALUES(1,?1)", [legacy])
            .unwrap();
        drop(store);
        let mut store = ModelStore::open(&path).unwrap();
        assert_eq!(store.config.max_output_tokens, 1024);
        assert_eq!(store.config.model, "custom");
        let before: String = store
            .connection
            .query_row("SELECT body FROM model_config", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, legacy);
        for budget in [0, 127, 8193, u32::MAX] {
            assert!(store
                .save(ModelConfig {
                    max_output_tokens: budget,
                    ..store.config.clone()
                })
                .is_err());
            assert_eq!(store.config.max_output_tokens, 1024);
        }
        let after: String = store
            .connection
            .query_row("SELECT body FROM model_config", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, legacy);
        for budget in [128, 8192] {
            assert!(ModelConfig {
                max_output_tokens: budget,
                ..Default::default()
            }
            .validated()
            .is_ok());
        }
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn restores_custom_provider_without_storing_a_key() {
        let path = std::env::temp_dir().join(format!("model-config-{}.db", uuid::Uuid::new_v4()));
        {
            let mut store = ModelStore::open(&path).unwrap();
            store
                .save(ModelConfig {
                    base_url: "https://example.com/v1/".into(),
                    model: "org/custom:model".into(),
                    use_api_key: false,
                    max_output_tokens: 4096,
                })
                .unwrap();
        }
        let store = ModelStore::open(&path).unwrap();
        assert_eq!(
            store.config.endpoint(),
            "https://example.com/v1/chat/completions"
        );
        assert_eq!(store.config.model, "org/custom:model");
        assert!(!store.config.use_api_key);
        assert_eq!(store.config.max_output_tokens, 4096);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}

//! Native account adapter. Loopback coordinator only for this local integration.
//! No account data is merged into desktop-local task/chat/memory databases.
use crate::account_vault::{SessionSecret, SessionVault, SystemVault};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use companion_core::Task;
use reqwest::{Client, Method};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};
use tauri::State;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

#[derive(Debug, Serialize)]
pub struct AccountError {
    pub code: &'static str,
    pub message: &'static str,
}
impl AccountError {
    fn new(code: &'static str) -> Self {
        let message = match code {
            "credentials" => "系统安全凭据存储不可用，未降级为明文保存。请检查当前系统支持与权限。",
            "logout_not_saved" => {
                "退出未完成：无法保存退出状态，请检查系统凭据存储后重试。当前会话仍可能有效。"
            }
            "storage" => "无法保存账号连接设置，请检查本机数据目录。",
            "authentication_required" => "登录已过期或被撤销，请重新登录。",
            "invalid_code" => "验证码无效或已过期，请检查后重试。",
            "rate_limited" => "请求较频繁，请至少间隔 60 秒再申请验证码。",
            "conflict" => "待办已在其他设备更新，请刷新列表。",
            "capacity" => "账号待办已达上限。",
            "invalid_request" => "请检查填写内容。待办最多 200 字，端口范围为 1024～65535。",
            "logout_pending" => "服务端撤销尚未完成，请恢复连接后重试。",
            "already_signed_in" => "请先退出当前账号，再修改连接或登录其他账号。",
            _ => "协调服务暂时不可用，操作结果尚未确认。请启动本机服务并重试。",
        };
        Self { code, message }
    }
}
type Result<T> = std::result::Result<T, AccountError>;
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    account_id: String,
    companion_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    port: u16,
    status: &'static str,
    profile: Option<Profile>,
    tasks: Vec<Task>,
    notice: Option<&'static str>,
}
struct Challenge {
    id: String,
    nonce: Zeroizing<String>,
}
pub struct AccountClient {
    db: std::sync::Mutex<Connection>,
    installation: String,
    port: u16,
    http: Client,
    vault: Arc<dyn SessionVault>,
    challenge: Option<Challenge>,
}
pub type AccountState = Mutex<AccountClient>;
impl AccountClient {
    pub fn open(path: &Path, vault: Arc<dyn SessionVault>) -> Result<Self> {
        let mut db = Connection::open(path).map_err(|_| AccountError::new("storage"))?;
        let load = |db: &mut Connection| -> std::result::Result<(String, u16), rusqlite::Error> {
            let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
            let app_id: u32 = db.pragma_query_value(None, "application_id", |r| r.get(0))?;
            const APP_ID: u32 = 0x51424144;
            if version == 0 {
                let tables: u32 = db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0))?;
                if app_id != 0 || tables != 0 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let tx = db.transaction()?;
                tx.execute_batch("CREATE TABLE account_settings(id INTEGER PRIMARY KEY CHECK(id=1),installation TEXT NOT NULL,port INTEGER NOT NULL); PRAGMA user_version=1; PRAGMA application_id=1363296580;")?;
                tx.execute(
                    "INSERT INTO account_settings VALUES(1,?1,4318)",
                    [uuid::Uuid::new_v4().to_string()],
                )?;
                tx.commit()?;
            } else if version != 1 || app_id != APP_ID {
                return Err(rusqlite::Error::InvalidQuery);
            }
            db.query_row(
                "SELECT installation,port FROM account_settings WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        };
        let (installation, port) = load(&mut db).map_err(|_| AccountError::new("storage"))?;
        if uuid::Uuid::parse_str(&installation).is_err() || port < 1024 {
            return Err(AccountError::new("storage"));
        }
        let http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(12))
            .build()
            .map_err(|_| AccountError::new("unavailable"))?;
        Ok(Self {
            db: std::sync::Mutex::new(db),
            installation,
            port,
            http,
            vault,
            challenge: None,
        })
    }
    fn scope(&self) -> String {
        format!("{}/127.0.0.1:{}", self.installation, self.port)
    }
    fn secret(&self) -> Result<Option<SessionSecret>> {
        let value = self
            .vault
            .read(&self.scope())
            .map_err(|_| AccountError::new("credentials"))?;
        if let Some(secret) = &value {
            if !valid_token(&secret.token) {
                return Err(AccountError::new("credentials"));
            }
        }
        Ok(value)
    }
    fn empty(&self, status: &'static str) -> Snapshot {
        Snapshot {
            port: self.port,
            status,
            profile: None,
            tasks: vec![],
            notice: None,
        }
    }
    fn require_signed_out(&self) -> Result<()> {
        if let Some(secret) = self.secret()? {
            return Err(AccountError::new(if secret.pending_logout.is_some() {
                "logout_pending"
            } else {
                "already_signed_in"
            }));
        }
        Ok(())
    }
    pub fn set_port(&mut self, port: u16) -> Result<()> {
        if port < 1024 {
            return Err(AccountError::new("invalid_request"));
        }
        self.require_signed_out()?;
        self.db
            .lock()
            .map_err(|_| AccountError::new("storage"))?
            .execute(
                "UPDATE account_settings SET port=?1 WHERE id=1",
                params![port],
            )
            .map_err(|_| AccountError::new("storage"))?;
        self.port = port;
        self.challenge = None;
        Ok(())
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("http://127.0.0.1:{}{}", self.port, path));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| AccountError::new("unavailable"))?;
        let status = response.status();
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AccountError::new("unavailable"))?
        {
            if bytes.len() + chunk.len() > 128 * 1024 {
                return Err(AccountError::new("unavailable"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| AccountError::new("unavailable"))?;
        if !status.is_success() {
            let code = match (status.as_u16(), value["error"]["code"].as_str()) {
                (401, Some("invalid_code")) if token.is_none() => "invalid_code",
                (401, _) => "authentication_required",
                (429, _) => "rate_limited",
                (409, Some("capacity")) => "capacity",
                (409, _) => "conflict",
                (400, _) => "invalid_request",
                _ => "unavailable",
            };
            if code == "authentication_required" && token.is_some() {
                // Even if local deletion fails, never return account data for this token.
                let _ = self.vault.delete(&self.scope());
            }
            return Err(AccountError::new(code));
        }
        Ok(value)
    }
    pub async fn request_code(&mut self, email: String) -> Result<()> {
        self.require_signed_out()?;
        if email.len() > 254 || email.trim().is_empty() {
            return Err(AccountError::new("invalid_request"));
        }
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| AccountError::new("unavailable"))?;
        let nonce = Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes));
        let receipt = self
            .request(
                Method::POST,
                "/v1/auth/request-code",
                None,
                Some(json!({"email": email.trim()})),
            )
            .await?;
        let id = receipt["challengeId"]
            .as_str()
            .filter(|s| uuid::Uuid::parse_str(s).is_ok())
            .ok_or(AccountError::new("unavailable"))?;
        self.challenge = Some(Challenge {
            id: id.into(),
            nonce,
        });
        Ok(())
    }
    pub async fn login(&mut self, code: String) -> Result<Snapshot> {
        self.require_signed_out()?;
        let code = Zeroizing::new(code);
        if code.len() != 8 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AccountError::new("invalid_request"));
        }
        let challenge = self
            .challenge
            .as_ref()
            .ok_or(AccountError::new("invalid_code"))?;
        let mut grant = self.request(Method::POST, "/v1/auth/verify-code", None,
            Some(json!({"challengeId":challenge.id,"code":code.as_str(),"nonce":challenge.nonce.as_str()}))).await?;
        let token = match grant["accessToken"].take() {
            Value::String(s) => s,
            _ => return Err(AccountError::new("unavailable")),
        };
        let secret = SessionSecret {
            token,
            pending_logout: None,
        };
        if !valid_token(&secret.token) {
            return Err(AccountError::new("unavailable"));
        }
        // If this write fails, retain challenge + nonce so a retry recovers the same grant.
        self.vault
            .write(&self.scope(), &secret)
            .map_err(|_| AccountError::new("credentials"))?;
        self.challenge = None;
        self.snapshot().await
    }
    async fn flush_logout(&self, secret: &SessionSecret) -> Result<bool> {
        let all = secret
            .pending_logout
            .ok_or(AccountError::new("logout_pending"))?;
        let confirmed = match self
            .request(
                Method::POST,
                "/v1/logout",
                Some(&secret.token),
                Some(json!({"allSessions":all})),
            )
            .await
        {
            Ok(_) => true,
            Err(e) if e.code == "authentication_required" => false,
            Err(e) => return Err(e),
        };
        self.vault
            .delete(&self.scope())
            .map_err(|_| AccountError::new("credentials"))?;
        Ok(confirmed)
    }
    pub async fn snapshot(&mut self) -> Result<Snapshot> {
        let Some(secret) = self.secret()? else {
            return Ok(self.empty("signed_out"));
        };
        if secret.pending_logout.is_some() {
            return Ok(match self.flush_logout(&secret).await {
                Ok(confirmed) => {
                    let mut snapshot = self.empty("signed_out");
                    if !confirmed && secret.pending_logout == Some(true) {
                        snapshot.notice = Some(
                            "本机会话已失效。其他设备退出结果未确认；请重新登录后再执行全部退出。",
                        );
                    }
                    snapshot
                }
                Err(_) => self.empty("logout_pending"),
            });
        }
        let profile: Profile = serde_json::from_value(
            self.request(Method::GET, "/v1/me", Some(&secret.token), None)
                .await?,
        )
        .map_err(|_| AccountError::new("unavailable"))?;
        if uuid::Uuid::parse_str(&profile.account_id).is_err()
            || uuid::Uuid::parse_str(&profile.companion_id).is_err()
        {
            return Err(AccountError::new("unavailable"));
        }
        let tasks: Vec<Task> = serde_json::from_value(
            self.request(Method::GET, "/v1/tasks", Some(&secret.token), None)
                .await?,
        )
        .map_err(|_| AccountError::new("unavailable"))?;
        Ok(Snapshot {
            port: self.port,
            status: "authenticated",
            profile: Some(profile),
            tasks,
            notice: None,
        })
    }
    fn active_secret(&self) -> Result<SessionSecret> {
        let secret = self
            .secret()?
            .ok_or(AccountError::new("authentication_required"))?;
        if secret.pending_logout.is_some() {
            return Err(AccountError::new("logout_pending"));
        }
        Ok(secret)
    }
    pub async fn create_task(&self, request_id: String, title: String) -> Result<Task> {
        let secret = self.active_secret()?;
        if uuid::Uuid::parse_str(&request_id).is_err()
            || title.trim().is_empty()
            || title.trim().chars().count() > 200
        {
            return Err(AccountError::new("invalid_request"));
        }
        serde_json::from_value(
            self.request(
                Method::POST,
                "/v1/tasks",
                Some(&secret.token),
                Some(json!({"requestId":request_id,"title":title.trim()})),
            )
            .await?,
        )
        .map_err(|_| AccountError::new("unavailable"))
    }
    pub async fn cancel_task(&self, id: String, revision: u32) -> Result<Task> {
        let secret = self.active_secret()?;
        if uuid::Uuid::parse_str(&id).is_err() {
            return Err(AccountError::new("invalid_request"));
        }
        serde_json::from_value(
            self.request(
                Method::POST,
                &format!("/v1/tasks/{id}/cancel"),
                Some(&secret.token),
                Some(json!({"revision":revision})),
            )
            .await?,
        )
        .map_err(|_| AccountError::new("unavailable"))
    }
    pub async fn logout(&mut self, all_sessions: bool) -> Result<Snapshot> {
        self.challenge = None;
        let Some(mut secret) = self
            .secret()
            .map_err(|_| AccountError::new("logout_not_saved"))?
        else {
            return Ok(self.empty("signed_out"));
        };
        secret.pending_logout = Some(secret.pending_logout.unwrap_or(false) || all_sessions);
        self.vault
            .write(&self.scope(), &secret)
            .map_err(|_| AccountError::new("logout_not_saved"))?;
        self.snapshot().await
    }
}
fn valid_token(token: &str) -> bool {
    token.len() == 47
        && token.starts_with("qbs_")
        && URL_SAFE_NO_PAD
            .decode(&token[4..])
            .is_ok_and(|v| v.len() == 32)
}
pub fn open_state(path: &Path) -> Result<AccountState> {
    Ok(Mutex::new(AccountClient::open(
        path,
        Arc::new(SystemVault),
    )?))
}

#[tauri::command]
pub async fn account_snapshot(state: State<'_, AccountState>) -> Result<Snapshot> {
    state.lock().await.snapshot().await
}
#[tauri::command]
pub async fn account_port_save(port: u16, state: State<'_, AccountState>) -> Result<()> {
    state.lock().await.set_port(port)
}
#[tauri::command]
pub async fn account_code_request(email: String, state: State<'_, AccountState>) -> Result<()> {
    state.lock().await.request_code(email).await
}
#[tauri::command]
pub async fn account_login(code: String, state: State<'_, AccountState>) -> Result<Snapshot> {
    state.lock().await.login(code).await
}
#[tauri::command]
pub async fn account_task_create(
    request_id: String,
    title: String,
    state: State<'_, AccountState>,
) -> Result<Task> {
    state.lock().await.create_task(request_id, title).await
}
#[tauri::command]
pub async fn account_task_cancel(
    id: String,
    revision: u32,
    state: State<'_, AccountState>,
) -> Result<Task> {
    state.lock().await.cancel_task(id, revision).await
}
#[tauri::command]
pub async fn account_logout(
    all_sessions: bool,
    state: State<'_, AccountState>,
) -> Result<Snapshot> {
    state.lock().await.logout(all_sessions).await
}

#[cfg(test)]
mod tests;

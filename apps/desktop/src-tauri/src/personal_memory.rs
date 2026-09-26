//! Read-only personal memory view: a loopback HTTP client for the standalone
//! memory service. Never writes, never starts the service, never injects
//! chat context; offline is a reported state, never faked data.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

const SERVICE_URL: &str = "http://127.0.0.1:4322";
// Covers the largest legal page (50 records x worst-case escaped content,
// the same arithmetic the service uses for its request-body limit).
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 16 * 1024;

/// Structured failure with stable codes so the frontend can branch on
/// `service_offline` without string matching; service codes pass through
/// unchanged, plus two client-side codes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMemoryFailure {
    pub code: &'static str,
    pub message: &'static str,
}

impl PersonalMemoryFailure {
    fn offline() -> Self {
        Self {
            code: "service_offline",
            message: "个人记忆服务未运行或无法连接，请先启动服务后再刷新",
        }
    }
    /// Connect-phase failures (including a dead port that drops instead of
    /// refusing) mean the service is not reachable: offline guidance. A read
    /// timeout on an established connection means slow-but-running.
    fn transport(error: reqwest::Error) -> Self {
        if !error.is_connect() && error.is_timeout() {
            Self {
                code: "timeout",
                message: "个人记忆服务响应超时，请稍后重试",
            }
        } else {
            Self::offline()
        }
    }
    fn invalid_request(message: &'static str) -> Self {
        Self {
            code: "invalid_request",
            message,
        }
    }
    fn from_service(code: &str) -> Self {
        Self {
            code: service_code(code),
            message: match code {
                "not_found" => "未找到该记忆，可能已被删除，请刷新后重试",
                "busy" => "个人记忆服务正忙，请稍后重试",
                "schema_mismatch" => "个人记忆服务数据库结构不兼容，请升级服务后再使用",
                "capacity" => "个人记忆服务容量已达上限，请联系维护",
                "storage_unavailable" => "个人记忆服务暂时不可用，请稍后重试",
                "invalid_request" => "检索条件不被服务接受，请调整后重试",
                _ => "个人记忆服务响应协议不兼容，请更新客户端或服务",
            },
        }
    }
    fn incompatible() -> Self {
        Self {
            code: "incompatible",
            message: "个人记忆服务响应协议不兼容，请更新客户端或服务",
        }
    }
}

/// Keeps unknown service codes machine-distinguishable instead of silently
/// remapping them onto an existing meaning.
fn service_code(code: &str) -> &'static str {
    match code {
        "not_found" => "not_found",
        "busy" => "busy",
        "schema_mismatch" => "schema_mismatch",
        "capacity" => "capacity",
        "storage_unavailable" => "storage_unavailable",
        "invalid_request" => "invalid_request",
        _ => "incompatible",
    }
}

/// The seven personal memory categories; `type` is a Rust keyword so the
/// field is `kind`, renamed on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersonalMemoryKind {
    #[serde(rename = "fact")]
    Fact,
    #[serde(rename = "decision")]
    Decision,
    #[serde(rename = "preference")]
    Preference,
    #[serde(rename = "project")]
    Project,
    #[serde(rename = "person")]
    Person,
    #[serde(rename = "insight")]
    Insight,
    #[serde(rename = "context")]
    Context,
}

impl PersonalMemoryKind {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "fact" => Self::Fact,
            "decision" => Self::Decision,
            "preference" => Self::Preference,
            "project" => Self::Project,
            "person" => Self::Person,
            "insight" => Self::Insight,
            "context" => Self::Context,
            _ => return None,
        })
    }
}

/// Client DTOs intentionally tolerate unknown fields: the desktop is a
/// client of the service and must not break on additive changes. The single
/// strict validation boundary lives in the TypeScript contracts.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMemoryRecord {
    pub id: i64,
    pub seq: i64,
    #[serde(rename = "type")]
    pub kind: PersonalMemoryKind,
    pub project: Option<String>,
    pub title: String,
    pub content: String,
    pub importance: i64,
    pub created_at: String,
    pub updated_at: String,
    pub valid_until: Option<String>,
    pub superseded_by: Option<i64>,
    pub contradicts: Option<i64>,
    pub tags: Vec<String>,
    pub origin: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMemoryList {
    pub count: i64,
    pub memories: Vec<PersonalMemoryRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMemoryDetail {
    pub memory: PersonalMemoryRecord,
    pub chain: Vec<PersonalMemoryRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMemoryStats {
    pub total: i64,
    pub active: i64,
    pub superseded: i64,
    pub by_type: BTreeMap<String, i64>,
    pub by_project: BTreeMap<String, i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMemoryOverview {
    pub online: bool,
    pub stats: Option<PersonalMemoryStats>,
    pub service_url: String,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: EnvelopeCode,
}

#[derive(Deserialize)]
struct EnvelopeCode {
    code: String,
}

#[derive(Deserialize)]
struct HealthDto {
    #[allow(dead_code)]
    status: String,
}

pub struct PersonalMemoryClient {
    base_url: String,
    response_cap: usize,
}

impl PersonalMemoryClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            response_cap: MAX_RESPONSE_BYTES,
        }
    }

    pub fn service() -> Self {
        Self::new(SERVICE_URL)
    }

    /// GET + bounded read + error-envelope mapping + strict decode. Query
    /// parameters go through `RequestBuilder::query` so search terms with
    /// `&`, `#` or non-ASCII survive percent-encoding.
    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, String)],
        timeout: Duration,
    ) -> Result<T, PersonalMemoryFailure> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            // This client only ever talks to a loopback service: a system or
            // environment proxy must never intercept it (routing loopback
            // traffic through a proxy both breaks offline detection and
            // leaks memory contents and search terms).
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(timeout)
            .build()
            .map_err(|_| PersonalMemoryFailure::incompatible())?;
        let response = client
            .get(format!("{}{path}", self.base_url))
            .query(params)
            .send()
            .await
            .map_err(PersonalMemoryFailure::transport)?;
        let status = response.status();
        let cap = if status.is_success() {
            self.response_cap
        } else {
            MAX_ERROR_BYTES
        };
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(PersonalMemoryFailure::transport)?
        {
            if bytes.len() + chunk.len() > cap {
                return Err(PersonalMemoryFailure::incompatible());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let envelope: ErrorEnvelope = serde_json::from_slice(&bytes)
                .map_err(|_| PersonalMemoryFailure::incompatible())?;
            return Err(PersonalMemoryFailure::from_service(&envelope.error.code));
        }
        serde_json::from_slice(&bytes).map_err(|_| PersonalMemoryFailure::incompatible())
    }

    /// Any failure means offline; this is a state probe, not an error path.
    pub async fn healthz(&self) -> bool {
        self.get_json::<HealthDto>("/healthz", &[], Duration::from_secs(3))
            .await
            .is_ok()
    }

    pub async fn stats(&self) -> Result<PersonalMemoryStats, PersonalMemoryFailure> {
        self.get_json::<PersonalMemoryStats>("/v1/stats", &[], Duration::from_secs(5))
            .await
    }

    pub async fn recall(
        &self,
        query: Option<&str>,
        project: Option<&str>,
        kind: Option<&str>,
        limit: Option<i64>,
    ) -> Result<PersonalMemoryList, PersonalMemoryFailure> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(query) = normalized(query, 200, "检索关键词最长 200 个字符")? {
            params.push(("query", query));
        }
        if let Some(project) = normalized(project, 64, "项目名最长 64 个字符")? {
            params.push(("project", project));
        }
        if let Some(kind) = kind {
            let kind = kind.trim();
            if !kind.is_empty() {
                if PersonalMemoryKind::parse(kind).is_none() {
                    return Err(PersonalMemoryFailure::invalid_request(
                        "记忆类型不在支持的七类之内",
                    ));
                }
                params.push(("type", kind.to_string()));
            }
        }
        if let Some(limit) = limit {
            params.push(("limit", limit.clamp(1, 50).to_string()));
        }
        self.get_json::<PersonalMemoryList>("/v1/memories", &params, Duration::from_secs(5))
            .await
    }

    pub async fn detail(&self, id: i64) -> Result<PersonalMemoryDetail, PersonalMemoryFailure> {
        if id < 1 {
            return Err(PersonalMemoryFailure::invalid_request(
                "记忆编号必须为正整数",
            ));
        }
        self.get_json::<PersonalMemoryDetail>(
            &format!("/v1/memories/{id}"),
            &[],
            Duration::from_secs(5),
        )
        .await
    }

    pub async fn overview(&self) -> PersonalMemoryOverview {
        let online = self.healthz().await;
        let stats = if online {
            self.stats().await.ok()
        } else {
            None
        };
        PersonalMemoryOverview {
            online,
            stats,
            service_url: self.base_url.clone(),
        }
    }
}

/// Trims, drops empty values, and enforces the character cap before any
/// network traffic happens.
fn normalized(
    value: Option<&str>,
    max_chars: usize,
    message: &'static str,
) -> Result<Option<String>, PersonalMemoryFailure> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > max_chars || trimmed.contains('\0') {
        return Err(PersonalMemoryFailure::invalid_request(message));
    }
    Ok(Some(trimmed.to_string()))
}

/// Offline is a state, not an error: the panel renders a dedicated block
/// with the service address instead of a red alert.
#[tauri::command]
pub async fn personal_memory_overview() -> PersonalMemoryOverview {
    PersonalMemoryClient::service().overview().await
}

#[tauri::command]
pub async fn personal_memory_recall(
    query: Option<String>,
    project: Option<String>,
    kind: Option<String>,
    limit: Option<i64>,
) -> Result<PersonalMemoryList, PersonalMemoryFailure> {
    PersonalMemoryClient::service()
        .recall(query.as_deref(), project.as_deref(), kind.as_deref(), limit)
        .await
}

#[tauri::command]
pub async fn personal_memory_detail(
    id: i64,
) -> Result<PersonalMemoryDetail, PersonalMemoryFailure> {
    PersonalMemoryClient::service().detail(id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory_service::{app_state, router, MemoryStore};
    use std::sync::Arc;

    struct TempDb(std::path::PathBuf);

    impl Drop for TempDb {
        fn drop(&mut self) {
            // The aborted server task drops its connection asynchronously,
            // so on Windows the delete can hit a sharing violation; retry
            // briefly instead of leaking the file (and its WAL siblings).
            for _ in 0..40 {
                let mut pending = false;
                for suffix in ["", "-wal", "-shm", ".v1.bak"] {
                    let path = if suffix.is_empty() {
                        self.0.clone()
                    } else {
                        // with_extension would mangle the stem; append raw.
                        let mut sibling = self.0.as_os_str().to_owned();
                        sibling.push(suffix);
                        std::path::PathBuf::from(sibling)
                    };
                    if path.exists() && std::fs::remove_file(&path).is_err() {
                        pending = true;
                    }
                }
                if !pending {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }

    struct TestService {
        base_url: String,
        task: tokio::task::JoinHandle<()>,
        _database: TempDb,
    }

    impl Drop for TestService {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn draft(
        kind: memory_service::MemoryKind,
        title: &str,
        content: &str,
    ) -> memory_service::store::NewMemory {
        memory_service::store::NewMemory {
            kind,
            title: title.to_string(),
            content: content.to_string(),
            project: None,
            importance: 3,
            tags: Vec::new(),
        }
    }

    /// Serves the real memory-service router on an ephemeral loopback port.
    async fn spawn_service() -> TestService {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "personal-memory-desktop-{}.db",
            uuid::Uuid::new_v4()
        ));
        let store = MemoryStore::open(&path, "http").expect("open archive");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let address = listener.local_addr().expect("local address");
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router(app_state(Arc::new(store)))).await;
        });
        TestService {
            base_url: format!("http://{address}"),
            task,
            _database: TempDb(path),
        }
    }

    /// A port that nothing listens on: the canonical offline condition.
    async fn dead_port_base() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        drop(listener);
        format!("http://{address}")
    }

    #[tokio::test]
    async fn overview_reports_online_stats_and_offline() {
        let service = spawn_service().await;
        let client = PersonalMemoryClient::new(&service.base_url);
        let overview = client.overview().await;
        assert!(overview.online);
        let stats = overview.stats.expect("stats online");
        assert_eq!(stats.total, 0);

        let dead = dead_port_base().await;
        let offline_client = PersonalMemoryClient::new(&dead);
        let overview = offline_client.overview().await;
        assert!(!overview.online);
        assert!(overview.stats.is_none());
        let failure = offline_client
            .recall(None, None, None, None)
            .await
            .unwrap_err();
        assert_eq!(failure.code, "service_offline");
    }

    #[tokio::test]
    async fn recall_encodes_query_and_applies_filters() {
        let service = spawn_service().await;
        let store = MemoryStore::open(&service._database.0, "http").expect("reopen");
        let mut scoped = draft(
            memory_service::MemoryKind::Fact,
            "带符号 & 和 # 的标题",
            "内容包含 JaAMAS & R2R #特殊",
        );
        scoped.project = Some("R2R".to_string());
        let tricky = store.remember(&scoped).expect("remember");
        store
            .remember(&draft(
                memory_service::MemoryKind::Preference,
                "全局偏好",
                "偏好内容",
            ))
            .expect("remember");
        drop(store);

        let client = PersonalMemoryClient::new(&service.base_url);
        // The ampersand and hash must survive percent-encoding.
        let hits = client
            .recall(Some("JaAMAS & R2R #特殊"), None, None, None)
            .await
            .expect("recall");
        assert_eq!(hits.count, 1);
        assert_eq!(hits.memories[0].id, tricky);

        // A project filter includes global rows, matching the service.
        let scoped = client
            .recall(None, Some("不存在的项目"), None, None)
            .await
            .expect("recall");
        assert_eq!(scoped.count, 1, "only the global row is visible");

        let by_kind = client
            .recall(None, None, Some("preference"), None)
            .await
            .expect("recall");
        assert_eq!(by_kind.count, 1);
        assert_eq!(by_kind.memories[0].kind, PersonalMemoryKind::Preference);

        // limit 0 is clamped to 1 before leaving the client.
        let limited = client
            .recall(None, None, None, Some(0))
            .await
            .expect("recall");
        assert_eq!(limited.count, 1);
    }

    #[tokio::test]
    async fn detail_returns_full_chain_and_not_found() {
        let service = spawn_service().await;
        let store = MemoryStore::open(&service._database.0, "http").expect("reopen");
        let first = store
            .remember(&draft(
                memory_service::MemoryKind::Decision,
                "旧决定",
                "旧内容",
            ))
            .expect("remember");
        let (_, second) = store
            .supersede(first, "新决定", "新内容")
            .expect("supersede");
        drop(store);

        let client = PersonalMemoryClient::new(&service.base_url);
        let detail = client.detail(first).await.expect("detail");
        assert_eq!(detail.memory.superseded_by, Some(second));
        assert_eq!(detail.chain.len(), 2);
        assert_eq!(detail.chain[0].id, first, "oldest first");
        assert_eq!(detail.chain[1].id, second);

        let failure = client.detail(10_000_000).await.unwrap_err();
        assert_eq!(failure.code, "not_found");
    }

    #[tokio::test]
    async fn invalid_filters_are_rejected_without_network() {
        let dead = dead_port_base().await;
        let client = PersonalMemoryClient::new(&dead);
        let bogus_kind = client
            .recall(None, None, Some("bogus"), None)
            .await
            .unwrap_err();
        assert_eq!(bogus_kind.code, "invalid_request");
        let long_query = client
            .recall(Some(&"词".repeat(201)), None, None, None)
            .await
            .unwrap_err();
        assert_eq!(long_query.code, "invalid_request");
        let bad_id = client.detail(0).await.unwrap_err();
        assert_eq!(bad_id.code, "invalid_request");
    }

    #[tokio::test]
    async fn oversized_response_is_rejected_instead_of_parsed() {
        let service = spawn_service().await;
        let store = MemoryStore::open(&service._database.0, "http").expect("reopen");
        for index in 0..3 {
            store
                .remember(&draft(
                    memory_service::MemoryKind::Fact,
                    &format!("大条目{index}"),
                    &"长".repeat(20_000),
                ))
                .expect("remember");
        }
        drop(store);
        let mut client = PersonalMemoryClient::new(&service.base_url);
        client.response_cap = 1024;
        let failure = client.recall(None, None, None, None).await.unwrap_err();
        assert_eq!(failure.code, "incompatible");
    }
}

/// Real-service walkthrough through the production constant: start the
/// deployed service first (`npm run memory:serve` or the installed exe),
/// then `cargo test -p companion-desktop real_service -- --ignored`.
#[tokio::test]
#[ignore = "requires the personal memory service running on 127.0.0.1:4322"]
async fn real_service_walkthrough_via_production_address() {
    let client = PersonalMemoryClient::service();
    let overview = client.overview().await;
    assert!(
        overview.online,
        "service must be running for this walkthrough"
    );
    let stats = overview.stats.expect("stats online");
    assert!(stats.active >= 1);
    let results = client.recall(None, None, None, None).await.expect("recall");
    assert_eq!(
        results.count as i64,
        stats.active.min(20),
        "default page matches active count up to the limit"
    );
    let first = results.memories[0].clone();
    let detail = client.detail(first.id).await.expect("detail");
    assert!(detail.chain.iter().any(|item| item.id == first.id));
}

//! Bounded document action contracts. No filesystem, Tauri or model calls.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_SOURCE_BYTES: usize = 256 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("请选择有效的 UTF-8 txt/md 文件（最多 256 KB），不能包含空内容或控制字符")]
    InvalidDocument,
    #[error("执行请求标识无效")]
    InvalidRequest,
    #[error("同一请求标识已用于另一份文档，请重新提交")]
    RequestConflict,
    #[error("任务状态已变化，请刷新后重新确认")]
    Conflict,
    #[error("当前状态不允许此操作；已开始的保存需要先核对结果")]
    InvalidTransition,
    #[error("本机执行记录已达 100 条，本轮样机暂不接受新任务")]
    Capacity,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    WaitingConfirmation,
    Running,
    Completed,
    Cancelled,
    Failed,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionTask {
    pub id: String,
    pub action_id: String,
    pub source_name: String,
    pub preview: String,
    pub artifact_name: String,
    pub artifact_hash: String,
    pub status: ExecutionStatus,
    pub revision: u32,
    pub created_at: u64,
    pub updated_at: u64,
    pub note: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionAttempt {
    pub id: String,
    pub action_id: String,
    pub owner: String,
    pub lease_until: u64,
    pub status: ExecutionStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionEvent {
    pub sequence: u64,
    pub task_id: String,
    pub revision: u32,
    pub status: ExecutionStatus,
    pub created_at: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionDetail {
    pub task: ExecutionTask,
    pub attempts: Vec<ExecutionAttempt>,
    pub events: Vec<ExecutionEvent>,
}
pub struct PreparedDocument {
    pub fingerprint: String,
    pub name: String,
    pub preview: String,
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn prepare_document(
    request_id: &str,
    name: &str,
    text: &str,
) -> Result<PreparedDocument, ExecutionError> {
    if uuid::Uuid::parse_str(request_id).is_err() {
        return Err(ExecutionError::InvalidRequest);
    }
    let lower = name.to_ascii_lowercase();
    if name.is_empty()
        || name.chars().count() > 150
        || name.chars().any(|c| c.is_control() || "/\\:".contains(c))
        || !(lower.ends_with(".txt") || lower.ends_with(".md"))
        || text.len() > MAX_SOURCE_BYTES
        || text.trim().is_empty()
        || text
            .chars()
            .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
    {
        return Err(ExecutionError::InvalidDocument);
    }
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(8)
        .map(|line| format!("> {}", line.chars().take(240).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n\n");
    let preview = format!("# 文档摘录草稿\n\n来源：{name}\n\n规则：取前 8 个非空行，每行最多 240 字；这是原文摘录，不是 AI 摘要。\n\n{lines}\n");
    Ok(PreparedDocument {
        fingerprint: digest(format!("excerpt-v1\0{name}\0{text}").as_bytes()),
        name: name.into(),
        preview,
    })
}
impl ExecutionTask {
    pub fn new(document: PreparedDocument) -> Self {
        let now = crate::now_ms();
        let action_id = uuid::Uuid::new_v4().to_string();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            artifact_name: format!("{action_id}.md"),
            artifact_hash: digest(document.preview.as_bytes()),
            action_id,
            source_name: document.name,
            preview: document.preview,
            status: ExecutionStatus::WaitingConfirmation,
            revision: 0,
            created_at: now,
            updated_at: now,
            note: "预览已保存，尚未创建草稿文件".into(),
        }
    }
    pub fn transition(
        &mut self,
        expected: u32,
        next: ExecutionStatus,
        note: &str,
    ) -> Result<(), ExecutionError> {
        if self.revision != expected {
            return Err(ExecutionError::Conflict);
        }
        use ExecutionStatus::*;
        if !matches!(
            (self.status, next),
            (WaitingConfirmation, Running | Cancelled)
                | (Running, Completed | Failed | Unknown)
                | (Unknown, Completed | Failed)
        ) {
            return Err(ExecutionError::InvalidTransition);
        }
        self.status = next;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(ExecutionError::Conflict)?;
        self.updated_at = crate::now_ms().max(self.updated_at);
        self.note = note.into();
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excerpt_is_bounded_unicode_and_preserves_explicit_scope() {
        let id = uuid::Uuid::new_v4().to_string();
        let source = (0..20)
            .map(|_| "字".repeat(400))
            .collect::<Vec<_>>()
            .join("\n");
        let p = prepare_document(&id, "日记.md", &source).unwrap();
        assert_eq!(p.preview.matches("> ").count(), 8);
        assert!(p.preview.contains("不是 AI 摘要"));
        assert!(prepare_document(&id, "../secret.txt", "abc").is_err());
        assert!(prepare_document(&id, "secret.pdf", "abc").is_err());
        assert!(prepare_document(&id, "a.txt", &"x".repeat(MAX_SOURCE_BYTES + 1)).is_err());
    }
    #[test]
    fn cancel_cannot_assert_running_action_has_stopped() {
        let p = prepare_document(&uuid::Uuid::new_v4().to_string(), "a.txt", "内容").unwrap();
        let mut t = ExecutionTask::new(p);
        t.transition(0, ExecutionStatus::Running, "开始").unwrap();
        assert!(t.transition(0, ExecutionStatus::Cancelled, "").is_err());
        assert!(t.transition(1, ExecutionStatus::Cancelled, "").is_err());
        t.transition(1, ExecutionStatus::Unknown, "待核对").unwrap();
        t.transition(2, ExecutionStatus::Completed, "已核对")
            .unwrap();
    }
}

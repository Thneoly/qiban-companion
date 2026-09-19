//! Pure domain contracts. No Tauri, network, filesystem or model dependency.
pub mod conversation;
pub mod memory;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("任务标题需要包含 1～200 个字符")]
    InvalidTitle,
    #[error("此任务状态不允许直接取消")]
    InvalidCancellation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    WaitingAuthorization,
    Running,
    Verifying,
    Completed,
    CancelRequested,
    Cancelled,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub revision: u32,
}

impl Task {
    pub fn new(title: &str) -> Result<Self, DomainError> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err(DomainError::InvalidTitle);
        }
        let now = now_ms();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.into(),
            status: TaskStatus::Queued,
            created_at: now,
            updated_at: now,
            revision: 0,
        })
    }

    // This skeleton only has an inbox: queued cancellation has no side effects.
    // Future running tasks must request cancellation and reconcile, never roll back by assertion.
    pub fn cancel_queued(&mut self) -> Result<(), DomainError> {
        match self.status {
            TaskStatus::Cancelled => Ok(()),
            TaskStatus::Queued => {
                self.status = TaskStatus::Cancelled;
                self.revision += 1;
                self.updated_at = now_ms().max(self.updated_at);
                Ok(())
            }
            _ => Err(DomainError::InvalidCancellation),
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blank_and_overlong_titles() {
        assert!(Task::new(" \n ").is_err());
        assert!(Task::new(&"字".repeat(201)).is_err());
        assert!(Task::new(&"🌿".repeat(200)).is_ok());
    }

    #[test]
    fn cancellation_is_idempotent_but_cannot_claim_running_work_stopped() {
        let mut task = Task::new("准备草稿").unwrap();
        task.cancel_queued().unwrap();
        task.cancel_queued().unwrap();
        assert_eq!(task.revision, 1);
        task.status = TaskStatus::Running;
        assert!(task.cancel_queued().is_err());
    }

    #[test]
    fn wire_shape_is_versioned_and_camel_case() {
        let task = Task::new("接口样本").unwrap();
        let value = serde_json::to_value(task).unwrap();
        assert_eq!(value["status"], "queued");
        assert!(value["createdAt"].is_u64());
        assert!(value.get("created_at").is_none());
    }
}

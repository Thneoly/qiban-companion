//! Host coordination for explicit manual memory mutations.
use crate::chat::ChatState;
use companion_core::memory::{Memory, MemoryDraft, MemoryError};
use companion_storage::{memory::MemoryCommit, StorageError};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFailure {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub affected_scopes: Vec<companion_core::memory::MemoryScope>,
    pub message: &'static str,
}
impl MemoryFailure {
    pub(crate) fn unavailable() -> Self {
        Self {
            affected_scopes: Vec::new(),
            code: "storage_unavailable",
            message: "本机记忆不可用，请刷新或检查数据目录",
        }
    }
    pub(crate) fn export() -> Self {
        Self {
            affected_scopes: Vec::new(),
            code: "export_failed",
            message: "导出失败，未报告保存成功；请选择可写的JSON文件位置重试",
        }
    }
    pub(crate) fn changed() -> Self {
        Self {
            affected_scopes: Vec::new(),
            code: "context_changed",
            message: "记忆或会话已变化，请刷新后重试",
        }
    }
}
impl From<StorageError> for MemoryFailure {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Memory(error) => match error {
                MemoryError::ConfirmationRequired => Self {
                    affected_scopes: Vec::new(),
                    code: "confirmation_required",
                    message: "请先确认清空本机全部聊天",
                },
                MemoryError::InvalidInput | MemoryError::UnsupportedKind => Self {
                    affected_scopes: Vec::new(),
                    code: "invalid_input",
                    message: "仅可保存偏好或经历，正文需1～200字且日期有效",
                },
                MemoryError::CapacityExceeded => Self {
                    affected_scopes: Vec::new(),
                    code: "capacity_exceeded",
                    message: "最多保留30条记忆，请先整理已有内容",
                },
                MemoryError::SelectionTooLarge => Self {
                    affected_scopes: Vec::new(),
                    code: "selection_too_large",
                    message: "修改后超出已选记忆预算，请先调整选择",
                },
                MemoryError::Conflict | MemoryError::NotFound => Self {
                    affected_scopes: Vec::new(),
                    code: "conflict",
                    message: "条目已更改或删除，请刷新后重试",
                },
                MemoryError::ContextChanged => Self::changed(),
                MemoryError::CounterExhausted => Self::unavailable(),
            },
            _ => Self::unavailable(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshot {
    pub items: Vec<Memory>,
    pub context_epoch: i64,
}
#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MemoryMutation {
    Create {
        draft: MemoryDraft,
        expected_epoch: i64,
    },
    Update {
        id: String,
        expected_revision: i64,
        expected_epoch: i64,
        draft: MemoryDraft,
        restart_conversation: bool,
    },
    Delete {
        id: String,
        expected_revision: i64,
        expected_epoch: i64,
        restart_conversation: bool,
    },
    DeleteAll {
        expected_epoch: i64,
        restart_conversation: bool,
    },
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryChanged {
    pub context_epoch: i64,
    pub chat_cleared: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReceipt {
    pub context_epoch: i64,
    pub chat_cleared: bool,
    pub notifications_delivered: bool,
}

impl ChatState {
    pub(crate) fn memory_snapshot(&self) -> Result<MemorySnapshot, MemoryFailure> {
        let inner = self.0.lock().map_err(|_| MemoryFailure::unavailable())?;
        let store = inner
            .store
            .as_ref()
            .ok_or_else(MemoryFailure::unavailable)?;
        Ok(MemorySnapshot {
            items: store.memory_list()?,
            context_epoch: store.context_epoch()?,
        })
    }
    // Notification happens under the same lock so queued deltas cannot overtake invalidation.
    fn change_memory(
        &self,
        request: MemoryMutation,
        notify: impl FnOnce(MemoryChanged) -> bool,
    ) -> Result<MemoryReceipt, MemoryFailure> {
        let confirmed = match &request {
            MemoryMutation::Create { .. } => true,
            MemoryMutation::Update {
                restart_conversation,
                ..
            }
            | MemoryMutation::Delete {
                restart_conversation,
                ..
            }
            | MemoryMutation::DeleteAll {
                restart_conversation,
                ..
            } => *restart_conversation,
        };
        if !confirmed {
            return Err(MemoryFailure {
                affected_scopes: Vec::new(),
                code: "confirmation_required",
                message: "请先确认停止当前回复并清空本机全部模型的聊天记录",
            });
        }
        let mut inner = self.0.lock().map_err(|_| MemoryFailure::unavailable())?;
        let store = inner
            .store
            .as_mut()
            .ok_or_else(MemoryFailure::unavailable)?;
        let previous_epoch = store.context_epoch()?;
        let (context_epoch, chat_cleared) = match request {
            MemoryMutation::Create {
                draft,
                expected_epoch,
            } => {
                let result = store.memory_create(&draft, expected_epoch)?;
                (result.context_epoch, result.chat_cleared)
            }
            MemoryMutation::Update {
                id,
                expected_revision,
                expected_epoch,
                draft,
                ..
            } => {
                if previous_epoch != expected_epoch {
                    return Err(MemoryFailure::changed());
                }
                let affected_scopes = store.memory_budget_conflicts(&id, &draft)?;
                if !affected_scopes.is_empty() {
                    return Err(MemoryFailure {
                        code: "selection_too_large",
                        message: "更正将超出这些模型的记忆预算，请先减少选择",
                        affected_scopes,
                    });
                }
                let result = store.memory_update(&id, expected_revision, expected_epoch, &draft)?;
                (result.context_epoch, result.chat_cleared)
            }
            MemoryMutation::Delete {
                id,
                expected_revision,
                expected_epoch,
                ..
            } => {
                let result = store.memory_delete(&id, expected_revision, expected_epoch)?;
                (result.context_epoch, result.chat_cleared)
            }
            MemoryMutation::DeleteAll { expected_epoch, .. } => {
                let MemoryCommit {
                    context_epoch,
                    chat_cleared,
                    ..
                } = store.memory_delete_all(expected_epoch)?;
                (context_epoch, chat_cleared)
            }
        };
        // An idempotent delete retry must not stop a newer conversation.
        if context_epoch != previous_epoch {
            inner.invalidate_memory(chat_cleared);
        }
        let notifications_delivered = notify(MemoryChanged {
            context_epoch,
            chat_cleared,
        });
        Ok(MemoryReceipt {
            context_epoch,
            chat_cleared,
            notifications_delivered,
        })
    }
}
#[tauri::command]
pub fn memory_list(state: State<'_, ChatState>) -> Result<MemorySnapshot, MemoryFailure> {
    state.memory_snapshot()
}
#[tauri::command]
pub fn memory_mutate(
    app: AppHandle,
    state: State<'_, ChatState>,
    request: MemoryMutation,
) -> Result<MemoryReceipt, MemoryFailure> {
    state.change_memory(request, |event| app.emit("memory-changed", event).is_ok())
}
/// Read-only pet refresh checkpoint; no memory contents or policy grants.
#[tauri::command]
pub fn chat_context_epoch(state: State<'_, ChatState>) -> Result<i64, MemoryFailure> {
    let inner = state.0.lock().map_err(|_| MemoryFailure::unavailable())?;
    Ok(inner
        .store
        .as_ref()
        .ok_or_else(MemoryFailure::unavailable)?
        .context_epoch()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use companion_core::{conversation::ChatTurn, memory::MemoryKind};
    fn draft() -> MemoryDraft {
        MemoryDraft {
            kind: MemoryKind::Preference,
            body: "合成偏好".into(),
            event_date: None,
        }
    }
    #[test]
    fn crash_after_commit_child() {
        let Ok(path) = std::env::var("QIBAN_Q6_COMMITTED_DB") else {
            return;
        };
        let state = ChatState::open(std::path::Path::new(&path));
        let snapshot = state.memory_snapshot().unwrap();
        let m = &snapshot.items[0];
        state
            .change_memory(
                MemoryMutation::Delete {
                    id: m.id.clone(),
                    expected_revision: m.revision,
                    expected_epoch: snapshot.context_epoch,
                    restart_conversation: true,
                },
                |_| std::process::exit(24),
            )
            .unwrap();
        panic!("notification barrier not reached");
    }
    #[test]
    fn q6_del14_crash_after_commit_before_notification_stays_deleted() {
        let path = std::env::temp_dir().join(format!("q6-committed-{}.db", uuid::Uuid::new_v4()));
        let state = ChatState::open(&path);
        state
            .change_memory(
                MemoryMutation::Create {
                    draft: draft(),
                    expected_epoch: 0,
                },
                |_| true,
            )
            .unwrap();
        {
            let mut inner = state.0.lock().unwrap();
            inner
                .store
                .as_mut()
                .unwrap()
                .append(
                    "https://q6.invalid",
                    "fixture",
                    &ChatTurn {
                        user: "old".into(),
                        assistant: "old".into(),
                    },
                )
                .unwrap();
        }
        drop(state);
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "memory::tests::crash_after_commit_child",
                "--nocapture",
            ])
            .env("QIBAN_Q6_COMMITTED_DB", &path)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(24));
        let state = ChatState::open(&path);
        assert!(state.memory_snapshot().unwrap().items.is_empty());
        assert_eq!(state.memory_snapshot().unwrap().context_epoch, 2);
        assert!(state
            .0
            .lock()
            .unwrap()
            .store
            .as_ref()
            .unwrap()
            .load("https://q6.invalid", "fixture")
            .unwrap()
            .is_empty());
        drop(state);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn confirmation_commit_invalidation_and_notification_failure_are_distinct() {
        let path = std::env::temp_dir().join(format!("memory-host-{}.db", uuid::Uuid::new_v4()));
        let state = ChatState::open(&path);
        state
            .change_memory(
                MemoryMutation::Create {
                    draft: draft(),
                    expected_epoch: 0,
                },
                |_| true,
            )
            .unwrap();
        let memory = state.memory_snapshot().unwrap().items.remove(0);
        let mut cancelled = {
            let mut inner = state.0.lock().unwrap();
            let (tx, rx) = tokio::sync::watch::channel(false);
            inner.active = Some(("old".into(), tx));
            inner.conversation.restore(
                "a",
                "m",
                vec![ChatTurn {
                    user: "old".into(),
                    assistant: "old".into(),
                }],
            );
            rx
        };
        let delete = |confirmed, epoch| MemoryMutation::Delete {
            id: memory.id.clone(),
            expected_revision: 1,
            expected_epoch: epoch,
            restart_conversation: confirmed,
        };
        assert_eq!(
            state
                .change_memory(delete(false, 1), |_| panic!())
                .unwrap_err()
                .code,
            "confirmation_required"
        );
        assert_eq!(
            state
                .change_memory(delete(true, 0), |_| panic!())
                .unwrap_err()
                .code,
            "context_changed"
        );
        assert!(!*cancelled.borrow_and_update());
        let receipt = state.change_memory(delete(true, 1), |_| false).unwrap();
        assert!(!receipt.notifications_delivered);
        assert!(receipt.chat_cleared);
        assert_eq!(receipt.context_epoch, 2);
        assert!(*cancelled.borrow_and_update());
        assert!(state.0.lock().unwrap().conversation.history().is_empty());
        assert!(state.memory_snapshot().unwrap().items.is_empty());
        let (signal, _) = tokio::sync::watch::channel(false);
        state.0.lock().unwrap().active = Some(("new".into(), signal));
        assert!(
            !state
                .change_memory(delete(true, 1), |_| true)
                .unwrap()
                .chat_cleared
        );
        assert!(state.0.lock().unwrap().active.is_some());
        drop(state);
        std::fs::remove_file(path).unwrap();
    }
}

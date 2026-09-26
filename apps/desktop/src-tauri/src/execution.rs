//! A local, explicit one-action executor. Never accepts filesystem paths from IPC.
use companion_core::execution::{digest, ExecutionDetail, ExecutionStatus, ExecutionTask};
use companion_storage::{execution::ExecutionStore, StorageError};
use serde::Serialize;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Manager, State, WebviewWindow};

#[derive(Debug, Serialize)]
pub struct ExecutionFailure {
    code: &'static str,
    message: String,
}
impl ExecutionFailure {
    fn io() -> Self {
        Self {
            code: "artifact_unavailable",
            message: "草稿文件不可用或内容不一致，请核对结果；不会自动覆盖或重试".into(),
        }
    }
    fn busy() -> Self {
        Self {
            code: "execution_busy",
            message: "正在保存或核对，请稍后刷新；不要重复提交".into(),
        }
    }
}
impl From<StorageError> for ExecutionFailure {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Execution(e) => Self {
                code: "execution_conflict",
                message: e.to_string(),
            },
            StorageError::NotFound => Self {
                code: "not_found",
                message: "未找到执行记录".into(),
            },
            _ => Self {
                code: "storage_unavailable",
                message: "执行记录未能保存，请刷新核对；不要重新创建任务".into(),
            },
        }
    }
}
pub struct ExecutionState {
    pub store: ExecutionStore,
    root: PathBuf,
    owner: String,
    operation: Mutex<()>,
}
fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
impl ExecutionState {
    pub fn open(data: &Path) -> Result<Self, StorageError> {
        let store = ExecutionStore::open(&data.join("executions.db"))?;
        store.interrupt_previous_owner()?;
        let state = Self {
            store,
            root: data.join("document-drafts"),
            owner: uuid::Uuid::new_v4().to_string(),
            operation: Mutex::new(()),
        };
        for task in state.store.list()? {
            if task.status == ExecutionStatus::Unknown {
                let _ = state.reconcile(&task.id);
            }
        }
        Ok(state)
    }
    fn artifact_path(&self, task: &ExecutionTask) -> Result<PathBuf, ExecutionFailure> {
        let action = uuid::Uuid::parse_str(&task.action_id).map_err(|_| ExecutionFailure::io())?;
        let expected = format!("{action}.md");
        if expected != task.artifact_name {
            return Err(ExecutionFailure::io());
        }
        std::fs::create_dir_all(&self.root).map_err(|_| ExecutionFailure::io())?;
        let metadata = std::fs::symlink_metadata(&self.root).map_err(|_| ExecutionFailure::io())?;
        if is_link(&metadata) || !metadata.is_dir() {
            return Err(ExecutionFailure::io());
        }
        Ok(self.root.join(expected))
    }
    fn read_artifact(&self, task: &ExecutionTask) -> Result<Option<String>, ExecutionFailure> {
        let path = self.artifact_path(task)?;
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(ExecutionFailure::io()),
        };
        if is_link(&metadata) || !metadata.is_file() || metadata.len() > 32 * 1024 {
            return Err(ExecutionFailure::io());
        }
        let mut content = String::new();
        std::fs::File::open(path)
            .map_err(|_| ExecutionFailure::io())?
            .take(32 * 1024 + 1)
            .read_to_string(&mut content)
            .map_err(|_| ExecutionFailure::io())?;
        if content != task.preview || digest(content.as_bytes()) != task.artifact_hash {
            return Err(ExecutionFailure::io());
        }
        Ok(Some(content))
    }
    fn publish(&self, task: &ExecutionTask) -> Result<(), ExecutionFailure> {
        let destination = self.artifact_path(task)?;
        let pending = self.root.join(format!("{}.pending", task.action_id));
        // Exclusive create plus atomic no-replace hard link. Never truncate an existing file.
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)
            .map_err(|_| ExecutionFailure::io())?;
        let outcome = (|| -> std::io::Result<()> {
            file.write_all(task.preview.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::hard_link(&pending, &destination)
        })();
        let _ = std::fs::remove_file(&pending); // only our fixed, newly created staging file
        outcome.map_err(|_| ExecutionFailure::io())
    }
    fn resolve_file(&self, id: &str) -> Result<ExecutionDetail, ExecutionFailure> {
        let detail = self.store.detail(id)?;
        if !matches!(
            detail.task.status,
            ExecutionStatus::Running | ExecutionStatus::Unknown
        ) {
            return Ok(detail);
        }
        let attempt = detail.attempts.last().ok_or_else(ExecutionFailure::io)?;
        match self.read_artifact(&detail.task) {
            Ok(Some(_)) => {
                self.store.resolve(
                    id,
                    &attempt.id,
                    ExecutionStatus::Completed,
                    "草稿存在且内容核验通过",
                )?;
            }
            Ok(None) => {
                self.store.resolve(
                    id,
                    &attempt.id,
                    ExecutionStatus::Failed,
                    "未发现已提交草稿，未自动重试",
                )?;
            }
            Err(_) => {
                if detail.task.status == ExecutionStatus::Running {
                    self.store.resolve(
                        id,
                        &attempt.id,
                        ExecutionStatus::Unknown,
                        "文件无法核验，不能确认是否完成；请核对结果",
                    )?;
                }
            }
        }
        Ok(self.store.detail(id)?)
    }
    pub fn execute(&self, id: &str, revision: u32) -> Result<ExecutionDetail, ExecutionFailure> {
        let _guard = self
            .operation
            .try_lock()
            .map_err(|_| ExecutionFailure::busy())?;
        let Some(attempt) = self.store.claim(id, revision, &self.owner)? else {
            // A repeated confirmation must not mask an externally removed/changed artifact.
            self.result(id)?;
            return Ok(self.store.detail(id)?);
        };
        let task = self.store.detail(id)?.task;
        if self.store.may_publish(id, &attempt)? {
            let _ = self.publish(&task);
        }
        self.resolve_file(id)
    }
    pub fn reconcile(&self, id: &str) -> Result<ExecutionDetail, ExecutionFailure> {
        let _guard = self
            .operation
            .try_lock()
            .map_err(|_| ExecutionFailure::busy())?;
        self.resolve_file(id)
    }
    pub fn result(&self, id: &str) -> Result<String, ExecutionFailure> {
        let detail = self.store.detail(id)?;
        if detail.task.status != ExecutionStatus::Completed {
            return Err(ExecutionFailure::io());
        }
        self.read_artifact(&detail.task)?
            .ok_or_else(ExecutionFailure::io)
    }
}
#[tauri::command]
pub fn execution_list(
    state: State<'_, ExecutionState>,
) -> Result<Vec<ExecutionTask>, ExecutionFailure> {
    Ok(state.store.list()?)
}
#[tauri::command]
pub fn execution_prepare(
    request_id: String,
    source_name: String,
    text: String,
    state: State<'_, ExecutionState>,
) -> Result<ExecutionDetail, ExecutionFailure> {
    let task = state.store.prepare(&request_id, &source_name, &text)?;
    Ok(state.store.detail(&task.id)?)
}
#[tauri::command]
pub fn execution_detail(
    id: String,
    state: State<'_, ExecutionState>,
) -> Result<ExecutionDetail, ExecutionFailure> {
    Ok(state.store.detail(&id)?)
}
#[tauri::command]
pub fn execution_cancel(
    id: String,
    expected_revision: u32,
    state: State<'_, ExecutionState>,
) -> Result<ExecutionDetail, ExecutionFailure> {
    state.store.cancel(&id, expected_revision)?;
    Ok(state.store.detail(&id)?)
}
#[tauri::command]
pub async fn execution_confirm(
    id: String,
    expected_revision: u32,
    window: WebviewWindow,
) -> Result<ExecutionDetail, ExecutionFailure> {
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<ExecutionState>()
            .execute(&id, expected_revision)
    })
    .await
    .map_err(|_| ExecutionFailure::busy())?
}
#[tauri::command]
pub async fn execution_reconcile(
    id: String,
    window: WebviewWindow,
) -> Result<ExecutionDetail, ExecutionFailure> {
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<ExecutionState>().reconcile(&id))
        .await
        .map_err(|_| ExecutionFailure::busy())?
}
#[tauri::command]
pub fn execution_result(
    id: String,
    state: State<'_, ExecutionState>,
) -> Result<String, ExecutionFailure> {
    state.result(&id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let p = std::env::temp_dir().join(format!("qiban-execution-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn failed_receipt_commit_keeps_action_recoverable() {
        let root = root();
        let id;
        {
            let state = ExecutionState::open(&root).unwrap();
            let task = state
                .store
                .prepare(&uuid::Uuid::new_v4().to_string(), "a.txt", "落盘后失败")
                .unwrap();
            id = task.id.clone();
            let db = rusqlite::Connection::open(root.join("executions.db")).unwrap();
            db.execute_batch("CREATE TRIGGER reject_complete BEFORE UPDATE ON execution_tasks WHEN instr(NEW.body, '\"status\":\"completed\"') > 0 BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
            assert!(state.execute(&id, 0).is_err());
            assert_eq!(
                state.store.detail(&id).unwrap().task.status,
                ExecutionStatus::Running
            );
            assert!(state.read_artifact(&task).unwrap().is_some());
            db.execute_batch("DROP TRIGGER reject_complete").unwrap();
        }
        {
            let state = ExecutionState::open(&root).unwrap();
            assert_eq!(
                state.store.detail(&id).unwrap().task.status,
                ExecutionStatus::Completed
            );
            assert_eq!(state.store.detail(&id).unwrap().attempts.len(), 1);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn real_file_and_duplicate_receipt_are_idempotent() {
        let root = root();
        let state = ExecutionState::open(&root).unwrap();
        let task = state
            .store
            .prepare(&uuid::Uuid::new_v4().to_string(), "demo.txt", "真实内容")
            .unwrap();
        assert!(!root.join("document-drafts").exists());
        assert_eq!(
            state.execute(&task.id, 0).unwrap().task.status,
            ExecutionStatus::Completed
        );
        state.execute(&task.id, 0).unwrap();
        assert_eq!(state.store.detail(&task.id).unwrap().attempts.len(), 1);
        assert_eq!(
            std::fs::read_dir(root.join("document-drafts"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(state.result(&task.id).unwrap(), task.preview);
        std::fs::write(
            root.join("document-drafts").join(&task.artifact_name),
            "篡改",
        )
        .unwrap();
        assert!(state.result(&task.id).is_err());
        assert!(state.execute(&task.id, 0).is_err());
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn restart_after_publish_before_receipt_reconciles_without_reexecution() {
        let root = root();
        let id;
        {
            let state = ExecutionState::open(&root).unwrap();
            let task = state
                .store
                .prepare(&uuid::Uuid::new_v4().to_string(), "demo.txt", "checkpoint")
                .unwrap();
            id = task.id.clone();
            state.store.claim(&id, 0, "old-process").unwrap();
            state.publish(&task).unwrap();
        }
        {
            let state = ExecutionState::open(&root).unwrap();
            let detail = state.store.detail(&id).unwrap();
            assert_eq!(detail.task.status, ExecutionStatus::Completed);
            assert_eq!(detail.attempts.len(), 1);
            assert_eq!(
                std::fs::read_dir(root.join("document-drafts"))
                    .unwrap()
                    .count(),
                1
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn before_publish_crash_and_conflicting_output_never_claim_success() {
        let root = root();
        let (missing, conflict);
        {
            let state = ExecutionState::open(&root).unwrap();
            let a = state
                .store
                .prepare(&uuid::Uuid::new_v4().to_string(), "a.txt", "a")
                .unwrap();
            let b = state
                .store
                .prepare(&uuid::Uuid::new_v4().to_string(), "b.txt", "b")
                .unwrap();
            state.store.claim(&a.id, 0, "old").unwrap();
            state.store.claim(&b.id, 0, "old").unwrap();
            let target = state.artifact_path(&b).unwrap();
            std::fs::write(target, "keep").unwrap();
            missing = a.id;
            conflict = b.id;
        }
        {
            let state = ExecutionState::open(&root).unwrap();
            assert_eq!(
                state.store.detail(&missing).unwrap().task.status,
                ExecutionStatus::Failed
            );
            let b = state.store.detail(&conflict).unwrap().task;
            assert_eq!(b.status, ExecutionStatus::Unknown);
            assert!(state.execute(&b.id, b.revision).is_err());
            assert_eq!(
                std::fs::read_to_string(state.artifact_path(&b).unwrap()).unwrap(),
                "keep"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

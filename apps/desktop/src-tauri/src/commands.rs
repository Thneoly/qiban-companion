use companion_core::{Task, PROTOCOL_VERSION};
use companion_storage::{StorageError, TaskStore};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    protocol_version: u32,
    app_version: &'static str,
    runtime: &'static str,
    persistence: &'static str,
    executor_available: bool,
}

#[derive(Serialize)]
pub struct CommandError {
    code: &'static str,
    message: String,
}

impl From<StorageError> for CommandError {
    fn from(error: StorageError) -> Self {
        let (code, message) = match error {
            StorageError::Domain(e) => ("invalid_request", e.to_string()),
            StorageError::NotFound => ("not_found", "未找到任务".into()),
            StorageError::NewerSchema => ("schema_mismatch", "请更新客户端后重试".into()),
            _ => (
                "storage_error",
                "本地存储操作失败，请重试或重启客户端".into(),
            ),
        };
        Self { code, message }
    }
}

#[tauri::command]
pub fn get_runtime_info() -> RuntimeInfo {
    RuntimeInfo {
        protocol_version: PROTOCOL_VERSION,
        app_version: env!("CARGO_PKG_VERSION"),
        runtime: "desktop",
        persistence: "sqlite",
        executor_available: false,
    }
}

#[tauri::command]
pub fn list_tasks(store: State<'_, TaskStore>) -> Result<Vec<Task>, CommandError> {
    store.list().map_err(Into::into)
}

#[tauri::command]
pub fn create_task(title: String, store: State<'_, TaskStore>) -> Result<Task, CommandError> {
    store.create(&title).map_err(Into::into)
}

#[tauri::command]
pub fn cancel_task(id: String, store: State<'_, TaskStore>) -> Result<Task, CommandError> {
    store.cancel(&id).map_err(Into::into)
}

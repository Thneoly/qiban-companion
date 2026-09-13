//! SQLite is owned by Rust. Frontend callers cannot choose paths or execute SQL.
use companion_core::{DomainError, Task};
use rusqlite::{params, Connection, OptionalExtension};
use std::{path::Path, sync::Mutex, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("本地存储暂时不可用")]
    Unavailable,
    #[error("未找到任务")]
    NotFound,
    #[error("数据库来自更新的应用版本，请更新客户端")]
    NewerSchema,
    #[error("任务时间戳超出存储范围")]
    InvalidTimestamp,
}

pub struct TaskStore(Mutex<Connection>);

impl TaskStore {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(mut connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 1 {
            return Err(StorageError::NewerSchema);
        }
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE tasks (id TEXT PRIMARY KEY NOT NULL, created_at INTEGER NOT NULL, body TEXT NOT NULL);
                 PRAGMA user_version = 1;",
            )?;
            transaction.commit()?;
        }
        Ok(Self(Mutex::new(connection)))
    }

    pub fn list(&self) -> Result<Vec<Task>, StorageError> {
        let connection = self.0.lock().map_err(|_| StorageError::Unavailable)?;
        let mut statement =
            connection.prepare("SELECT body FROM tasks ORDER BY created_at DESC, id ASC")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn create(&self, title: &str) -> Result<Task, StorageError> {
        let task = Task::new(title)?;
        let created_at =
            i64::try_from(task.created_at).map_err(|_| StorageError::InvalidTimestamp)?;
        let connection = self.0.lock().map_err(|_| StorageError::Unavailable)?;
        connection.execute(
            "INSERT INTO tasks (id, created_at, body) VALUES (?1, ?2, ?3)",
            params![task.id, created_at, serde_json::to_string(&task)?],
        )?;
        Ok(task)
    }

    pub fn cancel(&self, id: &str) -> Result<Task, StorageError> {
        let mut connection = self.0.lock().map_err(|_| StorageError::Unavailable)?;
        let transaction = connection.transaction()?;
        let body: String = transaction
            .query_row("SELECT body FROM tasks WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?
            .ok_or(StorageError::NotFound)?;
        let mut task: Task = serde_json::from_str(&body)?;
        task.cancel_queued()?;
        transaction.execute(
            "UPDATE tasks SET body = ?1 WHERE id = ?2",
            params![serde_json::to_string(&task)?, task.id],
        )?;
        transaction.commit()?;
        Ok(task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_across_reopen_and_repeated_cancel() {
        let path = std::env::temp_dir().join(format!("companion-test-{}.db", uuid::Uuid::new_v4()));
        let id;
        {
            let store = TaskStore::open(&path).unwrap();
            id = store.create("  写下一个想法  ").unwrap().id;
        }
        {
            let store = TaskStore::open(&path).unwrap();
            assert_eq!(store.list().unwrap()[0].title, "写下一个想法");
            store.cancel(&id).unwrap();
            store.cancel(&id).unwrap();
        }
        {
            let store = TaskStore::open(&path).unwrap();
            let task = store.list().unwrap().remove(0);
            assert_eq!(task.status, companion_core::TaskStatus::Cancelled);
            assert_eq!(task.revision, 1);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn refuses_unknown_schema_and_missing_tasks() {
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        assert!(matches!(
            TaskStore::from_connection(connection),
            Err(StorageError::NewerSchema)
        ));
        let store = TaskStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        assert!(matches!(
            store.cancel("missing"),
            Err(StorageError::NotFound)
        ));
        assert!(store.create(" ").is_err());
        assert!(store.list().unwrap().is_empty());
    }
}

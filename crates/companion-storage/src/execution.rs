//! One durable local action ledger. Events and state are committed together.
use crate::StorageError;
use companion_core::execution::{
    prepare_document, ExecutionAttempt, ExecutionDetail, ExecutionError, ExecutionEvent,
    ExecutionStatus, ExecutionTask,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::{path::Path, sync::Mutex, time::Duration};

pub struct ExecutionStore(Mutex<Connection>);
fn task(connection: &Connection, id: &str) -> Result<ExecutionTask, StorageError> {
    let body: String = connection
        .query_row("SELECT body FROM execution_tasks WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or(StorageError::NotFound)?;
    Ok(serde_json::from_str(&body)?)
}
fn write_task(tx: &Transaction<'_>, task: &ExecutionTask) -> Result<(), StorageError> {
    tx.execute(
        "UPDATE execution_tasks SET body=?1 WHERE id=?2",
        params![serde_json::to_string(task)?, task.id],
    )?;
    tx.execute(
        "INSERT INTO execution_events(task_id,revision,status,created_at) VALUES(?1,?2,?3,?4)",
        params![
            task.id,
            task.revision,
            serde_json::to_string(&task.status)?,
            i64::try_from(task.updated_at).map_err(|_| StorageError::InvalidTimestamp)?
        ],
    )?;
    Ok(())
}
impl ExecutionStore {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Self::from_connection(Connection::open(path)?)
    }
    fn from_connection(mut connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 1 {
            return Err(StorageError::NewerSchema);
        }
        if version == 0 {
            let tx = connection.transaction()?;
            tx.execute_batch("CREATE TABLE execution_tasks(id TEXT PRIMARY KEY NOT NULL,request_id TEXT NOT NULL UNIQUE,fingerprint TEXT NOT NULL,created_at INTEGER NOT NULL,body TEXT NOT NULL);
                CREATE TABLE execution_actions(id TEXT PRIMARY KEY NOT NULL,task_id TEXT NOT NULL UNIQUE REFERENCES execution_tasks(id),artifact_name TEXT NOT NULL,artifact_hash TEXT NOT NULL,approved_revision INTEGER);
                CREATE TABLE execution_attempts(id TEXT PRIMARY KEY NOT NULL,task_id TEXT NOT NULL REFERENCES execution_tasks(id),body TEXT NOT NULL);
                CREATE TABLE execution_events(sequence INTEGER PRIMARY KEY AUTOINCREMENT,task_id TEXT NOT NULL REFERENCES execution_tasks(id),revision INTEGER NOT NULL,status TEXT NOT NULL,created_at INTEGER NOT NULL,UNIQUE(task_id,revision));
                PRAGMA user_version=1;")?;
            tx.commit()?;
        }
        Ok(Self(Mutex::new(connection)))
    }
    pub fn prepare(
        &self,
        request_id: &str,
        name: &str,
        text: &str,
    ) -> Result<ExecutionTask, StorageError> {
        let prepared = prepare_document(request_id, name, text)?;
        let request_id = uuid::Uuid::parse_str(request_id)
            .map_err(|_| ExecutionError::InvalidRequest)?
            .to_string();
        let mut c = self.0.lock().map_err(|_| StorageError::Unavailable)?;
        let tx = c.transaction()?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT id,fingerprint FROM execution_tasks WHERE request_id=?1",
                [&request_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, fingerprint)) = prior {
            if fingerprint != prepared.fingerprint {
                return Err(ExecutionError::RequestConflict.into());
            }
            return task(&tx, &id);
        }
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM execution_tasks", [], |r| r.get(0))?;
        if count >= 100 {
            return Err(ExecutionError::Capacity.into());
        }
        let fingerprint = prepared.fingerprint.clone();
        let task = ExecutionTask::new(prepared);
        tx.execute(
            "INSERT INTO execution_tasks VALUES(?1,?2,?3,?4,?5)",
            params![
                task.id,
                request_id,
                fingerprint,
                i64::try_from(task.created_at).map_err(|_| StorageError::InvalidTimestamp)?,
                serde_json::to_string(&task)?
            ],
        )?;
        tx.execute("INSERT INTO execution_actions(id,task_id,artifact_name,artifact_hash) VALUES(?1,?2,?3,?4)",params![task.action_id,task.id,task.artifact_name,task.artifact_hash])?;
        write_task(&tx, &task)?;
        tx.commit()?;
        Ok(task)
    }
    pub fn list(&self) -> Result<Vec<ExecutionTask>, StorageError> {
        let c = self.0.lock().map_err(|_| StorageError::Unavailable)?;
        let mut q = c.prepare("SELECT body FROM execution_tasks ORDER BY created_at DESC,id")?;
        let rows = q.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn detail(&self, id: &str) -> Result<ExecutionDetail, StorageError> {
        let c = self.0.lock().map_err(|_| StorageError::Unavailable)?;
        let task = task(&c, id)?;
        let mut q =
            c.prepare("SELECT body FROM execution_attempts WHERE task_id=?1 ORDER BY rowid")?;
        let attempts = q
            .query_map([id], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect::<Result<Vec<_>, StorageError>>()?;
        let mut q=c.prepare("SELECT sequence,revision,status,created_at FROM execution_events WHERE task_id=?1 ORDER BY sequence")?;
        let raw = q.query_map([id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let events = raw
            .map(|r| {
                let (sequence, revision, status, created_at) = r?;
                Ok(ExecutionEvent {
                    sequence: u64::try_from(sequence)
                        .map_err(|_| StorageError::InvalidTimestamp)?,
                    task_id: id.into(),
                    revision,
                    status: serde_json::from_str(&status)?,
                    created_at: u64::try_from(created_at)
                        .map_err(|_| StorageError::InvalidTimestamp)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok(ExecutionDetail {
            task,
            attempts,
            events,
        })
    }
    pub fn claim(
        &self,
        id: &str,
        revision: u32,
        owner: &str,
    ) -> Result<Option<ExecutionAttempt>, StorageError> {
        let mut c = self.0.lock().map_err(|_| StorageError::Unavailable)?;
        let tx = c.transaction()?;
        let mut task = task(&tx, id)?;
        if task.status == ExecutionStatus::Completed {
            return Ok(None);
        }
        task.transition(revision, ExecutionStatus::Running, "已确认，正在保存草稿")?;
        let attempt = ExecutionAttempt {
            id: uuid::Uuid::new_v4().to_string(),
            action_id: task.action_id.clone(),
            owner: owner.into(),
            lease_until: companion_core::now_ms() + 30_000,
            status: ExecutionStatus::Running,
        };
        tx.execute(
            "UPDATE execution_actions SET approved_revision=?1 WHERE task_id=?2",
            params![revision, id],
        )?;
        tx.execute(
            "INSERT INTO execution_attempts VALUES(?1,?2,?3)",
            params![attempt.id, id, serde_json::to_string(&attempt)?],
        )?;
        write_task(&tx, &task)?;
        tx.commit()?;
        Ok(Some(attempt))
    }
    pub fn may_publish(&self, id: &str, attempt: &ExecutionAttempt) -> Result<bool, StorageError> {
        let detail = self.detail(id)?;
        Ok(detail.task.status == ExecutionStatus::Running
            && detail.attempts.last().is_some_and(|a| {
                a.id == attempt.id
                    && a.owner == attempt.owner
                    && a.lease_until >= companion_core::now_ms()
            }))
    }
    pub fn cancel(&self, id: &str, revision: u32) -> Result<ExecutionTask, StorageError> {
        let mut c = self.0.lock().map_err(|_| StorageError::Unavailable)?;
        let tx = c.transaction()?;
        let mut task = task(&tx, id)?;
        if task.status == ExecutionStatus::Cancelled {
            return Ok(task);
        }
        task.transition(
            revision,
            ExecutionStatus::Cancelled,
            "已取消，未创建草稿文件",
        )?;
        write_task(&tx, &task)?;
        tx.commit()?;
        Ok(task)
    }
    pub fn resolve(
        &self,
        id: &str,
        attempt_id: &str,
        next: ExecutionStatus,
        note: &str,
    ) -> Result<ExecutionTask, StorageError> {
        let mut c = self.0.lock().map_err(|_| StorageError::Unavailable)?;
        let tx = c.transaction()?;
        let mut task = task(&tx, id)?;
        let (actual_id, body): (String, String) = tx.query_row(
            "SELECT id,body FROM execution_attempts WHERE task_id=?1 ORDER BY rowid DESC LIMIT 1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if actual_id != attempt_id {
            return Err(ExecutionError::Conflict.into());
        }
        if task.status == next {
            return Ok(task);
        }
        task.transition(task.revision, next, note)?;
        let mut attempt: ExecutionAttempt = serde_json::from_str(&body)?;
        attempt.status = next;
        attempt.lease_until = 0;
        tx.execute(
            "UPDATE execution_attempts SET body=?1 WHERE id=?2",
            params![serde_json::to_string(&attempt)?, actual_id],
        )?;
        write_task(&tx, &task)?;
        tx.commit()?;
        Ok(task)
    }
    /// Called only after the application acquired its exclusive profile lease.
    pub fn interrupt_previous_owner(&self) -> Result<(), StorageError> {
        for task in self.list()? {
            if task.status == ExecutionStatus::Running {
                let detail = self.detail(&task.id)?;
                let attempt = detail.attempts.last().ok_or(StorageError::Unavailable)?;
                self.resolve(
                    &task.id,
                    &attempt.id,
                    ExecutionStatus::Unknown,
                    "上次执行中断，等待核对实际文件",
                )?;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durable_dedup_confirmation_and_recovery_fence() {
        let path = std::env::temp_dir().join(format!("execution-{}.db", uuid::Uuid::new_v4()));
        let request = uuid::Uuid::new_v4().to_string();
        let id;
        {
            let store = ExecutionStore::open(&path).unwrap();
            let task = store.prepare(&request, "a.txt", "one").unwrap();
            id = task.id.clone();
            assert_eq!(store.prepare(&request, "a.txt", "one").unwrap().id, id);
            assert!(store.prepare(&request, "a.txt", "different").is_err());
            assert!(store.claim(&id, 9, "owner-a").is_err());
            let attempt = store.claim(&id, 0, "owner-a").unwrap().unwrap();
            assert!(store.may_publish(&id, &attempt).unwrap());
            assert!(store.claim(&id, 0, "owner-b").is_err());
            assert!(store.cancel(&id, 1).is_err());
        }
        {
            let store = ExecutionStore::open(&path).unwrap();
            store.interrupt_previous_owner().unwrap();
            let d = store.detail(&id).unwrap();
            assert_eq!(d.task.status, ExecutionStatus::Unknown);
            assert!(!store.may_publish(&id, &d.attempts[0]).unwrap());
            assert!(store
                .resolve(&id, "stale", ExecutionStatus::Completed, "")
                .is_err());
            store
                .resolve(&id, &d.attempts[0].id, ExecutionStatus::Completed, "已核对")
                .unwrap();
            assert!(store.claim(&id, 0, "retry").unwrap().is_none());
            let d = store.detail(&id).unwrap();
            assert_eq!(d.attempts.len(), 1);
            assert_eq!(d.events.len(), 4);
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn cancellation_and_newer_database_are_not_bypassed() {
        let store = ExecutionStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let task = store
            .prepare(&uuid::Uuid::new_v4().to_string(), "a.md", "hello")
            .unwrap();
        store.cancel(&task.id, 0).unwrap();
        store.cancel(&task.id, 0).unwrap();
        assert!(store.claim(&task.id, 0, "owner").is_err());
        assert_eq!(store.detail(&task.id).unwrap().events.len(), 2);
        let c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "user_version", 2).unwrap();
        assert!(matches!(
            ExecutionStore::from_connection(c),
            Err(StorageError::NewerSchema)
        ));
    }
}

//! Local intent ledger for remotely confirmed excerpts; never accepts frontend SQL or paths.
use crate::StorageError;
use companion_core::{authorization::ShareDocument, execution::ExecutionTask};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub task: ExecutionTask,
    pub share: ShareDocument,
}
pub struct RemoteJobStore(Connection);
impl RemoteJobStore {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut db = Connection::open(path)?;
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let appid: u32 = db.pragma_query_value(None, "application_id", |r| r.get(0))?;
        if version == 0 {
            let tables: u32 = db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0))?;
            if tables != 0 || appid != 0 {
                return Err(StorageError::Unavailable);
            }
            let tx = db.transaction()?;
            tx.execute_batch("CREATE TABLE remote_jobs(id TEXT PRIMARY KEY NOT NULL,body TEXT NOT NULL,phase TEXT NOT NULL);PRAGMA user_version=1;PRAGMA application_id=1363296594;")?;
            tx.commit()?;
        } else if version != 1 || appid != 1363296594 {
            return Err(StorageError::NewerSchema);
        }
        Ok(Self(db))
    }
    pub fn contains(&self, id: &str) -> Result<bool, StorageError> {
        Ok(self.0.query_row(
            "SELECT EXISTS(SELECT 1 FROM remote_jobs WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?)
    }
    pub fn load(&self, id: &str) -> Result<(Prepared, String), StorageError> {
        let (body, phase): (String, String) = self.0.query_row(
            "SELECT body,phase FROM remote_jobs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((serde_json::from_str(&body)?, phase))
    }
    pub fn insert(&self, value: &Prepared) -> Result<(), StorageError> {
        self.0.execute(
            "INSERT INTO remote_jobs VALUES(?1,?2,'local')",
            params![value.task.id, serde_json::to_string(value)?],
        )?;
        Ok(())
    }
    pub fn phase(&self, id: &str, phase: &str) -> Result<(), StorageError> {
        if !["local", "sharing", "claiming", "admitted", "reported"].contains(&phase) {
            return Err(StorageError::Unavailable);
        }
        let changed = self.0.execute(
            "UPDATE remote_jobs SET phase=?1 WHERE id=?2",
            params![phase, id],
        )?;
        if changed != 1 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }
}

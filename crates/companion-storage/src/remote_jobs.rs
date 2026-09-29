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
    /// Ids whose phase PROVES the server once listed the record (the sync
    /// loop only reaches claiming/admitted after observing it). Used by the
    /// reverse sweep: absence from the current server list then means the
    /// record was deleted. `sharing` is deliberately excluded — it is set
    /// before the share POST, so "sharing + absent" is indistinguishable
    /// from "the POST never arrived" and must stay retryable.
    pub fn claiming_or_admitted(&self) -> Result<Vec<String>, StorageError> {
        let mut q = self
            .0
            .prepare("SELECT id FROM remote_jobs WHERE phase IN ('claiming','admitted')")?;
        let ids = q
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> RemoteJobStore {
        RemoteJobStore::open(std::path::Path::new(":memory:")).unwrap()
    }
    fn prepared() -> Prepared {
        let document = companion_core::execution::prepare_document(
            &uuid::Uuid::new_v4().to_string(),
            "demo.txt",
            &format!("demo line\n{}", uuid::Uuid::new_v4()),
        )
        .unwrap();
        let task = companion_core::execution::ExecutionTask::new(document);
        Prepared {
            share: ShareDocument {
                pairing_id: uuid::Uuid::new_v4().to_string(),
                binding: companion_core::authorization::ActionBinding {
                    action_id: task.action_id.clone(),
                    resource_id: task.id.clone(),
                    resource_version: 1,
                    parameters_digest: "a".repeat(64),
                    pair_revision: 1,
                    scope: companion_core::authorization::ActionScope::DocumentExcerpt,
                },
                source_name: task.source_name.clone(),
                preview: task.preview.clone(),
            },
            task,
        }
    }
    #[test]
    fn claiming_or_admitted_lists_only_proof_phases() {
        let s = store();
        let mut expected = Vec::new();
        for phase in ["local", "sharing", "claiming", "admitted", "reported"] {
            let p = prepared();
            let id = p.task.id.clone();
            s.insert(&p).unwrap();
            if phase != "local" {
                s.phase(&id, phase).unwrap();
            }
            if phase == "claiming" || phase == "admitted" {
                expected.push(id);
            }
        }
        let mut listed = s.claiming_or_admitted().unwrap();
        listed.sort();
        expected.sort();
        assert_eq!(listed, expected);
        s.phase(expected[0].as_str(), "reported").unwrap();
        assert_eq!(s.claiming_or_admitted().unwrap(), [expected[1].clone()]);
    }
}

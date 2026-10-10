//! Account-scoped memory mirror and sync engine (schema v7). The mirror keeps
//! full tombstones — compaction only drops events, never rows — so a deleted
//! memory can never be resurrected by a stale create or a lagging device.
//! Event ordering is a global AUTOINCREMENT sequence; per-account monotonicity
//! follows for free and watermarks stay unique after compaction.
use super::*;
use companion_core::memory::{validate_synced_body, MemoryKind, MemorySyncError, MAX_MEMORIES};
use serde::{Deserialize, Serialize};

/// Event retention window. Beyond it the account enters a new sync
/// generation: pending events are dropped and lagging devices resync from a
/// snapshot instead of replaying history.
const MEMORY_EVENT_WINDOW: i64 = 512;
/// One pull answers at most this many events; a device keeps pulling while
/// `pending` is true.
const MEMORY_PULL_LIMIT: usize = 200;
/// Per-account request journal bound. Retries of evicted entries fall through
/// to a fresh write attempt instead of replaying forever.
const MEMORY_REQUEST_JOURNAL: i64 = 256;
/// Native-issuer sessions count as live while their token was seen in this
/// window — mirrors pairing liveness in authorization::live.
const SESSION_LIVENESS_MS: i64 = 1_800_000;

pub(super) fn migrate(c: &mut Connection) -> Result<(), StorageError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE account_memories (
           account_id TEXT NOT NULL REFERENCES accounts(id), id TEXT NOT NULL,
           kind TEXT NOT NULL CHECK(kind IN ('preference','experience','task_fact')),
           body TEXT, event_date TEXT,
           revision INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
           deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)), deleted_at INTEGER,
           PRIMARY KEY(account_id,id),
           CHECK((deleted=0 AND body IS NOT NULL AND length(body) BETWEEN 1 AND 200)
              OR (deleted=1 AND body IS NULL AND event_date IS NULL AND deleted_at IS NOT NULL)));
         CREATE INDEX account_memories_list ON account_memories(account_id,deleted,created_at);
         CREATE TABLE memory_events (
           account_id TEXT NOT NULL, seq INTEGER PRIMARY KEY AUTOINCREMENT,
           memory_id TEXT NOT NULL, op TEXT NOT NULL CHECK(op IN ('upsert','delete')),
           revision INTEGER NOT NULL, kind TEXT, body TEXT, event_date TEXT,
           occurred_at INTEGER NOT NULL);
         CREATE INDEX memory_events_pull ON memory_events(account_id,seq);
         CREATE TABLE memory_devices (
           account_id TEXT NOT NULL REFERENCES accounts(id), device_id TEXT NOT NULL,
           generation INTEGER NOT NULL, applied_seq INTEGER NOT NULL, last_seen INTEGER NOT NULL,
           PRIMARY KEY(account_id,device_id));
         CREATE TABLE memory_accounts (
           account_id TEXT PRIMARY KEY REFERENCES accounts(id),
           generation INTEGER NOT NULL DEFAULT 1, watermark INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE memory_requests (
           account_id TEXT NOT NULL, request_id TEXT NOT NULL, request_key TEXT NOT NULL,
           response TEXT NOT NULL, created_at INTEGER NOT NULL,
           PRIMARY KEY(account_id,request_id));
         PRAGMA user_version=7;",
    )?;
    tx.commit()?;
    Ok(())
}

/// Snapshot/list item. Active entries carry the full body; tombstones carry
/// only id and revision so a peer can refuse to resurrect them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshotItem {
    pub id: String,
    pub kind: MemoryKind,
    pub body: Option<String>,
    pub event_date: Option<String>,
    pub revision: i64,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySyncOp {
    Upsert,
    Delete,
}
impl MemorySyncOp {
    fn as_str(self) -> &'static str {
        match self {
            Self::Upsert => "upsert",
            Self::Delete => "delete",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        match value {
            "upsert" => Some(Self::Upsert),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySyncEvent {
    pub seq: i64,
    pub memory_id: String,
    pub op: MemorySyncOp,
    pub revision: i64,
    pub kind: Option<MemoryKind>,
    pub body: Option<String>,
    pub event_date: Option<String>,
    pub occurred_at: i64,
}

/// Pull answer. `snapshot` is set when the caller's generation or cursor fell
/// behind the retention window — the client rebuilds from it wholesale.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySyncResponse {
    pub generation: i64,
    pub cursor: i64,
    pub pending: bool,
    pub events: Vec<MemorySyncEvent>,
    pub snapshot: Option<Vec<MemorySnapshotItem>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryWriteReceipt {
    pub memory_id: String,
    pub revision: i64,
    pub deleted: bool,
}

/// Domain write input shared by the desktop push and the mobile update/delete
/// routes — both peers meet the same conflict rules. Serialized only to build
/// the stable request journal fingerprint.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWrite {
    Create {
        kind: MemoryKind,
        body: String,
        event_date: Option<String>,
    },
    Update {
        base_revision: i64,
        kind: MemoryKind,
        body: String,
        event_date: Option<String>,
    },
    Delete {
        base_revision: i64,
    },
}

fn parse_kind(value: &str) -> Result<MemoryKind, StorageError> {
    match value {
        "preference" => Ok(MemoryKind::Preference),
        "experience" => Ok(MemoryKind::Experience),
        "task_fact" => Ok(MemoryKind::TaskFact),
        // The column CHECK makes this unreachable unless the file is corrupt.
        _ => Err(StorageError::Unavailable),
    }
}

struct EventRecord<'a> {
    id: &'a str,
    op: MemorySyncOp,
    revision: i64,
    kind: Option<&'a str>,
    body: Option<&'a str>,
    event_date: Option<&'a str>,
}

fn record_event(
    tx: &Transaction<'_>,
    account: &str,
    now: i64,
    event: &EventRecord<'_>,
) -> Result<(), StorageError> {
    tx.execute(
        "INSERT INTO memory_events(account_id,memory_id,op,revision,kind,body,event_date,occurred_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            account,
            event.id,
            event.op.as_str(),
            event.revision,
            event.kind,
            event.body,
            event.event_date,
            now
        ],
    )?;
    Ok(())
}

/// Drops device rows whose session can no longer authenticate (revoked, or a
/// native-issuer token gone idle/expired). Called from every pull and write so
/// a departed device stops gating compaction without waiting for sign-out.
fn sweep_dead_devices(tx: &Transaction<'_>, account: &str, now: i64) -> Result<(), StorageError> {
    tx.execute(
        "DELETE FROM memory_devices WHERE account_id=?1 AND NOT EXISTS(
         SELECT 1 FROM account_sessions s JOIN accounts a ON a.id=s.account_id
         WHERE s.account_id=memory_devices.account_id AND s.session_id=memory_devices.device_id
         AND s.revoked=0 AND (a.issuer<>?2 OR EXISTS(SELECT 1 FROM native_tokens t
         WHERE t.account_id=s.account_id AND t.session_id=s.session_id AND t.expires_at>?3
         AND t.last_seen<=?3 AND t.last_seen>?3-?4)))",
        params![account, NATIVE_ISSUER, now, SESSION_LIVENESS_MS],
    )?;
    Ok(())
}

fn ensure_account_row(tx: &Transaction<'_>, account: &str) -> Result<(i64, i64), StorageError> {
    tx.execute(
        "INSERT OR IGNORE INTO memory_accounts(account_id) VALUES(?1)",
        [account],
    )?;
    Ok(tx.query_row(
        "SELECT generation,watermark FROM memory_accounts WHERE account_id=?1",
        [account],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

fn upsert_device(
    tx: &Transaction<'_>,
    account: &str,
    device: &str,
    generation: i64,
    applied_seq: i64,
    now: i64,
) -> Result<(), StorageError> {
    tx.execute(
        "INSERT INTO memory_devices(account_id,device_id,generation,applied_seq,last_seen)
         VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account_id,device_id) DO UPDATE SET
         generation=excluded.generation,
         applied_seq=MAX(memory_devices.applied_seq,excluded.applied_seq),
         last_seen=excluded.last_seen",
        params![account, device, generation, applied_seq, now],
    )?;
    Ok(())
}

fn load_snapshot(
    tx: &Transaction<'_>,
    account: &str,
) -> Result<Vec<MemorySnapshotItem>, StorageError> {
    let mut query = tx.prepare(
        "SELECT id,kind,body,event_date,revision,deleted FROM account_memories
         WHERE account_id=?1 ORDER BY created_at,id",
    )?;
    let rows = query.query_map([account], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (id, kind, body, event_date, revision, deleted) = row?;
        items.push(MemorySnapshotItem {
            id,
            kind: parse_kind(&kind)?,
            body,
            event_date,
            revision,
            deleted: deleted != 0,
        });
    }
    Ok(items)
}

/// Runs after every accepted write, inside its transaction: release dead
/// devices, drop events no live device still needs, then roll the generation
/// forward if the window overflowed.
fn compact_if_needed(tx: &Transaction<'_>, account: &str, now: i64) -> Result<(), StorageError> {
    sweep_dead_devices(tx, account, now)?;
    // Events live until every live device has applied them. With no device
    // registered yet they are kept: the writer itself learns from the push
    // receipt, but other peers still need the event — and a fresh pull by
    // anyone will gate future compaction correctly. The 512-event window
    // remains the hard backstop either way.
    let applied: Option<i64> = tx.query_row(
        "SELECT MIN(applied_seq) FROM memory_devices WHERE account_id=?1",
        [account],
        |r| r.get::<_, Option<i64>>(0),
    )?;
    if let Some(cutoff) = applied {
        tx.execute(
            "DELETE FROM memory_events WHERE account_id=?1 AND seq<=?2",
            params![account, cutoff],
        )?;
    }
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM memory_events WHERE account_id=?1",
        [account],
        |r| r.get(0),
    )?;
    if count > MEMORY_EVENT_WINDOW {
        let watermark: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM memory_events WHERE account_id=?1",
            [account],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE memory_accounts SET generation=generation+1,watermark=?1 WHERE account_id=?2",
            params![watermark, account],
        )?;
        tx.execute("DELETE FROM memory_events WHERE account_id=?1", [account])?;
    }
    Ok(())
}

fn apply_write(
    tx: &Transaction<'_>,
    account: &str,
    now: i64,
    id: &str,
    write: &MemoryWrite,
) -> Result<MemoryWriteReceipt, StorageError> {
    ensure_account_row(tx, account)?;
    let row: Option<(i64, bool)> = tx
        .query_row(
            "SELECT revision,deleted FROM account_memories WHERE account_id=?1 AND id=?2",
            params![account, id],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
        )
        .optional()?;
    match write {
        MemoryWrite::Create {
            kind,
            body,
            event_date,
        } => {
            let (body, event_date) = validate_synced_body(body, event_date.as_deref())?;
            match row {
                None => {
                    let active: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM account_memories WHERE account_id=?1 AND deleted=0",
                        [account],
                        |r| r.get(0),
                    )?;
                    if active >= MAX_MEMORIES as i64 {
                        return Err(MemorySyncError::Capacity.into());
                    }
                    tx.execute(
                        "INSERT INTO account_memories(account_id,id,kind,body,event_date,revision,created_at,updated_at)
                         VALUES(?1,?2,?3,?4,?5,1,?6,?6)",
                        params![account, id, kind.as_str(), body, event_date, now],
                    )?;
                    record_event(
                        tx,
                        account,
                        now,
                        &EventRecord {
                            id,
                            op: MemorySyncOp::Upsert,
                            revision: 1,
                            kind: Some(kind.as_str()),
                            body: Some(&body),
                            event_date: event_date.as_deref(),
                        },
                    )?;
                    Ok(MemoryWriteReceipt {
                        memory_id: id.into(),
                        revision: 1,
                        deleted: false,
                    })
                }
                // The tombstone invariant: a deleted id stays deleted.
                Some((_, true)) => Err(MemorySyncError::Resurrection.into()),
                Some((_, false)) => Err(MemorySyncError::RevisionConflict.into()),
            }
        }
        MemoryWrite::Update {
            base_revision,
            kind,
            body,
            event_date,
        } => {
            let (body, event_date) = validate_synced_body(body, event_date.as_deref())?;
            match row {
                Some((revision, false)) if revision == *base_revision => {
                    tx.execute(
                        "UPDATE account_memories SET kind=?1,body=?2,event_date=?3,revision=revision+1,updated_at=?4
                         WHERE account_id=?5 AND id=?6 AND deleted=0 AND revision=?7",
                        params![kind.as_str(), body, event_date, now, account, id, revision],
                    )?;
                    record_event(
                        tx,
                        account,
                        now,
                        &EventRecord {
                            id,
                            op: MemorySyncOp::Upsert,
                            revision: revision + 1,
                            kind: Some(kind.as_str()),
                            body: Some(&body),
                            event_date: event_date.as_deref(),
                        },
                    )?;
                    Ok(MemoryWriteReceipt {
                        memory_id: id.into(),
                        revision: revision + 1,
                        deleted: false,
                    })
                }
                Some((_, true)) => Err(MemorySyncError::Resurrection.into()),
                Some((_, false)) => Err(MemorySyncError::RevisionConflict.into()),
                None => Err(StorageError::NotFound),
            }
        }
        MemoryWrite::Delete { base_revision } => match row {
            Some((revision, false)) if revision == *base_revision => {
                tx.execute(
                    "UPDATE account_memories SET deleted=1,body=NULL,event_date=NULL,deleted_at=?1,revision=revision+1,updated_at=?1
                     WHERE account_id=?2 AND id=?3 AND deleted=0 AND revision=?4",
                    params![now, account, id, revision],
                )?;
                record_event(
                    tx,
                    account,
                    now,
                    &EventRecord {
                        id,
                        op: MemorySyncOp::Delete,
                        revision: revision + 1,
                        kind: None,
                        body: None,
                        event_date: None,
                    },
                )?;
                Ok(MemoryWriteReceipt {
                    memory_id: id.into(),
                    revision: revision + 1,
                    deleted: true,
                })
            }
            // The exact same delete replayed after a lost response: the base
            // revision plus one is this delete's own tombstone revision.
            Some((revision, true)) if revision == base_revision + 1 => Ok(MemoryWriteReceipt {
                memory_id: id.into(),
                revision,
                deleted: true,
            }),
            Some((_, _)) => Err(MemorySyncError::RevisionConflict.into()),
            None => Err(StorageError::NotFound),
        },
    }
}

impl AccountStore {
    /// Active entries only, for the mobile peer list.
    pub fn list_memories(
        &self,
        identity: &VerifiedIdentity,
    ) -> Result<Vec<MemorySnapshotItem>, StorageError> {
        self.access(identity, |tx, profile, _| {
            let mut items = load_snapshot(tx, &profile.account_id)?;
            items.retain(|item| !item.deleted);
            Ok(items)
        })
    }

    /// Desktop pull. Degrades itself to a snapshot when the caller's
    /// generation or cursor fell behind the retention window.
    pub fn pull_memories(
        &self,
        identity: &VerifiedIdentity,
        generation: i64,
        cursor: i64,
    ) -> Result<MemorySyncResponse, StorageError> {
        self.access(identity, |tx, profile, now| {
            let account = &profile.account_id;
            let now = now as i64;
            sweep_dead_devices(tx, account, now)?;
            let (server_generation, watermark) = ensure_account_row(tx, account)?;
            let device = identity.session_id();
            if generation != server_generation || cursor < watermark {
                upsert_device(tx, account, device, server_generation, watermark, now)?;
                let pending: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM memory_events WHERE account_id=?1 AND seq>?2)",
                    params![account, watermark],
                    |r| r.get(0),
                )?;
                return Ok(MemorySyncResponse {
                    generation: server_generation,
                    cursor: watermark,
                    pending,
                    events: Vec::new(),
                    snapshot: Some(load_snapshot(tx, account)?),
                });
            }
            let mut query = tx.prepare(
                "SELECT seq,memory_id,op,revision,kind,body,event_date,occurred_at
                 FROM memory_events WHERE account_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3",
            )?;
            let rows = query.query_map(
                params![account, cursor, MEMORY_PULL_LIMIT as i64 + 1],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, i64>(7)?,
                    ))
                },
            )?;
            let mut raw = Vec::new();
            for row in rows {
                raw.push(row?);
            }
            let pending = raw.len() > MEMORY_PULL_LIMIT;
            raw.truncate(MEMORY_PULL_LIMIT);
            let new_cursor = raw.last().map(|row| row.0).unwrap_or(cursor);
            upsert_device(tx, account, device, server_generation, new_cursor, now)?;
            let events = raw
                .into_iter()
                .map(
                    |(seq, memory_id, op, revision, kind, body, event_date, occurred_at)| {
                        Ok(MemorySyncEvent {
                            seq,
                            memory_id,
                            op: MemorySyncOp::parse(&op).ok_or(StorageError::Unavailable)?,
                            revision,
                            kind: kind.as_deref().map(parse_kind).transpose()?,
                            body,
                            event_date,
                            occurred_at,
                        })
                    },
                )
                .collect::<Result<Vec<_>, StorageError>>()?;
            Ok(MemorySyncResponse {
                generation: server_generation,
                cursor: new_cursor,
                pending,
                events,
                snapshot: None,
            })
        })
    }

    /// Desktop push and mobile update/delete share this path: journal replay
    /// by request id, then the same revision-conflict rules.
    pub fn write_memory(
        &self,
        identity: &VerifiedIdentity,
        request_id: &str,
        id: &str,
        write: &MemoryWrite,
    ) -> Result<MemoryWriteReceipt, StorageError> {
        let request_id = uuid::Uuid::parse_str(request_id)
            .map_err(|_| IdentityError::InvalidRequest)?
            .to_string();
        let id = uuid::Uuid::parse_str(id)
            .map_err(|_| IdentityError::InvalidRequest)?
            .to_string();
        self.access(identity, |tx, profile, now| {
            let account = &profile.account_id;
            let now = now as i64;
            // The fingerprint binds a request id to one payload; a reused id
            // with different content is a client bug, not a retry.
            let request_key = format!("{id}|{}", serde_json::to_string(write)?);
            let prior: Option<(String, String)> = tx
                .query_row(
                    "SELECT request_key,response FROM memory_requests WHERE account_id=?1 AND request_id=?2",
                    params![account, request_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((key, response)) = prior {
                if key != request_key {
                    return Err(IdentityError::Conflict.into());
                }
                return Ok(serde_json::from_str(&response)?);
            }
            let receipt = apply_write(tx, account, now, &id, write)?;
            tx.execute(
                "INSERT INTO memory_requests(account_id,request_id,request_key,response,created_at)
                 VALUES(?1,?2,?3,?4,?5)",
                params![
                    account,
                    request_id,
                    request_key,
                    serde_json::to_string(&receipt)?,
                    now
                ],
            )?;
            tx.execute(
                "DELETE FROM memory_requests WHERE account_id=?1 AND rowid IN(
                 SELECT rowid FROM memory_requests WHERE account_id=?1
                 ORDER BY rowid DESC LIMIT -1 OFFSET ?2)",
                params![account, MEMORY_REQUEST_JOURNAL],
            )?;
            compact_if_needed(tx, account, now)?;
            Ok(receipt)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(issuer: &str, subject: &str, session: &str, login: u64) -> VerifiedIdentity {
        VerifiedIdentity::from_verified_provider(issuer, subject, session, login, 10_000).unwrap()
    }
    fn fixture() -> AccountStore {
        let mut store =
            AccountStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store.clock = || 1000;
        store
    }
    fn create(
        store: &AccountStore,
        who: &VerifiedIdentity,
        id: &str,
        body: &str,
    ) -> Result<MemoryWriteReceipt, StorageError> {
        store.write_memory(
            who,
            &uuid::Uuid::new_v4().to_string(),
            id,
            &MemoryWrite::Create {
                kind: MemoryKind::Preference,
                body: body.into(),
                event_date: Some("2026-01-02".into()),
            },
        )
    }
    fn update(
        store: &AccountStore,
        who: &VerifiedIdentity,
        id: &str,
        base: i64,
        body: &str,
    ) -> Result<MemoryWriteReceipt, StorageError> {
        store.write_memory(
            who,
            &uuid::Uuid::new_v4().to_string(),
            id,
            &MemoryWrite::Update {
                base_revision: base,
                kind: MemoryKind::Preference,
                body: body.into(),
                event_date: None,
            },
        )
    }
    fn delete(
        store: &AccountStore,
        who: &VerifiedIdentity,
        id: &str,
        base: i64,
    ) -> Result<MemoryWriteReceipt, StorageError> {
        store.write_memory(
            who,
            &uuid::Uuid::new_v4().to_string(),
            id,
            &MemoryWrite::Delete {
                base_revision: base,
            },
        )
    }
    fn sync_conflict<T>(result: Result<T, StorageError>) -> bool {
        matches!(
            result,
            Err(StorageError::MemorySync(MemorySyncError::RevisionConflict))
        )
    }
    fn events_of(store: &AccountStore, who: &VerifiedIdentity) -> (String, i64) {
        let profile = store.profile(who).unwrap();
        let c = store.connection.lock().unwrap();
        let count: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM memory_events WHERE account_id=?1",
                [&profile.account_id],
                |r| r.get(0),
            )
            .unwrap();
        (profile.account_id, count)
    }

    #[test]
    fn first_pull_serves_a_snapshot_then_events_in_order() {
        let store = fixture();
        let desktop = identity("provider", "alice", "session-desktop", 100);
        let phone = identity("provider", "alice", "session-phone", 100);
        let m1 = uuid::Uuid::new_v4().to_string();
        create(&store, &desktop, &m1, "prefer tea").unwrap();
        // A brand-new device starts at generation 0 and gets the snapshot.
        let first = store.pull_memories(&desktop, 0, 0).unwrap();
        assert_eq!(first.generation, 1);
        let snapshot = first.snapshot.expect("snapshot on first pull");
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, m1);
        assert_eq!(snapshot[0].body.as_deref(), Some("prefer tea"));
        assert!(!snapshot[0].deleted);
        // Writes from another session surface as ordered events.
        assert_eq!(
            update(&store, &phone, &m1, 1, "prefer coffee")
                .unwrap()
                .revision,
            2
        );
        assert!(delete(&store, &phone, &m1, 2).unwrap().deleted);
        let mut cursor = first.cursor;
        let mut generation = first.generation;
        let events = loop {
            let page = store.pull_memories(&desktop, generation, cursor).unwrap();
            assert!(page.snapshot.is_none());
            cursor = page.cursor;
            generation = page.generation;
            if !page.pending {
                break page.events;
            }
            assert!(!page.events.is_empty());
        };
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].op, MemorySyncOp::Upsert);
        assert_eq!(events[0].body.as_deref(), Some("prefer tea"));
        assert_eq!(events[0].revision, 1);
        assert_eq!(events[1].op, MemorySyncOp::Upsert);
        assert_eq!(events[1].body.as_deref(), Some("prefer coffee"));
        assert_eq!(events[1].revision, 2);
        assert_eq!(events[2].op, MemorySyncOp::Delete);
        assert_eq!(events[2].revision, 3);
        // The mobile list hides tombstones.
        assert!(store.list_memories(&phone).unwrap().is_empty());
        // No new write has run compaction since the desktop applied, so all
        // three events (create, update, delete) are still retained.
        assert_eq!(events_of(&store, &desktop).1, 3);
    }

    #[test]
    fn conflict_matrix_rejects_stale_duplicate_and_resurrecting_writes() {
        let store = fixture();
        let alice = identity("provider", "alice", "session-a", 100);
        let m1 = uuid::Uuid::new_v4().to_string();
        create(&store, &alice, &m1, "prefer tea").unwrap();
        assert!(sync_conflict(update(&store, &alice, &m1, 0, "stale")));
        assert!(sync_conflict(update(&store, &alice, &m1, 5, "future")));
        assert!(sync_conflict(create(&store, &alice, &m1, "duplicate")));
        assert!(matches!(
            update(
                &store,
                &alice,
                "00000000-0000-0000-0000-000000000000",
                0,
                "ghost"
            ),
            Err(StorageError::NotFound)
        ));
        delete(&store, &alice, &m1, 1).unwrap();
        assert!(matches!(
            create(&store, &alice, &m1, "resurrect"),
            Err(StorageError::MemorySync(MemorySyncError::Resurrection))
        ));
        assert!(matches!(
            update(&store, &alice, &m1, 2, "edit a tombstone"),
            Err(StorageError::MemorySync(MemorySyncError::Resurrection))
        ));
        // The same delete replayed after a lost response stays idempotent…
        assert_eq!(delete(&store, &alice, &m1, 1).unwrap().revision, 2);
        // …but a delete from an older base is a conflict.
        assert!(sync_conflict(delete(&store, &alice, &m1, 0)));
    }

    #[test]
    fn capacity_counts_active_rows_and_tombstones_stay_dead() {
        let store = fixture();
        let alice = identity("provider", "alice", "session-a", 100);
        let mut ids = Vec::new();
        for i in 0..MAX_MEMORIES {
            let id = uuid::Uuid::new_v4().to_string();
            create(&store, &alice, &id, &format!("memory {i}")).unwrap();
            ids.push(id);
        }
        assert!(matches!(
            create(
                &store,
                &alice,
                &uuid::Uuid::new_v4().to_string(),
                "one too many"
            ),
            Err(StorageError::MemorySync(MemorySyncError::Capacity))
        ));
        // Deleting frees a slot for a NEW id…
        delete(&store, &alice, &ids[0], 1).unwrap();
        create(
            &store,
            &alice,
            &uuid::Uuid::new_v4().to_string(),
            "replacements allowed",
        )
        .unwrap();
        // …but the deleted id itself cannot come back.
        assert!(matches!(
            create(&store, &alice, &ids[0], "zombie"),
            Err(StorageError::MemorySync(MemorySyncError::Resurrection))
        ));
    }

    #[test]
    fn request_journal_replays_receipts_and_binds_the_payload() {
        let store = fixture();
        let alice = identity("provider", "alice", "session-a", 100);
        let m1 = uuid::Uuid::new_v4().to_string();
        let request_id = uuid::Uuid::new_v4().to_string();
        let write = MemoryWrite::Create {
            kind: MemoryKind::Preference,
            body: "prefer tea".into(),
            event_date: None,
        };
        let first = store
            .write_memory(&alice, &request_id, &m1, &write)
            .unwrap();
        let replay = store
            .write_memory(&alice, &request_id, &m1, &write)
            .unwrap();
        assert_eq!(first, replay);
        // Same request id carrying a different payload is a client bug.
        let mutated = MemoryWrite::Create {
            kind: MemoryKind::Experience,
            body: "different".into(),
            event_date: None,
        };
        assert!(matches!(
            store.write_memory(&alice, &request_id, &m1, &mutated),
            Err(StorageError::Identity(IdentityError::Conflict))
        ));
        // Malformed ids never reach the journal.
        assert!(matches!(
            store.write_memory(&alice, &request_id, "not-a-uuid", &write),
            Err(StorageError::Identity(IdentityError::InvalidRequest))
        ));
    }

    #[test]
    fn request_journal_is_bounded_per_account() {
        let store = fixture();
        let alice = identity("provider", "alice", "session-a", 100);
        let m1 = uuid::Uuid::new_v4().to_string();
        // Only accepted writes enter the journal, so keep the base in step.
        let mut revision = 1;
        create(&store, &alice, &m1, "prefer tea").unwrap();
        for _ in 0..(MEMORY_REQUEST_JOURNAL + 10) {
            revision += 1;
            update(&store, &alice, &m1, revision - 1, "step").unwrap();
        }
        let c = store.connection.lock().unwrap();
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM memory_requests", [], |r| r.get(0))
            .unwrap();
        assert!(count <= MEMORY_REQUEST_JOURNAL, "journal grew to {count}");
    }

    #[test]
    fn compaction_waits_for_lagging_devices_and_unblocks_on_catchup() {
        let store = fixture();
        let alice = identity("provider", "alice", "session-a", 100);
        let slow = identity("provider", "alice", "session-slow", 100);
        let m1 = uuid::Uuid::new_v4().to_string();
        create(&store, &alice, &m1, "prefer tea").unwrap();
        // The slow device pulls once, then misses two updates.
        let anchored = store.pull_memories(&slow, 0, 0).unwrap();
        update(&store, &alice, &m1, 1, "v2").unwrap();
        update(&store, &alice, &m1, 2, "v3").unwrap();
        assert_eq!(
            events_of(&store, &alice).1,
            3,
            "lagging device retains events"
        );
        // Catching up, then one more write, releases the retained events…
        let mut cursor = anchored.cursor;
        let mut generation = anchored.generation;
        loop {
            let page = store.pull_memories(&slow, generation, cursor).unwrap();
            cursor = page.cursor;
            generation = page.generation;
            if !page.pending {
                break;
            }
        }
        update(&store, &alice, &m1, 3, "v4").unwrap();
        let (account, count) = events_of(&store, &alice);
        // Only the newest event survives: applied-by-all history is gone.
        assert_eq!(count, 1, "fully applied events should be compacted");
        // …while the mirror row itself survives compaction.
        let c = store.connection.lock().unwrap();
        let rows: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM account_memories WHERE account_id=?1",
                [&account],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn window_overflow_rolls_generation_and_serves_snapshots_to_laggards() {
        let store = fixture();
        let alice = identity("provider", "alice", "session-a", 100);
        let m1 = uuid::Uuid::new_v4().to_string();
        create(&store, &alice, &m1, "prefer tea").unwrap();
        // Keep every write accepted so each one records an event.
        let mut revision = 1;
        for i in 1..=(MEMORY_EVENT_WINDOW + 1) {
            revision += 1;
            update(&store, &alice, &m1, revision - 1, &format!("v{i}")).unwrap();
        }
        {
            // Take the profile before the connection guard: profile() locks
            // the same non-reentrant mutex internally.
            let profile = store.profile(&alice).unwrap();
            let c = store.connection.lock().unwrap();
            let (generation, watermark): (i64, i64) = c
                .query_row(
                    "SELECT generation,watermark FROM memory_accounts WHERE account_id=?1",
                    [&profile.account_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert!(generation >= 2, "generation must roll past {generation}");
            let events: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM memory_events WHERE account_id=?1",
                    [&profile.account_id],
                    |r| r.get(0),
                )
                .unwrap();
            // The roll fires during the loop (event 513 exceeds the window),
            // so exactly one post-roll event from the final iteration remains.
            assert_eq!(events, 1, "overflow must clear the account's events");
            assert!(watermark > 0);
        }
        // A device that never pulled gets the snapshot under the new generation.
        let late = identity("provider", "alice", "session-late", 100);
        let page = store.pull_memories(&late, 0, 0).unwrap();
        let snapshot = page.snapshot.expect("snapshot after generation roll");
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, m1);
        assert!(!snapshot[0].deleted);
        // A stale cursor from before the watermark is degraded to a snapshot.
        let stale = store.pull_memories(&late, page.generation, 0).unwrap();
        assert!(stale.snapshot.is_some());
    }

    #[test]
    fn departed_sessions_stop_gating_compaction() {
        let mut store = fixture();
        let alice = identity("provider", "alice", "session-old", 100);
        let m1 = uuid::Uuid::new_v4().to_string();
        create(&store, &alice, &m1, "prefer tea").unwrap();
        // The old device anchors its cursor…
        store.pull_memories(&alice, 0, 0).unwrap();
        update(&store, &alice, &m1, 1, "v2").unwrap();
        assert_eq!(events_of(&store, &alice).1, 2, "anchored events wait");
        // …revoking that session releases its device row on the next write.
        store.sign_out(&alice, false).unwrap();
        let refreshed = identity("provider", "alice", "session-new", 100);
        update(&store, &refreshed, &m1, 2, "v3").unwrap();
        let (account, count) = events_of(&store, &refreshed);
        {
            let c = store.connection.lock().unwrap();
            let devices: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM memory_devices WHERE account_id=?1",
                    [&account],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(devices, 0, "revoked session must lose its device row");
        }
        // With no live device the events are kept for the next peer's pull.
        assert_eq!(count, 3, "no live device still preserves unread events");
        // An idle native-issuer token is swept the same way on pull.
        let native = identity(NATIVE_ISSUER, "native@example", "session-native", 100);
        let m2 = uuid::Uuid::new_v4().to_string();
        create(&store, &native, &m2, "native memory").unwrap();
        store.pull_memories(&native, 0, 0).unwrap();
        // Capture the profile while the original identity is still valid —
        // the clock advances below and would expire it.
        let native_profile = store.profile(&native).unwrap();
        {
            let c = store.connection.lock().unwrap();
            c.execute(
                "INSERT INTO native_tokens(hash,account_id,session_id,expires_at,last_seen)
                 VALUES(x'0102',?1,'session-native',9999999,1)",
                [&native_profile.account_id],
            )
            .unwrap();
        }
        store.clock = || 1000 + SESSION_LIVENESS_MS as u64 + 1;
        let next = VerifiedIdentity::from_verified_provider(
            NATIVE_ISSUER,
            "native@example",
            "session-native2",
            1000,
            9_999_999,
        )
        .unwrap();
        store.pull_memories(&next, 0, 0).unwrap();
        {
            let c = store.connection.lock().unwrap();
            let devices: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM memory_devices WHERE account_id=?1 AND device_id='session-native'",
                    [&native_profile.account_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(devices, 0, "idle native token must lose its device row");
        }
    }

    #[test]
    fn accounts_never_see_each_others_memories_or_events() {
        let store = fixture();
        let alice = identity("provider", "alice", "session-a", 100);
        let bob = identity("provider", "bob", "session-b", 100);
        let m1 = uuid::Uuid::new_v4().to_string();
        create(&store, &alice, &m1, "alice only").unwrap();
        assert!(store.list_memories(&bob).unwrap().is_empty());
        let bob_pull = store.pull_memories(&bob, 0, 0).unwrap();
        let snapshot = bob_pull.snapshot.expect("fresh device snapshot");
        assert!(snapshot.is_empty());
        // The journal is namespaced per account: the same request id under
        // another account is simply a different entry.
        let request_id = uuid::Uuid::new_v4().to_string();
        let m2 = uuid::Uuid::new_v4().to_string();
        let write = MemoryWrite::Create {
            kind: MemoryKind::Preference,
            body: "bob writes".into(),
            event_date: None,
        };
        let bob_receipt = store.write_memory(&bob, &request_id, &m2, &write).unwrap();
        let bob_replay = store.write_memory(&bob, &request_id, &m2, &write).unwrap();
        assert_eq!(bob_receipt, bob_replay);
        assert_eq!(
            store
                .write_memory(
                    &alice,
                    &request_id,
                    &uuid::Uuid::new_v4().to_string(),
                    &write
                )
                .unwrap()
                .revision,
            1
        );
    }
}

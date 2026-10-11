//! Desktop side of the cross-device memory sync (schema v5): the outbox that
//! enqueues in the same transaction as every local mutation, the per-owner
//! server mirror, and the translation of remote events/snapshots back through
//! the local domain operations. Local and server revisions are deliberately
//! never compared — the mirror is the only rendezvous.
use crate::history::HistoryStore;
use crate::memory::{
    advance, epoch, read_active, remove_selections, tombstone_and_prune, validate_schema,
};
use crate::StorageError;
use companion_core::memory::*;
use companion_core::now_ms;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::collections::BTreeSet;

/// One pending local change, resolved by the host sync loop against the
/// current owner's mirror (create: base 0; update: mirror value or degrade to
/// create for an owner whose server has never seen the entry; delete: mirror
/// value, or nothing to push at all).
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxEntry {
    pub request_id: String,
    pub op: OutboxOp,
    pub memory_id: String,
    pub kind: Option<MemoryKind>,
    pub body: Option<String>,
    pub event_date: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxOp {
    Create,
    Update,
    Delete,
}
impl OutboxOp {
    fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        match value {
            "create" => Some(Self::Create),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

/// What the host needs to know after applying a pull: whether to emit
/// memory-changed, how many turns were pruned, and how many entries were
/// skipped because the local domain refuses their kind or the local capacity
/// is full (both still record the mirror so the stream keeps moving).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RemoteApplyReport {
    pub changed: bool,
    pub cleared_turns: u64,
    pub skipped_kind: usize,
    pub skipped_capacity: usize,
}

fn now() -> Result<i64, StorageError> {
    i64::try_from(now_ms())
        .ok()
        .filter(|v| *v <= MAX_COUNTER)
        .ok_or(StorageError::InvalidTimestamp)
}

/// Enqueue a local mutation inside its own transaction. At most one pending
/// row exists per memory id: successive updates fold onto it, and a delete
/// absorbs it. Folding always rewrites the request id so an in-flight push of
/// the old payload cannot ack away the folded-in newer change.
pub(crate) fn enqueue(
    tx: &Connection,
    op: OutboxOp,
    memory_id: &str,
    kind: Option<&MemoryKind>,
    body: Option<&str>,
    event_date: Option<&str>,
) -> Result<(), StorageError> {
    let pending: Option<String> = tx
        .query_row(
            "SELECT op FROM memory_outbox WHERE memory_id=?1",
            [memory_id],
            |r| r.get(0),
        )
        .optional()?;
    let fresh_id = uuid::Uuid::new_v4().to_string();
    match (pending.as_deref(), op) {
        (None, _) => {
            tx.execute(
                "INSERT INTO memory_outbox(request_id,op,memory_id,kind,body,event_date,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    fresh_id,
                    op.as_str(),
                    memory_id,
                    kind.map(|k| k.as_str()),
                    body,
                    event_date,
                    now()?
                ],
            )?;
        }
        // create+update folds into the create; update+update keeps the oldest
        // base semantics by staying one row with a fresh id.
        (Some("create") | Some("update"), OutboxOp::Update) => {
            tx.execute(
                "UPDATE memory_outbox SET request_id=?1,kind=?2,body=?3,event_date=?4,created_at=?5
                 WHERE memory_id=?6",
                params![
                    fresh_id,
                    kind.map(|k| k.as_str()),
                    body,
                    event_date,
                    now()?,
                    memory_id
                ],
            )?;
        }
        // create+delete and update+delete both keep a DELETE row rather than
        // vanishing: if the earlier push is already in flight, the server
        // entry exists and only this delete row can retire it. When no mirror
        // ever forms, the push phase resolves it to a local net zero.
        (Some("create") | Some("update"), OutboxOp::Delete) => {
            tx.execute(
                "UPDATE memory_outbox SET request_id=?1,op='delete',kind=NULL,body=NULL,event_date=NULL,created_at=?2
                 WHERE memory_id=?3",
                params![fresh_id, now()?, memory_id],
            )?;
        }
        // A deleted id cannot change again locally (resurrection is
        // server-rejected); reaching here means a stale redundant enqueue.
        _ => {}
    }
    Ok(())
}

fn outbox_pending(tx: &Connection, memory_id: &str) -> Result<bool, StorageError> {
    Ok(tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_outbox WHERE memory_id=?1)",
            [memory_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

fn parse_kind(value: &str) -> Option<MemoryKind> {
    match value {
        "preference" => Some(MemoryKind::Preference),
        "experience" => Some(MemoryKind::Experience),
        "task_fact" => Some(MemoryKind::TaskFact),
        _ => None,
    }
}

fn mirror_revision(
    tx: &Connection,
    owner: &str,
    memory_id: &str,
) -> Result<Option<i64>, StorageError> {
    Ok(tx
        .query_row(
            "SELECT server_revision FROM memory_server_mirror WHERE owner=?1 AND memory_id=?2",
            params![owner, memory_id],
            |r| r.get(0),
        )
        .optional()?)
}

fn set_mirror(
    tx: &Connection,
    owner: &str,
    memory_id: &str,
    revision: i64,
) -> Result<(), StorageError> {
    tx.execute(
        "INSERT INTO memory_server_mirror(owner,memory_id,server_revision) VALUES(?1,?2,?3)
         ON CONFLICT(owner,memory_id) DO UPDATE SET server_revision=MAX(memory_server_mirror.server_revision,excluded.server_revision)",
        params![owner, memory_id, revision],
    )?;
    Ok(())
}

/// Resurrect a tombstone to the server's active wording. Local chats quoting
/// the deleted wording stay deleted (honest boundary: the conflict-abandon
/// cleanup is irreversible); the epoch still advances so in-flight completions
/// die. Returns false when the row is not a tombstone.
fn resurrect(
    tx: &Connection,
    id: &str,
    kind: MemoryKind,
    body: &str,
    event_date: Option<&str>,
) -> Result<bool, StorageError> {
    let revision: Option<i64> = tx
        .query_row(
            "SELECT revision FROM memories WHERE id=?1 AND deleted_at IS NOT NULL",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(revision) = revision else {
        return Ok(false);
    };
    let revision = next_counter(revision)?;
    let now = now()?;
    advance(tx, epoch(tx)?)?;
    tx.execute(
        "UPDATE memories SET kind=?1,body=?2,source_kind='user_manual',source_label=?3,event_date=?4,
         created_at=?5,confirmed_at=?5,updated_at=?5,revision=?6,deleted_at=NULL WHERE id=?7",
        params![kind.as_str(), body, MANUAL_SOURCE_LABEL, event_date, now, revision, id],
    )?;
    validate_schema(tx)?;
    Ok(true)
}

/// Apply one remote upsert to a locally active row: plain domain update with
/// the same precise prune as a user edit. A selection that the new wording
/// would overflow is dropped inside the same transaction rather than failing
/// the whole pull — a poisoned event loop would be worse than a visible
/// deselection.
fn apply_remote_update(
    tx: &Connection,
    id: &str,
    kind: MemoryKind,
    body: &str,
    event_date: Option<&str>,
) -> Result<u64, StorageError> {
    let attempt = |tx: &Connection| -> Result<u64, StorageError> {
        let revision: i64 = tx.query_row(
            "SELECT revision FROM memories WHERE id=?1 AND deleted_at IS NULL",
            [id],
            |r| r.get(0),
        )?;
        let revision = next_counter(revision)?;
        advance(tx, epoch(tx)?)?;
        let now = now()?;
        tx.execute(
            "UPDATE memories SET kind=?1,body=?2,event_date=?3,updated_at=?4,confirmed_at=?4,revision=?5 WHERE id=?6 AND deleted_at IS NULL",
            params![kind.as_str(), body, event_date, now, revision, id],
        )?;
        validate_schema(tx)?;
        prune_chats_pub(tx, &[id.to_string()])
    };
    match attempt(tx) {
        Ok(cleared) => Ok(cleared),
        Err(_) => {
            remove_selections(tx, Some(id))?;
            attempt(tx)
        }
    }
}

fn prune_chats_pub(tx: &Connection, ids: &[String]) -> Result<u64, StorageError> {
    crate::memory::prune_chats(tx, ids, &[], &BTreeSet::new())
}

/// One remote entry (event or snapshot item) in translation-ready form.
struct RemoteEntry<'a> {
    id: &'a str,
    kind: Option<MemoryKind>,
    body: Option<&'a str>,
    event_date: Option<&'a str>,
    server_revision: i64,
    deleted: bool,
}

/// Shared translation of one remote entry (event or snapshot item). The
/// pending check runs first: an in-flight local op will win over the server
/// wording, so only the mirror moves.
fn apply_remote_entry(
    tx: &Connection,
    owner: &str,
    entry: &RemoteEntry<'_>,
    report: &mut RemoteApplyReport,
) -> Result<(), StorageError> {
    let id = entry.id;
    let pending = outbox_pending(tx, id)?;
    let local: Option<(i64, bool)> = tx
        .query_row(
            "SELECT revision,deleted_at IS NOT NULL FROM memories WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if entry.deleted {
        if let (Some((_, false)), false) = (local, pending) {
            // A task fact's "delete" from the server is the replacement or
            // eviction half of a redo — a system action that must NOT prune
            // the chats quoting the old wording (that gate belongs to user
            // deletes). Manual memories keep the precise prune.
            let is_fact: bool = tx.query_row(
                "SELECT kind='task_fact' FROM memories WHERE id=?1 AND deleted_at IS NULL",
                [id],
                |r| r.get(0),
            )?;
            if is_fact {
                crate::task_facts::retire_fact(tx, id)?;
                advance(tx, epoch(tx)?)?;
                report.changed = true;
            } else {
                let cleared = tombstone_and_prune(tx, id)?;
                advance(tx, epoch(tx)?)?;
                report.changed = true;
                report.cleared_turns += cleared;
            }
        }
    } else {
        let (Some(kind), Some(body)) = (entry.kind, entry.body) else {
            return Err(StorageError::Unavailable);
        };
        match local {
            None => {
                // Kind-split materialization with kind-split capacity: a full
                // local side skips the insert but still records the mirror so
                // this pull's cursor advances and the loop never wedges — the
                // entry catches up on a later snapshot once room exists.
                // Older binaries (pre-task-fact) keep their mirror-only skip;
                // this build materializes executor facts as read-only rows.
                let (manual, facts) = crate::memory::active_counts(tx)?;
                let full = if kind == MemoryKind::TaskFact {
                    facts >= MAX_TASK_FACTS
                } else {
                    manual >= MAX_MEMORIES
                };
                if full {
                    report.skipped_capacity += 1;
                } else if kind == MemoryKind::TaskFact {
                    let body_text: String = body.into();
                    if body_text.trim().is_empty()
                        || body_text.chars().count() > MAX_BODY_CHARS
                        || body_text
                            .chars()
                            .any(|c| c.is_control() && c != '\n' && c != '\t')
                    {
                        return Err(StorageError::Unavailable);
                    }
                    let now = now()?;
                    advance(tx, epoch(tx)?)?;
                    tx.execute(
                        "INSERT INTO memories(id,kind,body,source_kind,source_label,event_date,created_at,confirmed_at,updated_at,revision)
                         VALUES(?1,'task_fact',?2,'executor',?3,?4,?5,?5,?5,1)",
                        params![
                            id,
                            body_text,
                            companion_core::execution::TASK_FACT_SOURCE_LABEL,
                            entry.event_date,
                            now
                        ],
                    )?;
                    validate_schema(tx)?;
                    report.changed = true;
                } else {
                    let draft = MemoryDraft {
                        kind,
                        body: body.into(),
                        event_date: entry.event_date.map(Into::into),
                    }
                    .validate()?;
                    let now = now()?;
                    advance(tx, epoch(tx)?)?;
                    tx.execute(
                        "INSERT INTO memories(id,kind,body,source_kind,source_label,event_date,created_at,confirmed_at,updated_at,revision)
                         VALUES(?1,?2,?3,'user_manual',?4,?5,?6,?6,?6,1)",
                        params![id, draft.kind.as_str(), draft.body, MANUAL_SOURCE_LABEL, draft.event_date, now],
                    )?;
                    validate_schema(tx)?;
                    report.changed = true;
                }
            }
            Some((_, false)) => {
                if !pending && kind != MemoryKind::TaskFact {
                    let cleared = apply_remote_update(tx, id, kind, body, entry.event_date)?;
                    report.changed = true;
                    report.cleared_turns += cleared;
                }
            }
            Some((_, true)) => {
                if !pending
                    && kind != MemoryKind::TaskFact
                    && resurrect(tx, id, kind, body, entry.event_date)?
                {
                    report.changed = true;
                }
            }
        }
    }
    set_mirror(tx, owner, id, entry.server_revision)
}

impl HistoryStore {
    /// Pending local changes in enqueue order (folding bounds this to one row
    /// per memory id).
    pub fn memory_outbox_peek(&self) -> Result<Vec<OutboxEntry>, StorageError> {
        let mut query = self.0.prepare(
            "SELECT request_id,op,memory_id,kind,body,event_date FROM memory_outbox ORDER BY seq",
        )?;
        let rows = query.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (request_id, op, memory_id, kind, body, event_date) = row?;
            entries.push(OutboxEntry {
                request_id,
                op: OutboxOp::parse(&op).ok_or(StorageError::Unavailable)?,
                memory_id,
                kind: kind.as_deref().and_then(parse_kind),
                body,
                event_date,
            });
        }
        Ok(entries)
    }

    /// Resolve the push base for the current owner; None means the owner's
    /// server has never seen the entry.
    pub fn memory_push_base(
        &self,
        owner: &str,
        memory_id: &str,
    ) -> Result<Option<i64>, StorageError> {
        mirror_revision(&self.0, owner, memory_id)
    }

    /// An accepted push: drop the row (no-op when folding already replaced
    /// it) and ALWAYS move the mirror to the receipt's revision — the server
    /// has accepted the change, so its revision is a fact regardless of
    /// whether this row still exists. Folding away the row must not strand
    /// that fact: the folded delete row pushes against the recorded mirror
    /// and retires the server entry; a folded update row rebases onto it
    /// (last-writer-wins per revision).
    pub fn memory_outbox_ack(
        &mut self,
        owner: &str,
        request_id: &str,
        memory_id: &str,
        server_revision: i64,
    ) -> Result<(), StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM memory_outbox WHERE request_id=?1",
            [request_id],
        )?;
        set_mirror(&tx, owner, memory_id, server_revision)?;
        tx.commit()?;
        Ok(())
    }

    /// A rejected push (409): drop the row for good, clear the entry's mirror
    /// row, and reset the owner's pull position so the next tick takes the
    /// snapshot path. The dropped edit may have been the only thing keeping a
    /// pending-skipped server wording unapplied — the event stream will not
    /// resend it, and a mirror equal to the snapshot's revision would skip
    /// the very wording that needs to land, so the cursor AND the mirror both
    /// go. The change itself is never rebased onto a newer base.
    pub fn memory_outbox_drop(
        &mut self,
        owner: &str,
        request_id: &str,
    ) -> Result<bool, StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let memory_id: Option<String> = tx
            .query_row(
                "SELECT memory_id FROM memory_outbox WHERE request_id=?1",
                [request_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = memory_id.as_deref() {
            tx.execute(
                "DELETE FROM memory_outbox WHERE request_id=?1",
                [request_id],
            )?;
            tx.execute(
                "DELETE FROM memory_server_mirror WHERE owner=?1 AND memory_id=?2",
                params![owner, id],
            )?;
            persist_cursor(&tx, owner, 0, 0)?;
        }
        tx.commit()?;
        Ok(memory_id.is_some())
    }

    /// The owner's persisted pull position; (0, 0) forces the snapshot path.
    pub fn memory_sync_state(&self, owner: &str) -> Result<(i64, i64), StorageError> {
        Ok(self
            .0
            .query_row(
                "SELECT generation,cursor FROM memory_sync_owners WHERE owner=?1",
                [owner],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0)))
    }

    /// Apply one pull's events and persist the new cursor atomically: either
    /// every event lands plus the cursor moves, or nothing does. Upserts
    /// whose revision equals the mirror are our own pushes echoing back, but
    /// a delete is never skipped that way: a snapshot may have recorded the
    /// tombstone revision while this store had no row to delete, and the
    /// replayed delete is exactly what converges such a row.
    pub fn memory_apply_remote(
        &mut self,
        owner: &str,
        events: &[MemorySyncEvent],
        generation: i64,
        cursor: i64,
    ) -> Result<RemoteApplyReport, StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut report = RemoteApplyReport::default();
        for event in events {
            // Upserts at or below the mirror are history: our own push
            // echoing back, or an older event replayed after a snapshot
            // already carried a newer wording — applying it would roll the
            // entry backwards. Deletes are never skipped this way.
            if event.op == MemorySyncOp::Upsert
                && mirror_revision(&tx, owner, &event.memory_id)?
                    .is_some_and(|mirrored| mirrored >= event.revision)
            {
                continue;
            }
            apply_remote_entry(
                &tx,
                owner,
                &RemoteEntry {
                    id: &event.memory_id,
                    kind: event.kind,
                    body: event.body.as_deref(),
                    event_date: event.event_date.as_deref(),
                    server_revision: event.revision,
                    deleted: event.op == MemorySyncOp::Delete,
                },
                &mut report,
            )?;
        }
        persist_cursor(&tx, owner, generation, cursor)?;
        tx.commit()?;
        Ok(report)
    }

    /// Apply a snapshot (generation rollover or first pull) and persist the
    /// fresh position. Local active entries absent from the snapshot are
    /// merged by enqueueing a create — the owner's server simply has not met
    /// them yet; local tombstones absent from it stay as they are.
    pub fn memory_apply_snapshot(
        &mut self,
        owner: &str,
        items: &[MemorySnapshotItem],
        generation: i64,
        cursor: i64,
    ) -> Result<RemoteApplyReport, StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut report = RemoteApplyReport::default();
        for item in items {
            // Same monotonic guard as events: a snapshot item at or below the
            // mirror carries nothing newer than what already landed.
            if mirror_revision(&tx, owner, &item.id)?
                .is_some_and(|mirrored| mirrored >= item.revision)
            {
                continue;
            }
            apply_remote_entry(
                &tx,
                owner,
                &RemoteEntry {
                    id: &item.id,
                    kind: Some(item.kind),
                    body: item.body.as_deref(),
                    event_date: item.event_date.as_deref(),
                    server_revision: item.revision,
                    deleted: item.deleted,
                },
                &mut report,
            )?;
        }
        for memory in read_active(&tx)? {
            if !items.iter().any(|item| item.id == memory.id) && !outbox_pending(&tx, &memory.id)? {
                enqueue(
                    &tx,
                    OutboxOp::Create,
                    &memory.id,
                    Some(&memory.kind),
                    Some(&memory.body),
                    memory.event_date.as_deref(),
                )?;
            }
        }
        persist_cursor(&tx, owner, generation, cursor)?;
        tx.commit()?;
        Ok(report)
    }
}

fn persist_cursor(
    tx: &Transaction<'_>,
    owner: &str,
    generation: i64,
    cursor: i64,
) -> Result<(), StorageError> {
    tx.execute(
        "INSERT INTO memory_sync_owners(owner,generation,cursor) VALUES(?1,?2,?3)
         ON CONFLICT(owner) DO UPDATE SET generation=excluded.generation,cursor=excluded.cursor",
        params![owner, generation, cursor],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use companion_core::memory::MemorySyncOp;

    fn store() -> HistoryStore {
        HistoryStore::open(
            &std::env::temp_dir().join(format!("qiban-outbox-{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap()
    }
    fn draft(body: &str) -> MemoryDraft {
        MemoryDraft {
            kind: MemoryKind::Preference,
            body: body.into(),
            event_date: Some("2026-01-02".into()),
        }
    }
    fn create(store: &mut HistoryStore) -> Memory {
        let epoch = store.context_epoch().unwrap();
        store
            .memory_create(&draft("prefer tea"), epoch)
            .unwrap()
            .value
    }
    fn rows(store: &HistoryStore) -> Vec<OutboxEntry> {
        store.memory_outbox_peek().unwrap()
    }
    fn row(store: &HistoryStore, id: &str) -> Option<OutboxEntry> {
        rows(store).into_iter().find(|e| e.memory_id == id)
    }
    fn event(
        seq: i64,
        id: &str,
        op: MemorySyncOp,
        revision: i64,
        body: Option<&str>,
    ) -> MemorySyncEvent {
        MemorySyncEvent {
            seq,
            memory_id: id.into(),
            op,
            revision,
            kind: body.map(|_| MemoryKind::Preference),
            body: body.map(Into::into),
            event_date: None,
            occurred_at: 1,
        }
    }
    fn local_revision(store: &HistoryStore, id: &str) -> Option<i64> {
        store.context_epoch().ok()?;
        let c = &store.0;
        c.query_row("SELECT revision FROM memories WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .unwrap()
    }

    #[test]
    fn mutators_enqueue_and_fold_into_one_row_per_memory() {
        let owner = "4318-fold";
        let mut s = store();
        let m = create(&mut s);
        let first = row(&s, &m.id).expect("create enqueues");
        assert_eq!(first.op, OutboxOp::Create);
        assert_eq!(first.body.as_deref(), Some("prefer tea"));
        // create + update folds into the create with a fresh request id.
        let epoch = s.context_epoch().unwrap();
        s.memory_update(&m.id, m.revision, epoch, &draft("prefer coffee"))
            .unwrap();
        let folded = row(&s, &m.id).expect("folded row");
        assert_eq!(folded.op, OutboxOp::Create);
        assert_eq!(folded.body.as_deref(), Some("prefer coffee"));
        assert_ne!(folded.request_id, first.request_id);
        // update + update keeps one row, newest wording.
        let epoch = s.context_epoch().unwrap();
        s.memory_update(
            &m.id,
            local_revision(&s, &m.id).unwrap(),
            epoch,
            &draft("espresso"),
        )
        .unwrap();
        assert_eq!(rows(&s).len(), 1);
        assert_eq!(row(&s, &m.id).unwrap().body.as_deref(), Some("espresso"));
        // Once the server has seen the entry, update + delete folds into a
        // delete row with a fresh id.
        let entry = row(&s, &m.id).unwrap();
        s.memory_outbox_ack(owner, &entry.request_id, &m.id, 1)
            .unwrap();
        assert!(row(&s, &m.id).is_none());
        let epoch = s.context_epoch().unwrap();
        s.memory_update(
            &m.id,
            local_revision(&s, &m.id).unwrap(),
            epoch,
            &draft("v3"),
        )
        .unwrap();
        let update_row = row(&s, &m.id).expect("update enqueues post-ack");
        assert_eq!(update_row.op, OutboxOp::Update);
        let epoch = s.context_epoch().unwrap();
        s.memory_delete(&m.id, local_revision(&s, &m.id).unwrap(), epoch)
            .unwrap();
        let deleted = row(&s, &m.id).expect("delete row");
        assert_eq!(deleted.op, OutboxOp::Delete);
        assert!(deleted.body.is_none());
        assert_ne!(deleted.request_id, update_row.request_id);
        // Repeating the committed delete enqueues nothing new.
        let epoch = s.context_epoch().unwrap();
        s.memory_delete(&m.id, deleted_next(&s, &m.id), epoch)
            .unwrap();
        assert_eq!(rows(&s).len(), 1);
    }

    fn deleted_next(store: &HistoryStore, id: &str) -> i64 {
        let c = &store.0;
        c.query_row("SELECT revision FROM memories WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn create_then_delete_keeps_a_delete_row_even_without_a_push() {
        let owner = "4318-net-zero";
        let mut s = store();
        let m = create(&mut s);
        let epoch = s.context_epoch().unwrap();
        s.memory_delete(&m.id, m.revision, epoch).unwrap();
        // The row stays as a delete: the create push may already be in
        // flight, and only this row can retire the server entry it created.
        let pending = row(&s, &m.id).expect("delete row survives");
        assert_eq!(pending.op, OutboxOp::Delete);
        assert!(pending.body.is_none());
        // No mirror ever formed for this owner: the push phase resolves the
        // row to a local net zero without a round trip.
        assert_eq!(s.memory_push_base(owner, &m.id).unwrap(), None);
        s.memory_outbox_ack(owner, &pending.request_id, &m.id, 0)
            .unwrap();
        assert!(row(&s, &m.id).is_none());
    }

    #[test]
    fn delete_all_enqueues_one_delete_per_active_memory() {
        let mut s = store();
        create(&mut s);
        create(&mut s);
        let epoch = s.context_epoch().unwrap();
        s.memory_delete_all(epoch).unwrap();
        assert_eq!(rows(&s).len(), 2);
        assert!(rows(&s).iter().all(|e| e.op == OutboxOp::Delete));
    }

    #[test]
    fn remote_events_materialize_update_delete_and_resurrect() {
        let owner = "4318-test-owner";
        let mut s = store();
        let m = create(&mut s);
        let id = m.id.clone();
        // Ack the local create first: while a push is pending, remote events
        // for that id defer to the local change (only the mirror moves).
        let entry = row(&s, &id).unwrap();
        s.memory_outbox_ack(owner, &entry.request_id, &m.id, 1)
            .unwrap();
        // A brand-new id from another device lands as a local row.
        let other = uuid::Uuid::new_v4().to_string();
        let report = s
            .memory_apply_remote(
                owner,
                &[
                    event(1, &other, MemorySyncOp::Upsert, 1, Some("remote note")),
                    event(2, &id, MemorySyncOp::Upsert, 2, Some("prefer coffee")),
                ],
                1,
                2,
            )
            .unwrap();
        assert!(report.changed);
        let all = list_active(&s);
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|m| m.id == other && m.body == "remote note"));
        assert!(all.iter().any(|m| m.id == id && m.body == "prefer coffee"));
        assert_eq!(s.memory_sync_state(owner).unwrap(), (1, 2));
        // Our own echo: the same revision is already in the mirror.
        let report = s
            .memory_apply_remote(
                owner,
                &[event(
                    2,
                    &id,
                    MemorySyncOp::Upsert,
                    2,
                    Some("prefer coffee"),
                )],
                1,
                2,
            )
            .unwrap();
        assert!(!report.changed);
        // A pending local edit defers to itself: only the mirror moves.
        let epoch = s.context_epoch().unwrap();
        s.memory_update(
            &id,
            local_revision(&s, &id).unwrap(),
            epoch,
            &draft("local edit"),
        )
        .unwrap();
        s.memory_apply_remote(
            owner,
            &[event(3, &id, MemorySyncOp::Upsert, 3, Some("server edit"))],
            1,
            3,
        )
        .unwrap();
        assert!(list_active(&s)
            .iter()
            .any(|m| m.id == id && m.body == "local edit"));
        // The local edit has been pushed (ack clears the pending row), so the
        // server's delete may now land instead of deferring to it.
        let pending = row(&s, &id).expect("local edit enqueued");
        s.memory_outbox_ack(owner, &pending.request_id, &id, 3)
            .unwrap();
        // Deleting remotely tombstones locally, then the server's newer
        // wording resurrects the tombstone (chats stay deleted).
        let report = s
            .memory_apply_remote(owner, &[event(4, &id, MemorySyncOp::Delete, 4, None)], 1, 4)
            .unwrap();
        assert!(report.changed);
        assert!(local_revision(&s, &id).is_some()); // tombstoned row persists
        assert!(list_active(&s).iter().all(|m| m.id != id));
        let report = s
            .memory_apply_remote(
                owner,
                &[event(5, &id, MemorySyncOp::Upsert, 5, Some("back again"))],
                1,
                5,
            )
            .unwrap();
        assert!(report.changed);
        assert!(list_active(&s)
            .iter()
            .any(|m| m.id == id && m.body == "back again"));
    }

    #[test]
    fn remote_task_facts_materialize_as_executor_rows_and_retire_without_prune() {
        let owner = "4318-task-fact";
        let mut s = store();
        let id = uuid::Uuid::new_v4().to_string();
        let report = s
            .memory_apply_remote(
                owner,
                &[MemorySyncEvent {
                    seq: 1,
                    memory_id: id.clone(),
                    op: MemorySyncOp::Upsert,
                    revision: 1,
                    kind: Some(MemoryKind::TaskFact),
                    body: Some("把《笔记》的摘录保存为草稿：已核验完成".into()),
                    event_date: None,
                    occurred_at: 1,
                }],
                1,
                1,
            )
            .unwrap();
        assert!(report.changed, "this build materializes executor facts");
        let active = list_active(&s);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].kind, MemoryKind::TaskFact);
        assert_eq!(active[0].source_kind, MemorySource::Executor);
        assert_eq!(
            s.memory_push_base(owner, &id).unwrap(),
            Some(1),
            "the mirror records the applied revision"
        );
        // A later remote delete of a fact is the replacement/eviction half —
        // retire WITHOUT the precise prune (no chat cleanup from a system
        // action). There are no turns here; the prune distinction is pinned
        // by the task_facts suite; here we pin the retire itself.
        let report = s
            .memory_apply_remote(
                owner,
                &[MemorySyncEvent {
                    seq: 2,
                    memory_id: id.clone(),
                    op: MemorySyncOp::Delete,
                    revision: 2,
                    kind: None,
                    body: None,
                    event_date: None,
                    occurred_at: 2,
                }],
                1,
                2,
            )
            .unwrap();
        assert!(report.changed);
        assert!(list_active(&s).is_empty());
        let tombstone: i64 =
            s.0.query_row(
                "SELECT count(*) FROM memories WHERE id=?1 AND deleted_at IS NOT NULL
                 AND body IS NULL AND source_kind IS NULL",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tombstone, 1);
    }

    #[test]
    fn snapshot_diff_merges_local_only_entries_into_pushes() {
        let owner = "4318-snapshot";
        let mut s = store();
        let local_only = create(&mut s);
        // The create has been pushed: a snapshot tombstone may now delete it
        // locally instead of deferring to the pending push.
        let entry = row(&s, &local_only.id).unwrap();
        s.memory_outbox_ack(owner, &entry.request_id, &local_only.id, 1)
            .unwrap();
        let remote_id = uuid::Uuid::new_v4().to_string();
        let report = s
            .memory_apply_snapshot(
                owner,
                &[
                    MemorySnapshotItem {
                        id: remote_id.clone(),
                        kind: MemoryKind::Preference,
                        body: Some("from server".into()),
                        event_date: None,
                        revision: 3,
                        deleted: false,
                    },
                    MemorySnapshotItem {
                        id: local_only.id.clone(),
                        kind: MemoryKind::Preference,
                        body: Some("deleted elsewhere".into()),
                        event_date: None,
                        revision: 9,
                        deleted: true,
                    },
                ],
                2,
                40,
            )
            .unwrap();
        assert!(report.changed);
        let all = list_active(&s);
        assert!(all.iter().any(|m| m.id == remote_id));
        assert!(all.iter().all(|m| m.id != local_only.id));
        // The snapshot had no tombstone for the local-only entry: it is gone
        // server-side only via deletion… the diff enqueued nothing new.
        assert!(row(&s, &local_only.id).is_none());
        assert_eq!(s.memory_sync_state(owner).unwrap(), (2, 40));
        // A fresh store with a local entry the snapshot lacks merges by push.
        let mut fresh = store();
        let keep = create(&mut fresh);
        fresh
            .memory_apply_snapshot(owner, &[snapshot(remote_id.clone(), 1, false)], 1, 1)
            .unwrap();
        let pushed = row(&fresh, &keep.id).expect("local-only entry enqueued");
        assert_eq!(pushed.op, OutboxOp::Create);
        assert_eq!(pushed.body.as_deref(), Some("prefer tea"));
    }

    fn snapshot(id: String, revision: i64, deleted: bool) -> MemorySnapshotItem {
        MemorySnapshotItem {
            id,
            kind: MemoryKind::Preference,
            body: if deleted { None } else { Some("snap".into()) },
            event_date: None,
            revision,
            deleted,
        }
    }

    #[test]
    fn late_ack_of_a_folded_row_records_the_server_fact() {
        let owner = "4318-ack";
        let mut s = store();
        let m = create(&mut s);
        let entry = row(&s, &m.id).unwrap();
        s.memory_outbox_ack(owner, &entry.request_id, &m.id, 1)
            .unwrap();
        assert!(row(&s, &m.id).is_none());
        assert_eq!(s.memory_push_base(owner, &m.id).unwrap(), Some(1));
        // A folded row was replaced before the old push landed. The late ack
        // of the old id must still record the server's revision: that push
        // WAS accepted, and the folded row needs the recorded base — an
        // update rebases onto it (last-writer-wins), a delete retires the
        // server entry through it.
        let epoch = s.context_epoch().unwrap();
        s.memory_update(
            &m.id,
            local_revision(&s, &m.id).unwrap(),
            epoch,
            &draft("v2"),
        )
        .unwrap();
        let folded = row(&s, &m.id).unwrap();
        assert_ne!(folded.request_id, entry.request_id);
        s.memory_outbox_ack(owner, &entry.request_id, &m.id, 2)
            .unwrap();
        assert!(row(&s, &m.id).is_some(), "folded row survives");
        assert_eq!(
            s.memory_push_base(owner, &m.id).unwrap(),
            Some(2),
            "a server-accepted revision is a fact even for a folded-away row"
        );
        assert!(s.memory_outbox_drop(owner, &folded.request_id).unwrap());
        assert!(row(&s, &m.id).is_none());
    }

    #[test]
    fn in_flight_create_push_then_delete_retires_the_server_entry() {
        let owner = "4318-race";
        let mut s = store();
        let m = create(&mut s);
        let create_row = row(&s, &m.id).unwrap();
        // The user deletes while the create push is on the wire: folding
        // turns the row into a delete, the push lands, the (late) ack
        // records the server fact, and the delete now has a real base.
        let epoch = s.context_epoch().unwrap();
        s.memory_delete(&m.id, m.revision, epoch).unwrap();
        let delete_row = row(&s, &m.id).expect("delete row replaces the create");
        assert_eq!(delete_row.op, OutboxOp::Delete);
        s.memory_outbox_ack(owner, &create_row.request_id, &m.id, 1)
            .unwrap();
        assert_eq!(
            s.memory_push_base(owner, &m.id).unwrap(),
            Some(1),
            "the delete can now push against the accepted create"
        );
    }

    #[test]
    fn remote_materialization_skips_when_local_capacity_is_full() {
        let owner = "4318-capacity";
        let mut s = store();
        for i in 0..MAX_MEMORIES {
            let epoch = s.context_epoch().unwrap();
            s.memory_create(&draft(&format!("full {i}")), epoch)
                .unwrap();
        }
        // A remote-only entry arrives while every local slot is taken: the
        // pull must not fail (that would wedge the cursor); the entry is
        // skipped, counted, and mirrored so the stream moves on.
        let extra = uuid::Uuid::new_v4().to_string();
        let report = s
            .memory_apply_remote(
                owner,
                &[event(1, &extra, MemorySyncOp::Upsert, 1, Some("no room"))],
                1,
                1,
            )
            .unwrap();
        assert_eq!(report.skipped_capacity, 1);
        assert!(!report.changed);
        assert_eq!(list_active(&s).len(), MAX_MEMORIES);
        assert_eq!(s.memory_sync_state(owner).unwrap(), (1, 1));
        assert_eq!(
            s.memory_push_base(owner, &extra).unwrap(),
            Some(1),
            "mirror records the skipped revision"
        );
    }

    /// Test-only read of the active memories through the domain validator.
    fn list_active(store: &HistoryStore) -> Vec<Memory> {
        read_active(&store.0).unwrap()
    }
}

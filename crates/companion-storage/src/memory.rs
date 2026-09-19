//! Internal storage primitives. Host must coordinate cache invalidation before exposing writes.
use crate::{history::HistoryStore, StorageError};
use companion_core::{memory::*, now_ms};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;

#[derive(Debug)]
pub struct MemoryCommit<T> {
    pub value: T,
    pub context_epoch: i64,
    pub chat_cleared: bool,
}

fn timestamp() -> Result<i64, StorageError> {
    i64::try_from(now_ms())
        .ok()
        .filter(|v| *v <= MAX_COUNTER)
        .ok_or(StorageError::InvalidTimestamp)
}

fn epoch(db: &Connection) -> Result<i64, StorageError> {
    Ok(db.query_row(
        "SELECT context_epoch FROM memory_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?)
}
fn advance(db: &Connection, expected: i64) -> Result<i64, StorageError> {
    if epoch(db)? != expected {
        return Err(MemoryError::ContextChanged.into());
    }
    let next = next_counter(expected)?;
    db.execute(
        "UPDATE memory_meta SET context_epoch=?1 WHERE singleton=1",
        [next],
    )?;
    Ok(next)
}

fn read_active(db: &Connection) -> Result<Vec<Memory>, StorageError> {
    let mut statement = db.prepare("SELECT id,kind,body,source_kind,source_label,event_date,created_at,confirmed_at,updated_at,revision FROM memories WHERE deleted_at IS NULL ORDER BY created_at,id")?;
    let rows = statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, i64>(8)?,
            r.get::<_, i64>(9)?,
        ))
    })?;
    let mut memories = Vec::new();
    for row in rows {
        let (id, kind, body, source, label, date, created, confirmed, updated, revision) = row?;
        let kind = match kind.as_str() {
            "preference" => MemoryKind::Preference,
            "experience" => MemoryKind::Experience,
            _ => return Err(StorageError::Unavailable),
        };
        let draft = MemoryDraft {
            kind,
            body: body.clone(),
            event_date: date.clone(),
        }
        .validate()?;
        if uuid::Uuid::parse_str(&id).is_err()
            || draft.body != body
            || source != "user_manual"
            || label != MANUAL_SOURCE_LABEL
            || !(1..=MAX_COUNTER).contains(&revision)
            || created < 0
            || created > confirmed
            || confirmed != updated
            || updated > MAX_COUNTER
        {
            return Err(StorageError::Unavailable);
        }
        memories.push(Memory {
            id,
            kind,
            body,
            source_kind: MemorySource::UserManual,
            source_label: label,
            event_date: date,
            created_at: created,
            confirmed_at: confirmed,
            updated_at: updated,
            revision,
        });
    }
    if memories.len() > MAX_MEMORIES {
        return Err(StorageError::Unavailable);
    }
    Ok(memories)
}

pub(crate) fn validate_schema(db: &Connection) -> Result<(), StorageError> {
    if !(0..=MAX_COUNTER).contains(&epoch(db)?) {
        return Err(StorageError::Unavailable);
    }
    let active = read_active(db)?;
    let invalid: i64 = db.query_row("SELECT count(*) FROM memories WHERE deleted_at IS NOT NULL AND (body IS NOT NULL OR source_kind IS NOT NULL OR source_label IS NOT NULL OR event_date IS NOT NULL OR created_at IS NOT NULL OR confirmed_at IS NOT NULL OR updated_at IS NOT NULL)", [], |r| r.get(0))?;
    if invalid != 0 {
        return Err(StorageError::Unavailable);
    }
    let mut policies = db.prepare("SELECT base_url,model,enabled,revision FROM memory_policy")?;
    for row in policies.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })? {
        let (base, model, enabled, revision) = row?;
        let mut selections = db.prepare("SELECT memory_id FROM memory_selection WHERE base_url=?1 AND model=?2 ORDER BY position")?;
        let ids = selections
            .query_map(params![base, model], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if base.is_empty()
            || model.is_empty()
            || !(0..=MAX_COUNTER).contains(&revision)
            || ![0, 1].contains(&enabled)
            || (enabled == 1) == ids.is_empty()
        {
            return Err(StorageError::Unavailable);
        }
        validate_selection(&ids, &active)?;
    }
    if db.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(StorageError::Unavailable);
    }
    Ok(())
}

impl HistoryStore {
    pub fn context_epoch(&self) -> Result<i64, StorageError> {
        epoch(&self.0)
    }
    pub fn memory_list(&self) -> Result<Vec<Memory>, StorageError> {
        read_active(&self.0)
    }

    /// Explicit user input only; no caller-controlled ID, provenance or timestamps.
    pub fn memory_create(
        &mut self,
        draft: &MemoryDraft,
        expected_epoch: i64,
    ) -> Result<MemoryCommit<Memory>, StorageError> {
        let draft = draft.validate()?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let context_epoch = advance(&tx, expected_epoch)?;
        if read_active(&tx)?.len() >= MAX_MEMORIES {
            return Err(MemoryError::CapacityExceeded.into());
        }
        let now = timestamp()?;
        let memory = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            kind: draft.kind,
            body: draft.body,
            source_kind: MemorySource::UserManual,
            source_label: MANUAL_SOURCE_LABEL.into(),
            event_date: draft.event_date,
            created_at: now,
            confirmed_at: now,
            updated_at: now,
            revision: 1,
        };
        tx.execute("INSERT INTO memories(id,kind,body,source_kind,source_label,event_date,created_at,confirmed_at,updated_at,revision) VALUES(?1,?2,?3,'user_manual',?4,?5,?6,?6,?6,1)",
            params![memory.id,memory.kind.as_str(),memory.body,memory.source_label,memory.event_date,now])?;
        tx.commit()?;
        Ok(MemoryCommit {
            value: memory,
            context_epoch,
            chat_cleared: false,
        })
    }

    /// Host must obtain explicit restart confirmation and invalidate in-memory chat after commit.
    pub fn memory_update(
        &mut self,
        id: &str,
        expected_revision: i64,
        expected_epoch: i64,
        draft: &MemoryDraft,
    ) -> Result<MemoryCommit<Memory>, StorageError> {
        let draft = draft.validate()?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let context_epoch = advance(&tx, expected_epoch)?;
        let mut memory = read_active(&tx)?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or(MemoryError::NotFound)?;
        if memory.revision != expected_revision {
            return Err(MemoryError::Conflict.into());
        }
        memory.revision = next_counter(memory.revision)?;
        memory.body = draft.body;
        memory.kind = draft.kind;
        memory.event_date = draft.event_date;
        memory.updated_at = timestamp()?.max(memory.updated_at);
        memory.confirmed_at = memory.updated_at;
        tx.execute("UPDATE memories SET kind=?2,body=?3,event_date=?4,updated_at=?5,confirmed_at=?5,revision=?6 WHERE id=?1",
            params![id,memory.kind.as_str(),memory.body,memory.event_date,memory.updated_at,memory.revision])?;
        // Revalidate every model selection, including ones not currently displayed in the UI.
        validate_schema(&tx)?;
        tx.execute("DELETE FROM chat_turns", [])?;
        tx.commit()?;
        Ok(MemoryCommit {
            value: memory,
            context_epoch,
            chat_cleared: true,
        })
    }

    /// Repeating an already committed deletion does not clear subsequently created chats.
    pub fn memory_delete(
        &mut self,
        id: &str,
        expected_revision: i64,
        expected_epoch: i64,
    ) -> Result<MemoryCommit<()>, StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (revision, deleted): (i64, Option<i64>) = tx
            .query_row(
                "SELECT revision,deleted_at FROM memories WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(MemoryError::NotFound)?;
        if deleted.is_some() {
            return Ok(MemoryCommit {
                value: (),
                context_epoch: epoch(&tx)?,
                chat_cleared: false,
            });
        }
        if revision != expected_revision {
            return Err(MemoryError::Conflict.into());
        }
        let context_epoch = advance(&tx, expected_epoch)?;
        let revision = next_counter(revision)?;
        remove_selections(&tx, Some(id))?;
        tx.execute("UPDATE memories SET body=NULL,source_kind=NULL,source_label=NULL,event_date=NULL,created_at=NULL,confirmed_at=NULL,updated_at=NULL,revision=?2,deleted_at=?3 WHERE id=?1", params![id,revision,timestamp()?])?;
        tx.execute("DELETE FROM chat_turns", [])?;
        tx.commit()?;
        Ok(MemoryCommit {
            value: (),
            context_epoch,
            chat_cleared: true,
        })
    }

    pub fn memory_delete_all(
        &mut self,
        expected_epoch: i64,
    ) -> Result<MemoryCommit<()>, StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let context_epoch = advance(&tx, expected_epoch)?;
        // Check overflow before SQL arithmetic; never wrap or promote the integer to REAL.
        for memory in read_active(&tx)? {
            next_counter(memory.revision)?;
        }
        remove_selections(&tx, None)?;
        tx.execute("UPDATE memories SET body=NULL,source_kind=NULL,source_label=NULL,event_date=NULL,created_at=NULL,confirmed_at=NULL,updated_at=NULL,revision=revision+1,deleted_at=?1 WHERE deleted_at IS NULL", [timestamp()?])?;
        tx.execute("DELETE FROM chat_turns", [])?;
        tx.commit()?;
        Ok(MemoryCommit {
            value: (),
            context_epoch,
            chat_cleared: true,
        })
    }
}

fn remove_selections(db: &Connection, id: Option<&str>) -> Result<(), StorageError> {
    let mut statement = db.prepare("SELECT base_url,model,revision FROM memory_policy WHERE (?1 IS NULL AND (enabled=1 OR EXISTS(SELECT 1 FROM memory_selection s WHERE s.base_url=memory_policy.base_url AND s.model=memory_policy.model))) OR EXISTS(SELECT 1 FROM memory_selection s WHERE s.base_url=memory_policy.base_url AND s.model=memory_policy.model AND s.memory_id=?1)")?;
    let policies = statement
        .query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    db.execute(
        "DELETE FROM memory_selection WHERE ?1 IS NULL OR memory_id=?1",
        [id],
    )?;
    for (base, model, revision) in policies {
        db.execute("UPDATE memory_policy SET revision=?3,enabled=CASE WHEN EXISTS(SELECT 1 FROM memory_selection WHERE base_url=?1 AND model=?2) THEN enabled ELSE 0 END WHERE base_url=?1 AND model=?2", params![base,model,next_counter(revision)?])?;
    }
    Ok(())
}

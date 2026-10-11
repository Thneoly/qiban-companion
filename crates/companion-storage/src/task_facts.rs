//! Task-fact settlement: the ONLY write path for executor-sourced facts.
//! Deposit is idempotent per task fingerprint (redo retargets the mapping to
//! a NEW row and tombstones the previous one), system eviction tombstones
//! without pruning chats, and the epoch advances so in-flight completions
//! die — but no precise prune fires here: replacement and eviction are
//! system actions, the confirmation gate over chat cleanup belongs to user
//! deletes (which run the ordinary memory_delete path).
use crate::history::HistoryStore;
use crate::memory::{
    advance, epoch, read_active, remove_selections, validate_schema, MemoryCommit,
};
use crate::StorageError;
use companion_core::memory::*;
use companion_core::now_ms;
use rusqlite::{params, OptionalExtension, TransactionBehavior};

fn timestamp() -> Result<i64, StorageError> {
    i64::try_from(now_ms())
        .ok()
        .filter(|v| *v <= MAX_COUNTER)
        .ok_or(StorageError::InvalidTimestamp)
}

/// Tombstone a fact row WITHOUT the precise prune (system action). The
/// revision counter is checked in Rust, never promoted by SQL arithmetic.
pub(crate) fn retire_fact(tx: &rusqlite::Connection, id: &str) -> Result<(), StorageError> {
    let revision: Option<i64> = tx
        .query_row(
            "SELECT revision FROM memories WHERE id=?1 AND deleted_at IS NULL",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(revision) = revision else {
        return Ok(());
    };
    let revision = next_counter(revision)?;
    remove_selections(tx, Some(id))?;
    tx.execute(
        "UPDATE memories SET body=NULL,source_kind=NULL,source_label=NULL,event_date=NULL,
         created_at=NULL,confirmed_at=NULL,updated_at=NULL,revision=?2,deleted_at=?3
         WHERE id=?1 AND deleted_at IS NULL",
        params![id, revision, timestamp()?],
    )?;
    crate::memory_sync_outbox::enqueue(
        tx,
        crate::memory_sync_outbox::OutboxOp::Delete,
        id,
        None,
        None,
        None,
    )?;
    Ok(())
}

impl HistoryStore {
    /// Settle one task fact. Same fingerprint deposits twice -> the second
    /// call replaces the first (old row tombstoned, mapping retargeted, no
    /// new row). Returns the freshly deposited fact.
    pub fn task_fact_deposit(
        &mut self,
        task_fingerprint: &str,
        body: &str,
        event_date: Option<&str>,
    ) -> Result<MemoryCommit<Memory>, StorageError> {
        // Shape rules shared with manual rows (kind/date rules do not apply).
        let body = body.trim();
        if body.is_empty()
            || body.chars().count() > MAX_BODY_CHARS
            || body
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            || task_fingerprint.is_empty()
            || task_fingerprint.chars().count() > 128
        {
            return Err(MemoryError::InvalidInput.into());
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Idempotence: the mapping already points at an ACTIVE fact carrying
        // this exact body -> nothing to settle.
        let existing: Option<String> = tx
            .query_row(
                "SELECT memory_id FROM task_fact_index WHERE task_fingerprint=?1",
                [task_fingerprint],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = &existing {
            let same: Option<bool> = tx
                .query_row(
                    "SELECT body=?2 FROM memories WHERE id=?1 AND deleted_at IS NULL",
                    params![id, body],
                    |r| r.get(0),
                )
                .optional()?;
            if same == Some(true) {
                let memory = read_active(&tx)?
                    .into_iter()
                    .find(|m| &m.id == id)
                    .ok_or(StorageError::Unavailable)?;
                return Ok(MemoryCommit {
                    value: memory,
                    context_epoch: epoch(&tx)?,
                    chat_cleared: false,
                    cleared_turns: 0,
                });
            }
            // A tombstoned mapping (user deleted the fact) falls through to
            // the redo path: a fresh id, the deleted one stays dead.
        }
        // Redo: retire the previous row (tombstone + delete push; the old id
        // stays dead on both ends — redo lands on a brand-new id).
        if let Some(old) = &existing {
            retire_fact(&tx, old)?;
        }
        // FIFO capacity: evict the OLDEST active task fact; never a manual
        // memory, never a chat.
        let facts: i64 = tx.query_row(
            "SELECT count(*) FROM memories WHERE deleted_at IS NULL AND kind='task_fact'",
            [],
            |r| r.get(0),
        )?;
        if facts >= MAX_TASK_FACTS as i64 {
            let oldest: Option<String> = tx
                .query_row(
                    "SELECT id FROM memories WHERE deleted_at IS NULL AND kind='task_fact'
                     ORDER BY created_at,id LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(oldest) = oldest {
                // The redo target itself is the oldest — already retired above.
                if existing.as_deref() != Some(oldest.as_str()) {
                    retire_fact(&tx, &oldest)?;
                }
            }
        }
        let now = timestamp()?;
        let memory = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            kind: MemoryKind::TaskFact,
            body: body.into(),
            source_kind: MemorySource::Executor,
            source_label: companion_core::execution::TASK_FACT_SOURCE_LABEL.into(),
            event_date: event_date.map(Into::into),
            created_at: now,
            confirmed_at: now,
            updated_at: now,
            revision: 1,
        };
        let context_epoch = advance(&tx, epoch(&tx)?)?;
        tx.execute(
            "INSERT INTO memories(id,kind,body,source_kind,source_label,event_date,created_at,confirmed_at,updated_at,revision)
             VALUES(?1,'task_fact',?2,'executor',?3,?4,?5,?5,?5,1)",
            params![
                memory.id,
                memory.body,
                memory.source_label,
                memory.event_date,
                now
            ],
        )?;
        crate::memory_sync_outbox::enqueue(
            &tx,
            crate::memory_sync_outbox::OutboxOp::Create,
            &memory.id,
            Some(&memory.kind),
            Some(&memory.body),
            memory.event_date.as_deref(),
        )?;
        tx.execute(
            "INSERT INTO task_fact_index(task_fingerprint,memory_id) VALUES(?1,?2)
             ON CONFLICT(task_fingerprint) DO UPDATE SET memory_id=excluded.memory_id",
            params![task_fingerprint, memory.id],
        )?;
        validate_schema(&tx)?;
        tx.commit()?;
        Ok(MemoryCommit {
            value: memory,
            context_epoch,
            chat_cleared: false,
            cleared_turns: 0,
        })
    }

    /// The mapping row for a fingerprint, if any (used by tests and the
    /// bridge's diagnostics). Reconciliation itself is ledger-driven from the
    /// execution store's pending rows — a reverse scan over "terminal tasks
    /// without a fact" would re-deposit facts the user deliberately deleted.
    pub fn task_fact_mapping(
        &self,
        task_fingerprint: &str,
    ) -> Result<Option<String>, StorageError> {
        Ok(self
            .0
            .query_row(
                "SELECT memory_id FROM task_fact_index WHERE task_fingerprint=?1",
                [task_fingerprint],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> HistoryStore {
        HistoryStore::open(
            &std::env::temp_dir().join(format!("qiban-facts-{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap()
    }
    fn facts(s: &HistoryStore) -> Vec<Memory> {
        read_active(&s.0)
            .unwrap()
            .into_iter()
            .filter(|m| m.kind == MemoryKind::TaskFact)
            .collect()
    }
    fn manual(s: &HistoryStore) -> Vec<Memory> {
        read_active(&s.0)
            .unwrap()
            .into_iter()
            .filter(|m| m.kind != MemoryKind::TaskFact)
            .collect()
    }

    #[test]
    fn deposit_settles_one_fact_per_fingerprint_and_redo_replaces() {
        let mut s = store();
        let first = s
            .task_fact_deposit(
                "fp-1",
                "把《笔记》的摘录保存为草稿：已核验完成",
                Some("2026-10-11"),
            )
            .unwrap();
        assert_eq!(facts(&s).len(), 1);
        assert_eq!(
            s.task_fact_mapping("fp-1").unwrap().as_deref(),
            Some(first.value.id.as_str())
        );
        // Same fingerprint, same body: idempotent, still one row.
        let again = s
            .task_fact_deposit(
                "fp-1",
                "把《笔记》的摘录保存为草稿：已核验完成",
                Some("2026-10-11"),
            )
            .unwrap();
        assert_eq!(again.value.id, first.value.id);
        assert_eq!(facts(&s).len(), 1);
        // Redo with a NEW outcome: old row tombstoned (a NEW id, never a
        // resurrection), mapping retargeted, still exactly one active fact.
        let redone = s
            .task_fact_deposit(
                "fp-1",
                "把《笔记》的摘录保存为草稿：结果未知",
                Some("2026-10-12"),
            )
            .unwrap();
        assert_ne!(redone.value.id, first.value.id);
        let active = facts(&s);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, redone.value.id);
        assert_eq!(active[0].body, "把《笔记》的摘录保存为草稿：结果未知");
        assert_eq!(
            s.task_fact_mapping("fp-1").unwrap().as_deref(),
            Some(redone.value.id.as_str())
        );
        // The old row is a content-free tombstone.
        let tombstoned: i64 =
            s.0.query_row(
                "SELECT count(*) FROM memories WHERE id=?1 AND deleted_at IS NOT NULL
                 AND body IS NULL AND source_kind IS NULL AND source_label IS NULL",
                [&first.value.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tombstoned, 1);
    }

    #[test]
    fn user_delete_then_redo_lands_a_fresh_id() {
        let mut s = store();
        let first = s
            .task_fact_deposit("fp-2", "任务甲：已核验完成", None)
            .unwrap()
            .value;
        let epoch = s.context_epoch().unwrap();
        s.memory_delete(&first.id, first.revision, epoch).unwrap();
        assert!(facts(&s).is_empty());
        // Redo settles a brand-new id (the deleted one stays dead locally and
        // never resurrects through the sync mirror).
        let redone = s
            .task_fact_deposit("fp-2", "任务甲：结果未知", None)
            .unwrap();
        assert_ne!(redone.value.id, first.id);
        assert_eq!(facts(&s).len(), 1);
    }

    #[test]
    fn capacity_fifo_evicts_oldest_fact_and_never_a_manual_memory() {
        let mut s = store();
        // A manual memory must survive everything the facts do.
        let epoch = s.context_epoch().unwrap();
        let keep = s
            .memory_create(
                &MemoryDraft {
                    kind: MemoryKind::Preference,
                    body: "手动记忆不被动".into(),
                    event_date: None,
                },
                epoch,
            )
            .unwrap()
            .value;
        for index in 0..MAX_TASK_FACTS {
            s.task_fact_deposit(
                &format!("fp-{index}"),
                &format!("任务{index}：已核验完成"),
                None,
            )
            .unwrap();
        }
        assert_eq!(facts(&s).len(), MAX_TASK_FACTS);
        // One more: the OLDEST fact is evicted (tombstone only — no chat
        // prune, asserted by ledger absence below), nothing else moves.
        s.task_fact_deposit("fp-new", "新任务：已核验完成", None)
            .unwrap();
        let active = facts(&s);
        assert_eq!(active.len(), MAX_TASK_FACTS);
        assert!(
            active.iter().all(|m| m.body != "任务0：已核验完成"),
            "oldest fact evicted"
        );
        assert!(active.iter().any(|m| m.body == "新任务：已核验完成"));
        // The manual memory is untouched and the kind-split caps hold.
        assert_eq!(manual(&s).len(), 1);
        assert_eq!(manual(&s)[0].id, keep.id);
        validate_schema(&s.0).unwrap();
        // Eviction tombstones WITHOUT pruning: no usage ledger was cleared
        // (nothing to clear here) and the evicted id keeps its row.
        let evicted_tombstone: i64 =
            s.0.query_row(
                "SELECT count(*) FROM memories WHERE deleted_at IS NOT NULL AND kind='task_fact'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(evicted_tombstone >= 1);
    }

    #[test]
    fn deposit_rejects_bad_shape_and_fingerprint() {
        let mut s = store();
        assert!(s.task_fact_deposit("fp", " ", None).is_err());
        assert!(s.task_fact_deposit("fp", &"x".repeat(201), None).is_err());
        assert!(s.task_fact_deposit("", "正文", None).is_err());
        assert!(facts(&s).is_empty());
    }
}
#[cfg(test)]
mod bridge_tests {
    use super::*;
    use crate::execution::ExecutionStore;
    use companion_core::execution::{task_fact_projection, ExecutionStatus};

    /// The host bridge's core sequence, against both real stores: drain the
    /// execution ledger, deposit, clear. Crashing between the execution
    /// commit and the deposit (the two-database two-phase window) re-drains
    /// the same row next tick — idempotent per fingerprint.
    #[test]
    fn ledger_drain_is_idempotent_across_the_two_phase_window() {
        let exec = ExecutionStore::open(
            &std::env::temp_dir().join(format!("qiban-exec-{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let mut history = HistoryStore::open(
            &std::env::temp_dir().join(format!("qiban-hist-{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let request_id = uuid::Uuid::new_v4().to_string();
        let task = exec
            .prepare(&request_id, "报告.txt", "第一行\n第二行")
            .unwrap();
        // No terminal state yet: nothing pending.
        assert!(exec.fact_pending().unwrap().is_empty());
        // Walk to Completed via the real claim/resolve path.
        let attempt = exec
            .claim(&task.id, task.revision, "test-owner")
            .unwrap()
            .unwrap();
        exec.resolve(&task.id, &attempt.id, ExecutionStatus::Completed, "done")
            .unwrap();
        let pending = exec.fact_pending().unwrap();
        assert_eq!(
            pending.len(),
            1,
            "terminal registers in the same transaction"
        );
        let (fingerprint, task_id, task) = pending.into_iter().next().unwrap();
        assert_eq!(task.status, ExecutionStatus::Completed);
        // Crash window: drain TWICE (the second is the retry after a crash
        // between deposit and clearing the ledger row).
        let (body, date) = task_fact_projection(&task).unwrap();
        history
            .task_fact_deposit(&fingerprint, &body, date.as_deref())
            .unwrap();
        history
            .task_fact_deposit(&fingerprint, &body, date.as_deref())
            .unwrap();
        exec.fact_settled(&task_id).unwrap();
        assert!(exec.fact_pending().unwrap().is_empty());
        let facts: Vec<Memory> = read_active(&history.0)
            .unwrap()
            .into_iter()
            .filter(|m| m.kind == MemoryKind::TaskFact)
            .collect();
        assert_eq!(facts.len(), 1, "the retry settled no duplicate");
        assert_eq!(
            history.task_fact_mapping(&fingerprint).unwrap().as_deref(),
            Some(facts[0].id.as_str())
        );
    }
}

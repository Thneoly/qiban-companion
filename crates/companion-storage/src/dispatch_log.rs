//! Send-audit dispatch log (model-visible means logged): every request that
//! left for a model service lands one row — sent, cancelled, timed out or
//! dropped — with enough of its composition (system version, memory block
//! verbatim, personal digest, history count, prompt) to reconstruct what the
//! model was shown. Independent lifecycle: a ring, never tied to turn rows
//! (failed requests have none), untouched by turn pruning and the 6-turn
//! global eviction; only "delete all chats" clears it (history.rs::clear).
use crate::history::HistoryStore;
use crate::StorageError;
use companion_core::now_ms;
use rusqlite::params;
use serde::Serialize;

/// Rows kept per scope (the whole log is one local scope). Generous against
/// the 6-turn archive: the point is a durable window, not an archive.
const DISPATCH_RING: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchOutcome {
    Sent,
    Cancelled,
    Timeout,
    Dropped,
}
impl DispatchOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::Dropped => "dropped",
        }
    }
}

/// One dispatch row. `memory_block`/`personal_digest` are None when that
/// segment was absent from the request (explicit empty markers, so a broken
/// logger writing NULL-for-everything fails the rebuild check).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchRow {
    pub seq: i64,
    pub request_id: String,
    pub outcome: DispatchOutcome,
    pub model: String,
    pub base_url: String,
    pub dispatched_at: i64,
    pub system_version: i64,
    pub memory_block: Option<String>,
    pub personal_digest: Option<String>,
    pub history_turns: i64,
    pub memory_count: i64,
    pub memory_chars: i64,
    pub prompt: String,
}

/// The composition of one outgoing model request, captured at assembly time.
pub struct Dispatch<'a> {
    pub request_id: &'a str,
    pub outcome: DispatchOutcome,
    pub model: &'a str,
    pub base_url: &'a str,
    pub system_version: i64,
    pub memory_block: Option<&'a str>,
    pub personal_digest: Option<&'a str>,
    pub history_turns: usize,
    pub memory_count: usize,
    pub memory_chars: usize,
    pub prompt: &'a str,
}

impl HistoryStore {
    /// Append one dispatch row and trim the ring. request_id is UNIQUE: a
    /// same-id re-send is a client bug and lands as a constraint error
    /// rather than silently duplicating history.
    pub fn dispatch_log_append(&mut self, entry: &Dispatch<'_>) -> Result<(), StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO chat_dispatch_log(request_id,outcome,model,base_url,dispatched_at,
             system_version,memory_block,personal_digest,history_turns,memory_count,memory_chars,prompt)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                entry.request_id,
                entry.outcome.as_str(),
                entry.model,
                entry.base_url,
                i64::try_from(now_ms()).map_err(|_| StorageError::InvalidTimestamp)?,
                entry.system_version,
                entry.memory_block,
                entry.personal_digest,
                entry.history_turns as i64,
                entry.memory_count as i64,
                entry.memory_chars as i64,
                entry.prompt
            ],
        )?;
        tx.execute(
            "DELETE FROM chat_dispatch_log WHERE seq <= (
               SELECT MAX(seq) - ?1 FROM chat_dispatch_log
             )",
            [DISPATCH_RING],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The ring, oldest first (diagnostics + tests; no host command yet).
    pub fn dispatch_log_recent(&self) -> Result<Vec<DispatchRow>, StorageError> {
        let mut query = self.0.prepare(
            "SELECT seq,request_id,outcome,model,base_url,dispatched_at,system_version,
             memory_block,personal_digest,history_turns,memory_count,memory_chars,prompt
             FROM chat_dispatch_log ORDER BY seq",
        )?;
        let rows = query.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, i64>(10)?,
                r.get::<_, i64>(11)?,
                r.get::<_, String>(12)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                seq,
                request_id,
                outcome,
                model,
                base_url,
                dispatched_at,
                system_version,
                memory_block,
                personal_digest,
                history_turns,
                memory_count,
                memory_chars,
                prompt,
            ) = row?;
            let outcome = match outcome.as_str() {
                "sent" => DispatchOutcome::Sent,
                "cancelled" => DispatchOutcome::Cancelled,
                "timeout" => DispatchOutcome::Timeout,
                _ => DispatchOutcome::Dropped,
            };
            out.push(DispatchRow {
                seq,
                request_id,
                outcome,
                model,
                base_url,
                dispatched_at,
                system_version,
                memory_block,
                personal_digest,
                history_turns,
                memory_count,
                memory_chars,
                prompt,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> HistoryStore {
        HistoryStore::open(
            &std::env::temp_dir().join(format!("qiban-dispatch-{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap()
    }
    fn append(s: &mut HistoryStore, n: i64, outcome: DispatchOutcome, memory_block: Option<&str>) {
        let prompt = format!("第{n}轮输入");
        let request_id = format!("req-{n}");
        s.dispatch_log_append(&Dispatch {
            request_id: &request_id,
            outcome,
            model: "fixture-model",
            base_url: "https://fixture.test",
            system_version: 1,
            memory_block,
            personal_digest: Some("deadbeef"),
            history_turns: 3,
            memory_count: 2,
            memory_chars: 42,
            prompt: &prompt,
        })
        .unwrap();
    }

    #[test]
    fn rows_roundtrip_every_segment_and_outcomes_map() {
        let mut s = store();
        append(&mut s, 1, DispatchOutcome::Sent, Some("参考资料：甲\n乙"));
        append(&mut s, 2, DispatchOutcome::Cancelled, None);
        append(&mut s, 3, DispatchOutcome::Timeout, None);
        append(&mut s, 4, DispatchOutcome::Dropped, None);
        let rows = s.dispatch_log_recent().unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].request_id, "req-1");
        assert_eq!(rows[0].outcome, DispatchOutcome::Sent);
        assert_eq!(rows[0].memory_block.as_deref(), Some("参考资料：甲\n乙"));
        assert_eq!(rows[0].system_version, 1);
        assert_eq!(rows[0].history_turns, 3);
        assert_eq!(rows[0].memory_count, 2);
        assert_eq!(rows[0].memory_chars, 42);
        assert_eq!(rows[0].prompt, "第1轮输入");
        // Explicit-empty markers: absent segments are None, not "".
        assert_eq!(rows[1].memory_block, None);
        assert_eq!(rows[1].personal_digest.as_deref(), Some("deadbeef"));
        assert_eq!(rows[2].outcome, DispatchOutcome::Timeout);
        assert_eq!(rows[3].outcome, DispatchOutcome::Dropped);
        // Same request id twice is a client bug: the UNIQUE key rejects it
        // rather than duplicating history.
        assert!(s
            .dispatch_log_append(&Dispatch {
                request_id: "req-1",
                outcome: DispatchOutcome::Sent,
                model: "m",
                base_url: "b",
                system_version: 1,
                memory_block: None,
                personal_digest: None,
                history_turns: 0,
                memory_count: 0,
                memory_chars: 0,
                prompt: "p",
            })
            .is_err());
    }

    #[test]
    fn ring_trims_to_bound_and_survives_turn_eviction() {
        let mut s = store();
        for n in 1..=(DISPATCH_RING + 40) {
            append(&mut s, n, DispatchOutcome::Sent, Some("块"));
        }
        let rows = s.dispatch_log_recent().unwrap();
        assert_eq!(rows.len() as i64, DISPATCH_RING);
        assert_eq!(
            rows[0].request_id, "req-41",
            "oldest beyond the ring is gone"
        );
        assert_eq!(
            rows.last().unwrap().request_id,
            format!("req-{}", DISPATCH_RING + 40)
        );
        // The 6-turn/12k-char global eviction clears turns, never audit rows.
        for n in 0..8 {
            s.append_with_usage(
                "https://fixture.test",
                "fixture-model",
                &companion_core::conversation::ChatTurn {
                    user: format!("问{n}"),
                    assistant: format!("答{n}"),
                },
                &[],
            )
            .unwrap();
        }
        assert_eq!(
            s.dispatch_log_recent().unwrap().len() as i64,
            DISPATCH_RING,
            "turn eviction must not touch the audit ring"
        );
    }

    #[test]
    fn clear_deletes_the_audit_with_the_chats() {
        let mut s = store();
        append(&mut s, 1, DispatchOutcome::Sent, None);
        append(&mut s, 2, DispatchOutcome::Cancelled, None);
        s.append_with_usage(
            "https://fixture.test",
            "fixture-model",
            &companion_core::conversation::ChatTurn {
                user: "问".into(),
                assistant: "答".into(),
            },
            &[],
        )
        .unwrap();
        s.clear().unwrap();
        assert!(s.dispatch_log_recent().unwrap().is_empty());
        assert!(s
            .load("https://fixture.test", "fixture-model")
            .unwrap()
            .is_empty());
    }
}

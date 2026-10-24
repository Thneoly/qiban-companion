//! Local-search authorization and tool audit (read-only search, model-driven).
//! Two lifecycles by design: `authorized_roots` is standing user consent —
//! it survives chat_clear and changes only through the settings command;
//! `tool_invocation_log` is request content — every tool result replayed
//! into a model request lands one row with the EXACT payload sent, so
//! "what file content left this machine" stays answerable locally. The log
//! is a ring like the dispatch log and is cleared with the chats.
use crate::history::HistoryStore;
use crate::StorageError;
use companion_core::now_ms;
use rusqlite::{params, Connection};
use serde::Serialize;

/// Rows kept for the tool audit (one local scope, same ring idea as the
/// dispatch log). 200 rows x an 8KB excerpt bound stays under ~2MB.
const TOOL_LOG_RING: i64 = 200;

/// A user-authorized search root (canonicalized absolute path).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedRoot {
    pub id: i64,
    pub path: String,
    pub label: Option<String>,
    pub added_at: i64,
}

/// Insert payload for a new root; path must arrive canonicalized.
pub struct AuthorizedRootInsert<'a> {
    pub path: &'a str,
    pub label: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Ok,
    Denied,
    Error,
    Budget,
    UnknownTool,
}
impl ToolOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::Error => "error",
            Self::Budget => "budget",
            Self::UnknownTool => "unknown_tool",
        }
    }
    fn from_str(value: &str) -> Self {
        match value {
            "ok" => Self::Ok,
            "denied" => Self::Denied,
            "error" => Self::Error,
            "budget" => Self::Budget,
            _ => Self::UnknownTool,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInvocationRow {
    pub seq: i64,
    pub request_id: String,
    pub round: i64,
    pub tool: String,
    pub arguments: String,
    pub outcome: ToolOutcome,
    pub result_chars: i64,
    pub result_excerpt: Option<String>,
    pub duration_ms: i64,
    pub occurred_at: i64,
}

/// One tool execution, written BEFORE the result is sent anywhere.
pub struct ToolInvocation<'a> {
    pub request_id: &'a str,
    pub round: usize,
    pub tool: &'a str,
    /// Raw arguments string exactly as the model produced it.
    pub arguments: &'a str,
    pub outcome: ToolOutcome,
    pub result_chars: usize,
    /// The exact payload sent to the model (already 8KB-capped by the
    /// engine); None for outcomes that produced no payload.
    pub result_excerpt: Option<&'a str>,
    pub duration_ms: u64,
}

/// The shared audit INSERT + ring trim, usable on the store's own
/// connection and on private blocking-thread connections alike.
fn tool_log_insert(
    connection: &rusqlite::Connection,
    entry: &ToolInvocation<'_>,
) -> Result<(), StorageError> {
    connection.execute(
        "INSERT INTO tool_invocation_log(request_id,round,tool,arguments,outcome,
         result_chars,result_excerpt,duration_ms,occurred_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            entry.request_id,
            entry.round as i64,
            entry.tool,
            entry.arguments,
            entry.outcome.as_str(),
            entry.result_chars as i64,
            entry.result_excerpt,
            entry.duration_ms as i64,
            i64::try_from(now_ms()).map_err(|_| StorageError::InvalidTimestamp)?
        ],
    )?;
    connection.execute(
        "DELETE FROM tool_invocation_log WHERE seq <= (
           SELECT MAX(seq) - ?1 FROM tool_invocation_log
         )",
        [TOOL_LOG_RING],
    )?;
    Ok(())
}

impl HistoryStore {
    pub fn authorized_roots(&self) -> Result<Vec<AuthorizedRoot>, StorageError> {
        let mut query = self
            .0
            .prepare("SELECT id,path,label,added_at FROM authorized_roots ORDER BY id")?;
        let rows = query.query_map([], |r| {
            Ok(AuthorizedRoot {
                id: r.get(0)?,
                path: r.get(1)?,
                label: r.get(2)?,
                added_at: r.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Add a root; the UNIQUE(path) key makes a duplicate add an error
    /// rather than a second row silently shadowing the first.
    pub fn authorized_root_add(
        &mut self,
        entry: &AuthorizedRootInsert<'_>,
    ) -> Result<AuthorizedRoot, StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO authorized_roots(path,label,added_at) VALUES(?1,?2,?3)",
            params![
                entry.path,
                entry.label,
                i64::try_from(now_ms()).map_err(|_| StorageError::InvalidTimestamp)?
            ],
        )?;
        let row = tx.query_row(
            "SELECT id,path,label,added_at FROM authorized_roots WHERE path=?1",
            params![entry.path],
            |r| {
                Ok(AuthorizedRoot {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    label: r.get(2)?,
                    added_at: r.get(3)?,
                })
            },
        )?;
        tx.commit()?;
        Ok(row)
    }

    pub fn authorized_root_remove(&mut self, id: i64) -> Result<(), StorageError> {
        self.0
            .execute("DELETE FROM authorized_roots WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Append one tool invocation row and trim the ring. Written before the
    /// result is sent; a dropped loop never leaves an unsent-but-logged row.
    pub fn tool_log_append(&mut self, entry: &ToolInvocation<'_>) -> Result<(), StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tool_log_insert(&tx, entry)?;
        tx.commit()?;
        Ok(())
    }

    /// Append one tool invocation row on a PRIVATE connection. Used from
    /// blocking threads where the shared store handle cannot travel: a
    /// cancelled turn must never leave an executed-but-unaudited tool read.
    pub fn tool_log_append_standalone(
        db_path: &std::path::Path,
        entry: &ToolInvocation<'_>,
    ) -> Result<(), StorageError> {
        let mut connection = Connection::open(db_path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tool_log_insert(&tx, entry)?;
        tx.commit()?;
        Ok(())
    }

    /// The ring, oldest first (diagnostics + tests; no host command yet).
    pub fn tool_log_recent(&self) -> Result<Vec<ToolInvocationRow>, StorageError> {
        let mut query = self.0.prepare(
            "SELECT seq,request_id,round,tool,arguments,outcome,result_chars,
             result_excerpt,duration_ms,occurred_at
             FROM tool_invocation_log ORDER BY seq",
        )?;
        let rows = query.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, i64>(9)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                seq,
                request_id,
                round,
                tool,
                arguments,
                outcome,
                result_chars,
                result_excerpt,
                duration_ms,
                occurred_at,
            ) = row?;
            out.push(ToolInvocationRow {
                seq,
                request_id,
                round,
                tool,
                arguments,
                outcome: ToolOutcome::from_str(&outcome),
                result_chars,
                result_excerpt,
                duration_ms,
                occurred_at,
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
            &std::env::temp_dir().join(format!("qiban-local-search-{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap()
    }

    #[test]
    fn roots_roundtrip_unique_and_remove() {
        let mut s = store();
        let first = s
            .authorized_root_add(&AuthorizedRootInsert {
                path: "D:\\Projects",
                label: Some("项目"),
            })
            .unwrap();
        assert_eq!(first.path, "D:\\Projects");
        assert_eq!(s.authorized_roots().unwrap().len(), 1);
        // Same canonical path twice is a client bug: UNIQUE rejects it.
        assert!(s
            .authorized_root_add(&AuthorizedRootInsert {
                path: "D:\\Projects",
                label: None
            })
            .is_err());
        s.authorized_root_remove(first.id).unwrap();
        assert!(s.authorized_roots().unwrap().is_empty());
        // Removing again is idempotent (no row matched).
        s.authorized_root_remove(first.id).unwrap();
    }

    #[test]
    fn tool_log_roundtrips_excerpt_and_outcomes_and_trims() {
        let mut s = store();
        for round in 0..2 {
            s.tool_log_append(&ToolInvocation {
                request_id: "req-1",
                round,
                tool: "search_names",
                arguments: "{\"pattern\":\"quant\"}",
                outcome: ToolOutcome::Ok,
                result_chars: 64,
                result_excerpt: Some("{\"matches\":[\"D:\\\\Projects\\\\quant\"]}"),
                duration_ms: 12,
            })
            .unwrap();
        }
        s.tool_log_append(&ToolInvocation {
            request_id: "req-1",
            round: 2,
            tool: "read_slice",
            arguments: "{\"path\":\"C:\\\\Windows\\\\win.ini\"}",
            outcome: ToolOutcome::Denied,
            result_chars: 0,
            result_excerpt: None,
            duration_ms: 0,
        })
        .unwrap();
        let rows = s.tool_log_recent().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].outcome, ToolOutcome::Ok);
        assert_eq!(
            rows[0].result_excerpt.as_deref(),
            Some("{\"matches\":[\"D:\\\\Projects\\\\quant\"]}")
        );
        assert_eq!(rows[1].round, 1);
        assert_eq!(rows[2].outcome, ToolOutcome::Denied);
        assert_eq!(rows[2].result_excerpt, None);
        // Ring trim.
        for n in 0..(TOOL_LOG_RING + 10) {
            s.tool_log_append(&ToolInvocation {
                request_id: &format!("req-r{n}"),
                round: 0,
                tool: "list_roots",
                arguments: "{}",
                outcome: ToolOutcome::Ok,
                result_chars: 1,
                result_excerpt: Some("{}"),
                duration_ms: 0,
            })
            .unwrap();
        }
        assert_eq!(s.tool_log_recent().unwrap().len() as i64, TOOL_LOG_RING);
    }

    #[test]
    fn clear_deletes_tool_log_but_keeps_roots() {
        let mut s = store();
        s.authorized_root_add(&AuthorizedRootInsert {
            path: "D:\\Projects",
            label: None,
        })
        .unwrap();
        s.tool_log_append(&ToolInvocation {
            request_id: "req-1",
            round: 0,
            tool: "list_roots",
            arguments: "{}",
            outcome: ToolOutcome::Ok,
            result_chars: 1,
            result_excerpt: Some("{}"),
            duration_ms: 0,
        })
        .unwrap();
        s.clear().unwrap();
        assert!(s.tool_log_recent().unwrap().is_empty());
        assert_eq!(
            s.authorized_roots().unwrap().len(),
            1,
            "standing consent survives chat clear"
        );
    }
}

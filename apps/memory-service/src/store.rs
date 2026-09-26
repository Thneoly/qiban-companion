//! One personal memory archive with lifecycle: supersession chains, validity
//! windows and a monotonic change sequence for remote-sync reads. SQLite is
//! owned by this service; callers never choose paths or execute SQL.

use crate::errors::{MemoryError, MAX_COUNTER};
use rusqlite::{params, Connection, TransactionBehavior};
use std::{path::Path, sync::Mutex, time::Duration};

/// SQL expression for the v1-compatible UTC timestamp format. String
/// comparison against `datetime('now')` keeps the Python semantics intact.
const SQL_NOW: &str = "strftime('%Y-%m-%d %H:%M:%S','now')";

/// Columns of the Python-era v1 `memories` table; anything else fails closed.
const V1_COLUMNS: [&str; 12] = [
    "id",
    "type",
    "project",
    "title",
    "content",
    "importance",
    "created_at",
    "updated_at",
    "valid_until",
    "superseded_by",
    "contradicts",
    "tags",
];

const V2_COLUMNS: [&str; 14] = [
    "id",
    "seq",
    "type",
    "project",
    "title",
    "content",
    "importance",
    "created_at",
    "updated_at",
    "valid_until",
    "superseded_by",
    "contradicts",
    "tags",
    "origin",
];

const KINDS: [&str; 7] = [
    "fact",
    "decision",
    "preference",
    "project",
    "person",
    "insight",
    "context",
];

const SELECT_COLUMNS: &str = "id,seq,type,project,title,content,importance,created_at,updated_at,valid_until,superseded_by,contradicts,tags,origin";

/// Memory category. `type` is a Rust keyword, so the field is named `kind`
/// and renamed on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    Fact,
    Decision,
    Preference,
    Project,
    Person,
    Insight,
    Context,
}

impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Fact => "fact",
            MemoryKind::Decision => "decision",
            MemoryKind::Preference => "preference",
            MemoryKind::Project => "project",
            MemoryKind::Person => "person",
            MemoryKind::Insight => "insight",
            MemoryKind::Context => "context",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "fact" => MemoryKind::Fact,
            "decision" => MemoryKind::Decision,
            "preference" => MemoryKind::Preference,
            "project" => MemoryKind::Project,
            "person" => MemoryKind::Person,
            "insight" => MemoryKind::Insight,
            "context" => MemoryKind::Context,
            _ => return None,
        })
    }
}

/// A new memory as submitted by a caller, validated before storage.
#[derive(Debug, Clone)]
pub struct NewMemory {
    pub kind: MemoryKind,
    pub title: String,
    pub content: String,
    pub project: Option<String>,
    pub importance: i64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RecallFilter {
    pub query: Option<String>,
    pub project: Option<String>,
    pub kind: Option<MemoryKind>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct MemoryRecord {
    pub id: i64,
    pub seq: i64,
    pub kind: MemoryKind,
    pub project: Option<String>,
    pub title: String,
    pub content: String,
    pub importance: i64,
    pub created_at: String,
    pub updated_at: String,
    pub valid_until: Option<String>,
    pub superseded_by: Option<i64>,
    pub contradicts: Option<i64>,
    pub tags: Vec<String>,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub total: i64,
    pub active: i64,
    pub superseded: i64,
    pub by_type: Vec<(String, i64)>,
    pub by_project: Vec<(String, i64)>,
}

#[derive(Debug)]
pub struct MemoryStore(Mutex<Connection>, String);

impl MemoryStore {
    /// A caught panic rolls the in-flight transaction back during unwind
    /// (rusqlite `Transaction::drop` issues ROLLBACK), so the connection is
    /// consistent; recovering from a poisoned mutex keeps serving instead of
    /// turning every later call into a panic loop.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Opens (and if needed migrates or creates) the archive at `path`.
    /// `origin` stamps every write made through this handle ("mcp"/"http").
    pub fn open(path: &Path, origin: &str) -> Result<Self, MemoryError> {
        validate_origin(origin)?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "secure_delete", "ON")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 2 {
            return Err(MemoryError::NewerSchema);
        }
        let integrity: String = tx.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(MemoryError::Unavailable);
        }
        match version {
            2 => validate_schema(&tx)?,
            0 => match detect_layout(&tx)? {
                Layout::Fresh => {
                    tx.execute_batch(include_str!("../schema-v2.sql"))?;
                    validate_schema(&tx)?;
                }
                Layout::PythonV1 => {
                    // Validate before backing up: a malformed archive must
                    // fail closed without leaving any new files behind.
                    let rows = read_and_validate_v1(&tx)?;
                    backup_v1(path)?;
                    rebuild_v1(&tx, rows)?;
                }
            },
            // No v1 ever stamped user_version; anything claiming 1 is unknown.
            _ => return Err(MemoryError::IncompatibleSchema),
        }
        tx.commit()?;
        // WAL cannot be enabled inside a transaction. This is a deliberate,
        // documented deviation from the desktop stores: the MCP child and the
        // HTTP daemon are two writer processes on one file.
        connection.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0))?;
        Ok(Self(Mutex::new(connection), origin.to_string()))
    }

    pub fn remember(&self, draft: &NewMemory) -> Result<i64, MemoryError> {
        let draft = normalize_draft(draft)?;
        let mut guard = self.lock();
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let seq = allocate_seq(&tx)?;
        let sql = format!(
            "INSERT INTO memories(seq,type,project,title,content,importance,created_at,updated_at,valid_until,tags,origin) \
             VALUES(?1,?2,?3,?4,?5,?6,{SQL_NOW},{SQL_NOW},NULL,?7,?8)"
        );
        tx.execute(
            &sql,
            params![
                seq,
                draft.kind.as_str(),
                draft.project,
                draft.title,
                draft.content,
                draft.importance,
                draft.tags.join(","),
                self.1
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    pub fn recall(&self, filter: &RecallFilter) -> Result<Vec<MemoryRecord>, MemoryError> {
        let limit = filter.limit.unwrap_or(20).clamp(1, 50);
        let mut sql = format!(
            "SELECT {SELECT_COLUMNS} FROM memories \
             WHERE superseded_by IS NULL AND (valid_until IS NULL OR valid_until > {SQL_NOW})"
        );
        let mut values: Vec<String> = Vec::new();
        if let Some(project) = filter.project.as_deref().filter(|p| !p.trim().is_empty()) {
            sql.push_str(" AND (project = ? OR project IS NULL)");
            values.push(project.trim().to_string());
        }
        if let Some(kind) = filter.kind {
            sql.push_str(" AND type = ?");
            values.push(kind.as_str().to_string());
        }
        if let Some(query) = filter.query.as_deref().filter(|q| !q.is_empty()) {
            // LIKE keeps the Python semantics: % and _ are not escaped.
            sql.push_str(" AND (title LIKE ? OR content LIKE ? OR tags LIKE ?)");
            let pattern = format!("%{query}%");
            values.extend([pattern.clone(), pattern.clone(), pattern]);
        }
        // The limit is an i64 clamped above, so inlining it is injection-safe
        // and keeps the bound parameters textual.
        sql.push_str(&format!(
            " ORDER BY importance DESC, updated_at DESC, id ASC LIMIT {limit}"
        ));
        let guard = self.lock();
        let mut statement = guard.prepare(&sql)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(values), record_mapper)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn supersede(
        &self,
        old_id: i64,
        title: &str,
        content: &str,
    ) -> Result<(i64, i64), MemoryError> {
        let title = normalize_title(title)?;
        let content = normalize_content(content)?;
        let mut guard = self.lock();
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(Option<String>, Option<i64>, i64, String)> = tx
            .query_row(
                "SELECT project,superseded_by,importance,tags FROM memories WHERE id=?1",
                [old_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        let Some((project, already, importance, tags)) = old else {
            return Err(MemoryError::NotFound);
        };
        if already.is_some() {
            // The Python service allowed forking a chain by superseding twice;
            // we refuse to corrupt the lineage instead.
            return Err(MemoryError::Validation("该记忆已被取代，不能再次取代"));
        }
        let new_seq = allocate_seq(&tx)?;
        let sql = format!(
            "INSERT INTO memories(seq,type,project,title,content,importance,created_at,updated_at,tags,origin) \
             VALUES(?1,(SELECT type FROM memories WHERE id=?2),?3,?4,?5,?6,{SQL_NOW},{SQL_NOW},?7,?8)"
        );
        tx.execute(
            &sql,
            params![new_seq, old_id, project, title, content, importance, tags, self.1],
        )?;
        let new_id = tx.last_insert_rowid();
        let old_seq = allocate_seq(&tx)?;
        let sql = format!(
            "UPDATE memories SET superseded_by=?1, updated_at={SQL_NOW}, seq=?2 WHERE id=?3"
        );
        tx.execute(&sql, params![new_id, old_seq, old_id])?;
        tx.commit()?;
        Ok((old_id, new_id))
    }

    pub fn forget(&self, id: i64) -> Result<bool, MemoryError> {
        let mut guard = self.lock();
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sql = format!(
            "SELECT (valid_until IS NULL OR valid_until < {SQL_NOW}) FROM memories WHERE id=?1"
        );
        let active: Option<bool> =
            tx.query_row(&sql, [id], |r| r.get(0))
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
        let Some(active) = active else {
            return Err(MemoryError::NotFound);
        };
        if active {
            let seq = allocate_seq(&tx)?;
            let sql = format!(
                "UPDATE memories SET valid_until={SQL_NOW}, updated_at={SQL_NOW}, seq=?1 WHERE id=?2"
            );
            tx.execute(&sql, params![seq, id])?;
        }
        // Already-expired rows are an idempotent no-op.
        tx.commit()?;
        Ok(true)
    }

    pub fn update(&self, id: i64, content: &str) -> Result<(), MemoryError> {
        let content = normalize_content(content)?;
        let mut guard = self.lock();
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let seq = allocate_seq(&tx)?;
        let sql =
            format!("UPDATE memories SET content=?1, updated_at={SQL_NOW}, seq=?2 WHERE id=?3");
        let changed = tx.execute(&sql, params![content, seq, id])?;
        if changed == 0 {
            // The Python service reported success for unknown ids; we refuse
            // to fake it.
            return Err(MemoryError::NotFound);
        }
        tx.commit()?;
        Ok(())
    }

    /// All counters come from one read snapshot; in WAL mode separate
    /// autocommit statements could straddle another process's commit and
    /// produce e.g. active + superseded > total.
    pub fn stats(&self) -> Result<Stats, MemoryError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let total: i64 = tx.query_row("SELECT count(*) FROM memories", [], |r| r.get(0))?;
        let active: i64 = tx.query_row(
            &format!(
                "SELECT count(*) FROM memories WHERE superseded_by IS NULL \
                 AND (valid_until IS NULL OR valid_until > {SQL_NOW})"
            ),
            [],
            |r| r.get(0),
        )?;
        let superseded: i64 = tx.query_row(
            "SELECT count(*) FROM memories WHERE superseded_by IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        let by_type = pair_list(
            &tx,
            &format!(
                "SELECT type, count(*) FROM memories WHERE superseded_by IS NULL \
                 AND (valid_until IS NULL OR valid_until > {SQL_NOW}) GROUP BY type ORDER BY type"
            ),
        )?;
        let by_project = pair_list(
            &tx,
            &format!(
                "SELECT coalesce(project,'(global)'), count(*) FROM memories \
                 WHERE superseded_by IS NULL AND (valid_until IS NULL OR valid_until > {SQL_NOW}) \
                 GROUP BY project ORDER BY count(*) DESC, coalesce(project,'(global)') LIMIT 10"
            ),
        )?;
        tx.commit()?;
        Ok(Stats {
            total,
            active,
            superseded,
            by_type,
            by_project,
        })
    }

    pub fn get(&self, id: i64) -> Result<Option<MemoryRecord>, MemoryError> {
        let guard = self.lock();
        fetch_one(&guard, "WHERE id=?1", params![id])
    }

    /// Single-row fetch plus its full supersession line, from one snapshot so
    /// the memory and its chain cannot disagree mid-supersede.
    pub fn get_with_chain(
        &self,
        id: i64,
    ) -> Result<Option<(MemoryRecord, Vec<MemoryRecord>)>, MemoryError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let Some(start) = fetch_one(&tx, "WHERE id=?1", params![id])? else {
            return Ok(None);
        };
        let line = walk_chain(&tx, start.clone())?;
        tx.commit()?;
        Ok(Some((start, line)))
    }

    /// The full supersession line: oldest ancestor through every successor.
    pub fn chain(&self, id: i64) -> Result<Vec<MemoryRecord>, MemoryError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let Some(start) = fetch_one(&tx, "WHERE id=?1", params![id])? else {
            return Err(MemoryError::NotFound);
        };
        let line = walk_chain(&tx, start)?;
        tx.commit()?;
        Ok(line)
    }

    /// Rows changed after `since`, plus the sync cursor to continue from.
    /// Soft deletion means every change is visible as a row state; no
    /// tombstones. The cursor is the highest seq inside the returned page
    /// (the global max when the page is empty) read in the SAME snapshot:
    /// advancing `since` to it can never skip a row, even when the page was
    /// truncated by the limit or another process commits concurrently.
    pub fn changes_since(
        &self,
        since: i64,
        limit: i64,
    ) -> Result<(Vec<MemoryRecord>, i64), MemoryError> {
        if since < 0 {
            return Err(MemoryError::Validation("since 不能为负"));
        }
        let limit = limit.clamp(1, 1000);
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM memories WHERE seq > ?1 ORDER BY seq ASC LIMIT {limit}"
        );
        let mut statement = tx.prepare(&sql)?;
        let rows = statement
            .query_map(params![since], record_mapper)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let cursor = match rows.last() {
            Some(last) => last.seq,
            None => tx.query_row("SELECT coalesce(max(seq),0) FROM memories", [], |r| {
                r.get(0)
            })?,
        };
        tx.commit()?;
        Ok((rows, cursor))
    }

    /// Deterministic, non-generative summary: the top active entries of the
    /// persona-bearing kinds, in a fixed order, from one read snapshot.
    pub fn personality_summary(
        &self,
    ) -> Result<Vec<(&'static str, Vec<MemoryRecord>)>, MemoryError> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let mut sections = Vec::new();
        for kind in [
            MemoryKind::Preference,
            MemoryKind::Insight,
            MemoryKind::Person,
        ] {
            let sql = format!(
                "SELECT {SELECT_COLUMNS} FROM memories \
                 WHERE superseded_by IS NULL AND (valid_until IS NULL OR valid_until > {SQL_NOW}) \
                 AND type=?1 ORDER BY importance DESC, updated_at DESC, id ASC LIMIT 3"
            );
            let mut statement = tx.prepare(&sql)?;
            let rows = statement
                .query_map(params![kind.as_str()], record_mapper)?
                .collect::<Result<Vec<_>, _>>()?;
            sections.push((kind.as_str(), rows));
        }
        tx.commit()?;
        Ok(sections)
    }

    /// Journal mode as stored on disk, for tests and diagnostics.
    pub fn journal_mode(&self) -> Result<String, MemoryError> {
        let guard = self.lock();
        Ok(guard.query_row("PRAGMA journal_mode", [], |r| r.get(0))?)
    }
}

enum Layout {
    Fresh,
    PythonV1,
}

fn detect_layout(tx: &rusqlite::Transaction<'_>) -> Result<Layout, MemoryError> {
    let mut statement = tx.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let tables = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if tables.is_empty() {
        return Ok(Layout::Fresh);
    }
    if tables.len() != 1 || tables[0] != "memories" {
        return Err(MemoryError::IncompatibleSchema);
    }
    let mut statement = tx.prepare("PRAGMA table_info(memories)")?;
    let columns = statement
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if columns.len() != V1_COLUMNS.len()
        || !columns.iter().all(|c| V1_COLUMNS.contains(&c.as_str()))
    {
        return Err(MemoryError::IncompatibleSchema);
    }
    Ok(Layout::PythonV1)
}

/// File-level snapshot before the one-way migration. Only created once so a
/// crashed retry never overwrites the first backup. The v1 layout uses
/// journal_mode=delete, so a plain copy of the main file is complete.
fn backup_v1(path: &Path) -> Result<(), MemoryError> {
    let backup = path.with_extension("db.v1.bak");
    if backup.exists() {
        return Ok(());
    }
    let temp = path.with_extension("db.v1.bak.tmp");
    std::fs::copy(path, &temp)?;
    std::fs::rename(&temp, &backup)?;
    Ok(())
}

struct V1Row {
    id: i64,
    kind: String,
    project: Option<String>,
    title: String,
    content: String,
    importance: Option<f64>,
    created_at: Option<String>,
    updated_at: Option<String>,
    valid_until: Option<String>,
    superseded_by: Option<i64>,
    contradicts: Option<i64>,
    tags: Option<String>,
}

/// Mirrors the schema-v2 GLOB exactly (`YYYY-MM-DD HH:MM:SS` with the loose
/// digit classes the CHECK uses), so pre-validation never rejects a row the
/// rebuild would accept.
fn matches_timestamp_glob(value: &str) -> bool {
    let bytes: Vec<char> = value.chars().collect();
    if bytes.len() != 19 {
        return false;
    }
    let digit = |c: char| c.is_ascii_digit();
    let class = |c: char, low: char| digit(c) && c <= low;
    digit(bytes[0])
        && digit(bytes[1])
        && digit(bytes[2])
        && digit(bytes[3])
        && bytes[4] == '-'
        && class(bytes[5], '1')
        && digit(bytes[6])
        && bytes[7] == '-'
        && class(bytes[8], '3')
        && digit(bytes[9])
        && bytes[10] == ' '
        && class(bytes[11], '2')
        && digit(bytes[12])
        && bytes[13] == ':'
        && class(bytes[14], '5')
        && digit(bytes[15])
        && bytes[16] == ':'
        && class(bytes[17], '5')
        && digit(bytes[18])
}

/// Reads and validates every v1 row before anything is dropped; a malformed
/// table must fail closed with the file (and its directory) untouched.
/// Validation covers the FULL v2 CHECK surface, because the Python service
/// wrote without server-side validation: oversized titles/content, free-form
/// `valid_until`, REAL importance or self-references would otherwise abort
/// the rebuild after the backup was already created.
fn read_and_validate_v1(tx: &rusqlite::Transaction<'_>) -> Result<Vec<V1Row>, MemoryError> {
    let mut statement = tx.prepare(
        "SELECT id,type,project,title,content,importance,created_at,updated_at,valid_until,superseded_by,contradicts,tags FROM memories ORDER BY id",
    )?;
    let rows = statement
        .query_map([], |r| {
            Ok(V1Row {
                id: r.get(0)?,
                kind: r.get(1)?,
                project: r.get(2)?,
                title: r.get(3)?,
                content: r.get(4)?,
                importance: r.get(5)?,
                created_at: r.get(6)?,
                updated_at: r.get(7)?,
                valid_until: r.get(8)?,
                superseded_by: r.get(9)?,
                contradicts: r.get(10)?,
                tags: r.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let ids: std::collections::HashSet<i64> = rows.iter().map(|r| r.id).collect();
    for row in &rows {
        if !KINDS.contains(&row.kind.as_str()) {
            return Err(MemoryError::IncompatibleSchema);
        }
        if !(1..=200).contains(&row.title.chars().count()) {
            return Err(MemoryError::IncompatibleSchema);
        }
        if !(1..=20_000).contains(&row.content.chars().count()) {
            return Err(MemoryError::IncompatibleSchema);
        }
        // JSON 4.5 stores as REAL in an INTEGER-affinity column; only
        // integral values in range can be carried into v2.
        if let Some(importance) = row.importance {
            if importance.fract() != 0.0 || !(1.0..=5.0).contains(&importance) {
                return Err(MemoryError::IncompatibleSchema);
            }
        }
        for stamp in [row.created_at.as_deref(), row.updated_at.as_deref()]
            .into_iter()
            .flatten()
        {
            if !matches_timestamp_glob(stamp) {
                return Err(MemoryError::IncompatibleSchema);
            }
        }
        if let Some(valid_until) = row.valid_until.as_deref() {
            if !matches_timestamp_glob(valid_until) {
                return Err(MemoryError::IncompatibleSchema);
            }
        }
        for id in [row.superseded_by, row.contradicts].into_iter().flatten() {
            if id == row.id || !ids.contains(&id) {
                return Err(MemoryError::IncompatibleSchema);
            }
        }
    }
    Ok(rows)
}

fn rebuild_v1(tx: &rusqlite::Transaction<'_>, rows: Vec<V1Row>) -> Result<(), MemoryError> {
    // Read BEFORE the drop: DROP TABLE also deletes the sqlite_sequence row,
    // and AUTOINCREMENT's never-reuse promise is defined by that bottom line.
    let sequence_bottom = tx
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='memories'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .ok();
    // Old rows may reference successors inserted later in this same batch;
    // immediate foreign keys would abort mid-transaction.
    tx.execute_batch("PRAGMA defer_foreign_keys=ON")?;
    tx.execute("DROP TABLE memories", [])?;
    tx.execute_batch(include_str!("../schema-v2.sql"))?;
    let sql = format!(
        "INSERT INTO memories(id,seq,type,project,title,content,importance,created_at,updated_at,valid_until,superseded_by,contradicts,tags,origin) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,coalesce(?8,{SQL_NOW}),coalesce(?9,{SQL_NOW}),?10,?11,?12,coalesce(?13,''),NULL)"
    );
    let mut statement = tx.prepare(&sql)?;
    for (index, row) in rows.iter().enumerate() {
        let seq = i64::try_from(index).expect("row count fits i64") + 1;
        statement.execute(params![
            row.id,
            seq,
            row.kind,
            row.project,
            row.title,
            row.content,
            row.importance.map(|value| value as i64).unwrap_or(3),
            row.created_at,
            row.updated_at,
            row.valid_until,
            row.superseded_by,
            row.contradicts,
            row.tags,
        ])?;
    }
    drop(statement);
    // AUTOINCREMENT never reuses ids; restoring the pre-migration bottom
    // line keeps that promise even when the highest rows were deleted from
    // the v1 archive before migrating.
    if let Some(bottom) = sequence_bottom {
        let max_id = rows.last().map(|row| row.id).unwrap_or(0);
        if bottom > max_id {
            tx.execute(
                "UPDATE sqlite_sequence SET seq=?1 WHERE name='memories'",
                [bottom],
            )?;
        }
    }
    let next = i64::try_from(rows.len()).expect("row count fits i64") + 1;
    if next > MAX_COUNTER {
        return Err(MemoryError::CounterOverflow);
    }
    tx.execute(
        "UPDATE memory_meta SET next_seq=?1 WHERE singleton=1",
        [next],
    )?;
    let mut violations = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = violations.query([])?;
    let mut broken = 0usize;
    while rows.next()?.is_some() {
        broken += 1;
    }
    if broken > 0 {
        return Err(MemoryError::IncompatibleSchema);
    }
    Ok(())
}

fn validate_schema(tx: &rusqlite::Transaction<'_>) -> Result<(), MemoryError> {
    let mut statement = tx.prepare("PRAGMA table_info(memories)")?;
    let columns = statement
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    if columns.len() != V2_COLUMNS.len()
        || !columns.iter().all(|c| V2_COLUMNS.contains(&c.as_str()))
    {
        return Err(MemoryError::IncompatibleSchema);
    }
    let next_seq: i64 = tx.query_row(
        "SELECT next_seq FROM memory_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    if !(1..=MAX_COUNTER).contains(&next_seq) {
        return Err(MemoryError::IncompatibleSchema);
    }
    Ok(())
}

fn allocate_seq(tx: &rusqlite::Transaction<'_>) -> Result<i64, MemoryError> {
    let current: i64 = tx.query_row(
        "SELECT next_seq FROM memory_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    // `current` is the value stamped on a row, so it must respect the same
    // JS-safe ceiling the schema enforces; reserving the last value would
    // push next_seq past the CHECK bound.
    if current >= MAX_COUNTER {
        return Err(MemoryError::CounterOverflow);
    }
    let next = current.checked_add(1).ok_or(MemoryError::CounterOverflow)?;
    tx.execute(
        "UPDATE memory_meta SET next_seq=?1 WHERE singleton=1",
        [next],
    )?;
    Ok(current)
}

fn validate_origin(origin: &str) -> Result<(), MemoryError> {
    let valid = (1..=32).contains(&origin.len())
        && origin
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(MemoryError::Validation(
            "origin 必须是 1～32 位小写字母、数字、下划线或连字符",
        ))
    }
}

struct NormalizedDraft {
    kind: MemoryKind,
    title: String,
    content: String,
    project: Option<String>,
    importance: i64,
    tags: Vec<String>,
}

fn normalize_draft(draft: &NewMemory) -> Result<NormalizedDraft, MemoryError> {
    Ok(NormalizedDraft {
        kind: draft.kind,
        title: normalize_title(&draft.title)?,
        content: normalize_content(&draft.content)?,
        project: match draft.project.as_deref() {
            None => None,
            Some(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else if trimmed.chars().count() > 64 {
                    return Err(MemoryError::Validation("project 最长 64 个字符"));
                } else {
                    Some(trimmed.to_string())
                }
            }
        },
        importance: if (1..=5).contains(&draft.importance) {
            draft.importance
        } else {
            // Out-of-range importance is a caller error (the Python service
            // failed it via the schema CHECK); silently rewriting the value
            // would fake success.
            return Err(MemoryError::Validation("importance 须在 1～5 之间"));
        },
        tags: normalize_tags(&draft.tags)?,
    })
}

fn normalize_title(title: &str) -> Result<String, MemoryError> {
    let trimmed = title.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 200 {
        return Err(MemoryError::Validation("标题需为 1～200 个字符"));
    }
    if trimmed.contains('\0') {
        return Err(MemoryError::Validation("标题包含非法字符"));
    }
    Ok(trimmed.to_string())
}

fn normalize_content(content: &str) -> Result<String, MemoryError> {
    if content.trim().is_empty() || content.chars().count() > 20_000 {
        return Err(MemoryError::Validation("内容需为 1～20000 个字符"));
    }
    if content.contains('\0') {
        return Err(MemoryError::Validation("内容包含非法字符"));
    }
    Ok(content.to_string())
}

fn normalize_tags(tags: &[String]) -> Result<Vec<String>, MemoryError> {
    if tags.len() > 8 {
        return Err(MemoryError::Validation("最多 8 个标签"));
    }
    let mut cleaned = Vec::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains(',') || trimmed.chars().count() > 32 {
            return Err(MemoryError::Validation("标签不能含逗号且最长 32 个字符"));
        }
        cleaned.push(trimmed.to_string());
    }
    Ok(cleaned)
}

fn record_mapper(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    Ok(MemoryRecord {
        id: row.get(0)?,
        seq: row.get(1)?,
        kind: MemoryKind::parse(&row.get::<_, String>(2)?).ok_or(
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                "unknown memory kind".into(),
            ),
        )?,
        project: row.get(3)?,
        title: row.get(4)?,
        content: row.get(5)?,
        importance: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        valid_until: row.get(9)?,
        superseded_by: row.get(10)?,
        contradicts: row.get(11)?,
        tags: row
            .get::<_, String>(12)?
            .split(',')
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .collect(),
        origin: row.get(13)?,
    })
}

/// Walks one supersession line inside an open read snapshot: up to the
/// oldest ancestor (min id wins if legacy Python data ever forked a chain),
/// then down through every successor.
fn walk_chain(
    tx: &rusqlite::Transaction<'_>,
    start: MemoryRecord,
) -> Result<Vec<MemoryRecord>, MemoryError> {
    let mut root = start;
    for _ in 0..10_000 {
        // Aggregate always yields one row; NULL means no predecessor.
        let parent_id: Option<i64> = tx.query_row(
            "SELECT min(id) FROM memories WHERE superseded_by=?1",
            [root.id],
            |r| r.get(0),
        )?;
        let Some(parent_id) = parent_id else {
            break;
        };
        root = fetch_one(tx, "WHERE id=?1", params![parent_id])?.ok_or(MemoryError::Unavailable)?;
    }
    let mut line = vec![root.clone()];
    let mut current = root;
    for _ in 0..10_000 {
        let Some(next_id) = current.superseded_by else {
            break;
        };
        current =
            fetch_one(tx, "WHERE id=?1", params![next_id])?.ok_or(MemoryError::Unavailable)?;
        line.push(current.clone());
    }
    Ok(line)
}

fn fetch_one(
    connection: &Connection,
    suffix: &str,
    params: impl rusqlite::Params,
) -> Result<Option<MemoryRecord>, MemoryError> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM memories {suffix}");
    connection
        .query_row(&sql, params, record_mapper)
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other.into()),
        })
}

fn pair_list(connection: &Connection, sql: &str) -> Result<Vec<(String, i64)>, MemoryError> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// An in-memory v2 archive, for tests that do not need a file.
    pub(crate) fn memory_store(origin: &str) -> MemoryStore {
        let connection = Connection::open_in_memory().expect("in-memory sqlite");
        connection
            .execute_batch(include_str!("../schema-v2.sql"))
            .expect("v2 schema");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("foreign keys");
        MemoryStore(Mutex::new(connection), origin.to_string())
    }

    /// A unique temp file for tests that need a real on-disk database.
    pub(crate) fn temp_db(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("{name}-{}.db", uuid::Uuid::new_v4()));
        path
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

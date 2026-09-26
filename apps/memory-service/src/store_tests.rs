use super::test_support::{memory_store, temp_db};
use super::*;
use crate::errors::{MemoryError, MAX_COUNTER};
use rusqlite::{params, Connection};

fn draft(kind: MemoryKind, title: &str, content: &str) -> NewMemory {
    NewMemory {
        kind,
        title: title.to_string(),
        content: content.to_string(),
        project: None,
        importance: 3,
        tags: Vec::new(),
    }
}

#[test]
fn remember_stamps_defaults_and_origin() {
    let store = memory_store("mcp");
    let id = store
        .remember(&NewMemory {
            kind: MemoryKind::Decision,
            title: "  投 JAAMAS  ".to_string(),
            content: "论文投递决定。".to_string(),
            project: Some("  ".to_string()),
            importance: 0,
            tags: vec![" 论文 ".to_string()],
        })
        .unwrap();
    let record = store.get(id).unwrap().expect("row exists");
    assert_eq!(record.title, "投 JAAMAS");
    assert_eq!(record.project, None, "blank project normalizes to global");
    assert_eq!(record.importance, 3);
    assert_eq!(record.tags, vec!["论文".to_string()]);
    assert_eq!(record.origin.as_deref(), Some("mcp"));
    assert_eq!(record.seq, 1);
    assert!(record.created_at.starts_with("20"));
    assert_eq!(record.created_at, record.updated_at);
}

#[test]
fn recall_filters_match_python_semantics() {
    let store = memory_store("mcp");
    let global = store
        .remember(&draft(MemoryKind::Preference, "全局偏好", "偏好A"))
        .unwrap();
    let project = store
        .remember(&NewMemory {
            kind: MemoryKind::Project,
            title: "R2R 状态".to_string(),
            content: "E13 已完成。".to_string(),
            project: Some("R2R".to_string()),
            importance: 5,
            tags: vec!["研究".to_string()],
        })
        .unwrap();

    // A project filter still sees global memories, like the Python service.
    let in_project = store
        .recall(&RecallFilter {
            project: Some("R2R".to_string()),
            ..RecallFilter::default()
        })
        .unwrap();
    assert_eq!(in_project.len(), 2);

    let by_kind = store
        .recall(&RecallFilter {
            kind: Some(MemoryKind::Project),
            ..RecallFilter::default()
        })
        .unwrap();
    assert_eq!(by_kind.len(), 1);
    assert_eq!(by_kind[0].id, project);

    let by_query = store
        .recall(&RecallFilter {
            query: Some("E13".to_string()),
            ..RecallFilter::default()
        })
        .unwrap();
    assert_eq!(by_query.len(), 1);
    assert_eq!(by_query[0].id, project);

    let by_tag = store
        .recall(&RecallFilter {
            query: Some("研究".to_string()),
            ..RecallFilter::default()
        })
        .unwrap();
    assert_eq!(by_tag.len(), 1);

    // Importance desc, then updated_at desc, then id asc for determinism.
    assert_eq!(by_query[0].importance, 5);
    let _ = global;

    // Limits clamp to 1..=50: negative no longer means "unlimited" (the
    // Python service would dump the whole archive), oversized caps at 50.
    let floor = store
        .recall(&RecallFilter {
            limit: Some(-5),
            ..RecallFilter::default()
        })
        .unwrap();
    assert_eq!(floor.len(), 1);
    let ceiling = store
        .recall(&RecallFilter {
            limit: Some(99),
            ..RecallFilter::default()
        })
        .unwrap();
    assert_eq!(ceiling.len(), 2);
}

#[test]
fn supersede_moves_the_chain_and_marks_both_seqs() {
    let store = memory_store("mcp");
    let old_id = store
        .remember(&NewMemory {
            kind: MemoryKind::Decision,
            title: "旧决定".to_string(),
            content: "旧内容".to_string(),
            project: Some("R2R".to_string()),
            importance: 4,
            tags: vec!["决策".to_string()],
        })
        .unwrap();
    let (returned_old, new_id) = store.supersede(old_id, "新决定", "新内容").unwrap();
    assert_eq!(returned_old, old_id);
    assert_ne!(new_id, old_id);

    let old = store.get(old_id).unwrap().unwrap();
    let new = store.get(new_id).unwrap().unwrap();
    assert_eq!(old.superseded_by, Some(new_id));
    assert_eq!(new.superseded_by, None);
    // The successor inherits the lineage's domain fields.
    assert_eq!(new.kind, MemoryKind::Decision);
    assert_eq!(new.project.as_deref(), Some("R2R"));
    assert_eq!(new.importance, 4);
    assert_eq!(new.tags, vec!["决策".to_string()]);
    // Both rows carry fresh sequences: the successor row is inserted first,
    // then the superseded row is re-stamped, so its state change is newest.
    assert!(old.seq > new.seq);

    let visible = store.recall(&RecallFilter::default()).unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, new_id);

    // Superseding a superseded row is refused instead of forking the chain.
    assert!(matches!(
        store.supersede(old_id, "再次取代", "x"),
        Err(MemoryError::Validation(_))
    ));
}

#[test]
fn chain_walks_ancestors_and_successors() {
    let store = memory_store("mcp");
    let first = store
        .remember(&draft(MemoryKind::Fact, "一代", "1"))
        .unwrap();
    let (_, second) = store.supersede(first, "二代", "2").unwrap();
    let (_, third) = store.supersede(second, "三代", "3").unwrap();

    let from_middle = store.chain(second).unwrap();
    let ids: Vec<i64> = from_middle.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![first, second, third]);

    let from_root = store.chain(first).unwrap();
    assert_eq!(from_root.len(), 3);
    let from_leaf = store.chain(third).unwrap();
    assert_eq!(from_leaf.len(), 3);

    assert!(matches!(store.chain(99), Err(MemoryError::NotFound)));
}

#[test]
fn forget_is_idempotent_and_hides_the_row() {
    let store = memory_store("mcp");
    let id = store
        .remember(&draft(MemoryKind::Context, "临时上下文", "很快过期"))
        .unwrap();
    assert!(store.forget(id).unwrap());
    assert_eq!(store.recall(&RecallFilter::default()).unwrap().len(), 0);

    let record = store.get(id).unwrap().expect("row kept as history");
    assert!(record.valid_until.is_some());

    // Second forget is a no-op that still reports success, and a fresh
    // sequence is not consumed.
    let seq_before = record.seq;
    assert!(store.forget(id).unwrap());
    let after = store.get(id).unwrap().unwrap();
    assert_eq!(
        after.seq, seq_before,
        "idempotent forget keeps the seq stable"
    );

    assert!(matches!(store.forget(42), Err(MemoryError::NotFound)));
}

#[test]
fn update_of_unknown_id_fails_instead_of_faking_success() {
    let store = memory_store("mcp");
    assert!(matches!(
        store.update(7, "内容"),
        Err(MemoryError::NotFound)
    ));
    let id = store
        .remember(&draft(MemoryKind::Fact, "标题", "原内容"))
        .unwrap();
    store.update(id, "新内容").unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().content, "新内容");
}

#[test]
fn stats_counts_like_the_python_service() {
    let store = memory_store("mcp");
    let a = store
        .remember(&NewMemory {
            kind: MemoryKind::Preference,
            title: "偏好".to_string(),
            content: "x".to_string(),
            project: None,
            importance: 3,
            tags: vec![],
        })
        .unwrap();
    let b = store
        .remember(&NewMemory {
            kind: MemoryKind::Project,
            title: "项目".to_string(),
            content: "y".to_string(),
            project: Some("R2R".to_string()),
            importance: 3,
            tags: vec![],
        })
        .unwrap();
    store.supersede(a, "偏好2", "z").unwrap();
    store.forget(b).unwrap();
    let stats = store.stats().unwrap();
    assert_eq!(stats.total, 3);
    // The successor of the supersession is still active.
    assert_eq!(stats.active, 1);
    assert_eq!(stats.superseded, 1);
    assert_eq!(stats.by_type, vec![("preference".to_string(), 1)]);
    assert_eq!(stats.by_project, vec![("(global)".to_string(), 1)]);
}

#[test]
fn sequences_are_strictly_monotonic_across_operations() {
    let store = memory_store("http");
    let first = store.remember(&draft(MemoryKind::Fact, "a", "a")).unwrap();
    let second = store.remember(&draft(MemoryKind::Fact, "b", "b")).unwrap();
    let (_, superseding) = store.supersede(first, "a2", "a2").unwrap();
    let seq_of = |id: i64| store.get(id).unwrap().unwrap().seq;
    // Allocation order: insert a (1), insert b (2), insert successor (3),
    // re-stamp superseded row (4). Every write consumed a fresh counter value.
    assert_eq!(seq_of(second), 2);
    assert_eq!(seq_of(superseding), 3);
    assert_eq!(seq_of(first), 4, "the superseded row's change is newest");
    let (rows, current) = store.changes_since(0, 1000).unwrap();
    assert_eq!(current, 4);
    assert_eq!(rows.len(), 3);
}

#[test]
fn changes_since_pages_with_current_seq() {
    let store = memory_store("http");
    let a = store.remember(&draft(MemoryKind::Fact, "a", "a")).unwrap();
    let b = store.remember(&draft(MemoryKind::Fact, "b", "b")).unwrap();
    let (rows, current) = store.changes_since(0, 1000).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(current, 2);

    let (page, _) = store.changes_since(0, 1).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, a);

    store.forget(b).unwrap();
    let (rows, current) = store.changes_since(0, 1000).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(current, 3);
    let forgotten = rows.iter().find(|r| r.id == b).unwrap();
    assert!(
        forgotten.valid_until.is_some(),
        "expiry is a row state, not a tombstone"
    );

    assert!(matches!(
        store.changes_since(-1, 10),
        Err(MemoryError::Validation(_))
    ));
}

#[test]
fn personality_summary_is_deterministic_and_bounded() {
    let store = memory_store("http");
    for index in 0..5 {
        store
            .remember(&NewMemory {
                kind: MemoryKind::Preference,
                title: format!("偏好{index}"),
                content: "内容".to_string(),
                project: None,
                importance: 1 + index % 5,
                tags: vec![],
            })
            .unwrap();
    }
    store
        .remember(&draft(MemoryKind::Fact, "事实不入人格", "x"))
        .unwrap();
    let first = store.personality_summary().unwrap();
    let second = store.personality_summary().unwrap();
    assert_eq!(first.len(), 3);
    assert_eq!(first[0].0, "preference");
    assert_eq!(first[0].1.len(), 3, "bounded to three entries");
    assert_eq!(first[1].0, "insight");
    assert_eq!(first[2].0, "person");
    // Deterministic: same data, same result.
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
}

#[test]
fn counter_exhaustion_refuses_to_write() {
    let store = memory_store("mcp");
    {
        let guard = store.0.lock().unwrap();
        guard
            .execute("UPDATE memory_meta SET next_seq=?1", [MAX_COUNTER])
            .unwrap();
    }
    assert!(matches!(
        store.remember(&draft(MemoryKind::Fact, "溢出", "x")),
        Err(MemoryError::CounterOverflow)
    ));
    assert_eq!(store.recall(&RecallFilter::default()).unwrap().len(), 0);
}

/// (kind, project, title, content, importance) fixture row.
type V1Fixture = (
    &'static str,
    Option<&'static str>,
    &'static str,
    &'static str,
    Option<i64>,
);

/// Builds a Python-era v1 database exactly like server.py did.
fn python_v1(path: &Path, rows: &[V1Fixture]) {
    let connection = Connection::open(path).unwrap();
    connection.execute_batch(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            type TEXT NOT NULL CHECK(type IN ('fact','decision','preference','project','person','insight','context')),
            project TEXT,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            importance INTEGER DEFAULT 3 CHECK(importance BETWEEN 1 AND 5),
            created_at TEXT DEFAULT (datetime('now')),
            updated_at TEXT DEFAULT (datetime('now')),
            valid_until TEXT,
            superseded_by INTEGER REFERENCES memories(id),
            contradicts INTEGER REFERENCES memories(id),
            tags TEXT DEFAULT ''
        );
        CREATE INDEX idx_active ON memories(superseded_by IS NULL, valid_until);
        CREATE INDEX idx_project ON memories(project);
        CREATE INDEX idx_type ON memories(type);",
    )
    .unwrap();
    for (kind, project, title, content, importance) in rows {
        connection
            .execute(
                "INSERT INTO memories(type,project,title,content,importance,valid_until,superseded_by,tags) \
                 VALUES(?1,?2,?3,?4,?5,NULL,NULL,?6)",
                params![kind, project, title, content, importance, ""],
            )
            .unwrap();
    }
    connection.close().unwrap();
}

#[test]
fn fresh_file_creates_v2_directly() {
    let path = temp_db("fresh");
    let store = MemoryStore::open(&path, "mcp").unwrap();
    assert_eq!(store.journal_mode().unwrap(), "wal");
    let connection = Connection::open(&path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
    drop(connection);
    let _ = std::fs::remove_file(path);
}

#[test]
fn python_v1_migrates_in_place_with_backup_and_ids_preserved() {
    let path = temp_db("migrate");
    python_v1(
        &path,
        &[
            (
                "decision",
                Some("R2R"),
                "论文投 JAAMAS",
                "决定投 JAAMAS。",
                Some(5),
            ),
            ("preference", None, "偏好简洁代码", "注释用中文。", None),
            (
                "insight",
                None,
                "R2R 核心洞察",
                "状态化信息的自治理。",
                Some(4),
            ),
            ("fact", None, "测试中文记忆", "测试写入。", Some(1)),
        ],
    );
    // Add a supersession pair: row 4 is replaced by new row 5.
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO memories(type,project,title,content,importance,tags) \
                 VALUES('fact',NULL,'测试中文记忆（修订）','修订后。',1,'')",
                [],
            )
            .unwrap();
        let new_id = connection.last_insert_rowid();
        connection
            .execute("UPDATE memories SET superseded_by=?1 WHERE id=4", [new_id])
            .unwrap();
        connection.close().unwrap();
    }

    let store = MemoryStore::open(&path, "mcp").unwrap();
    let stats = store.stats().unwrap();
    assert_eq!(stats.total, 5);
    assert_eq!(stats.active, 4);
    assert_eq!(stats.superseded, 1);

    let connection = Connection::open(&path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
    let seqs: Vec<i64> = connection
        .prepare("SELECT seq FROM memories ORDER BY seq")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
    drop(connection);

    // AUTOINCREMENT continues after the highest preserved id, so old
    // references never collide.
    let next_id = store
        .remember(&draft(MemoryKind::Fact, "迁移后新增", "x"))
        .unwrap();
    assert_eq!(next_id, 6);

    // The backup snapshot exists next to the archive and is v1-shaped.
    let backup = path.with_extension("db.v1.bak");
    assert!(backup.exists());
    let backup_connection = Connection::open(&backup).unwrap();
    let columns = backup_connection
        .prepare("PRAGMA table_info(memories)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(columns.len(), 12);

    // Reopening neither re-migrates nor overwrites the first backup.
    let reopened = MemoryStore::open(&path, "http").unwrap();
    assert_eq!(reopened.stats().unwrap().total, 6);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&backup);
}

#[test]
fn malformed_v1_fails_closed_without_touching_the_file() {
    let path = temp_db("malformed");
    python_v1(
        &path,
        &[("decision", Some("R2R"), "正常行", "内容", Some(5))],
    );
    // A dangling supersede reference: Python ran without foreign keys, so
    // the surgery connection must opt out to reproduce that legacy state.
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        connection
            .execute("UPDATE memories SET superseded_by=99 WHERE id=1", [])
            .unwrap();
        connection.close().unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let error = MemoryStore::open(&path, "mcp").unwrap_err();
    assert!(matches!(error, MemoryError::IncompatibleSchema));
    let after = std::fs::read(&path).unwrap();
    assert_eq!(
        before, after,
        "fail-closed must not rewrite a malformed archive"
    );
    assert!(!path.with_extension("db.v1.bak").exists());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn unknown_layout_and_newer_version_fail_closed() {
    let path = temp_db("unknown");
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE something_else (id INTEGER PRIMARY KEY);")
            .unwrap();
        connection.close().unwrap();
    }
    assert!(matches!(
        MemoryStore::open(&path, "mcp"),
        Err(MemoryError::IncompatibleSchema)
    ));
    let _ = std::fs::remove_file(&path);

    let path = temp_db("newer");
    {
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("PRAGMA user_version=3;").unwrap();
        connection.close().unwrap();
    }
    assert!(matches!(
        MemoryStore::open(&path, "mcp"),
        Err(MemoryError::NewerSchema)
    ));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn two_stores_on_one_file_interleave_without_seq_collisions() {
    let path = temp_db("concurrent");
    let mcp_store = MemoryStore::open(&path, "mcp").unwrap();
    let http_store = MemoryStore::open(&path, "http").unwrap();
    let mut seen: Vec<i64> = Vec::new();
    for round in 0..5 {
        let id_a = mcp_store
            .remember(&draft(MemoryKind::Fact, &format!("mcp {round}"), "x"))
            .unwrap();
        let id_b = http_store
            .remember(&draft(MemoryKind::Fact, &format!("http {round}"), "y"))
            .unwrap();
        seen.push(mcp_store.get(id_a).unwrap().unwrap().seq);
        seen.push(http_store.get(id_b).unwrap().unwrap().seq);
    }
    let unique: std::collections::HashSet<i64> = seen.iter().copied().collect();
    assert_eq!(
        unique.len(),
        seen.len(),
        "sequences never collide: {seen:?}"
    );
    let unique_ids: std::collections::HashSet<i64> = [&mcp_store, &http_store]
        .into_iter()
        .flat_map(|store| {
            store
                .recall(&RecallFilter {
                    limit: Some(50),
                    ..RecallFilter::default()
                })
                .unwrap()
        })
        .map(|record| record.id)
        .collect();
    assert_eq!(unique_ids.len(), 10, "both writers see the full archive");
    assert_eq!(mcp_store.journal_mode().unwrap(), "wal");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db.v1.bak"));
}

#[test]
fn origin_must_be_a_slug() {
    let path = temp_db("origin");
    assert!(matches!(
        MemoryStore::open(&path, "Not Valid"),
        Err(MemoryError::Validation(_))
    ));
    let _ = std::fs::remove_file(path);
}

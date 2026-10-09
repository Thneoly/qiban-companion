//! One local archive, globally bounded; model context is scoped by endpoint and model.
use crate::StorageError;
use companion_core::conversation::{ChatTurn, ChatTurnUsage, TurnUsageKind};
use companion_core::memory::MAX_COUNTER;
use rusqlite::{params, Connection, TransactionBehavior};
use std::{path::Path, time::Duration};

/// The ledger accepts at most one app and one personal selection set per turn
/// (mirrors the send-time admission limits); anything else is refused whole.
fn validate_usage(usage: &[ChatTurnUsage]) -> Result<(), StorageError> {
    let mut app = 0usize;
    let mut personal = 0usize;
    for (index, entry) in usage.iter().enumerate() {
        match entry.kind {
            TurnUsageKind::App => {
                app += 1;
                if uuid::Uuid::parse_str(&entry.memory_id).is_err()
                    || entry.memory_id.contains('\0')
                    || !(1..=MAX_COUNTER).contains(&entry.revision)
                {
                    return Err(StorageError::Unavailable);
                }
            }
            TurnUsageKind::Personal => {
                personal += 1;
                if entry.personal_id < 1 || !(1..=MAX_COUNTER).contains(&entry.personal_seq) {
                    return Err(StorageError::Unavailable);
                }
            }
        }
        // Duplicate rows would fight the unique index; reject before the INSERT.
        if usage[..index].iter().any(|other| {
            other.kind == entry.kind
                && other.memory_id == entry.memory_id
                && other.personal_id == entry.personal_id
        }) {
            return Err(StorageError::Unavailable);
        }
    }
    if app > 5 || personal > 5 {
        return Err(StorageError::Unavailable);
    }
    Ok(())
}

pub struct HistoryStore(pub(crate) Connection);
impl HistoryStore {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Self::from_connection(Connection::open(path)?)
    }
    fn from_connection(mut connection: Connection) -> Result<Self, StorageError> {
        // Match the 5s wait used by the task/account stores: the previous
        // instance's exit can overlap the next one's startup on quick restarts.
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "secure_delete", "ON")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 4 {
            return Err(StorageError::NewerSchema);
        }
        let integrity: String = tx.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(StorageError::Unavailable);
        }
        if version == 0 {
            tx.execute_batch("CREATE TABLE chat_turns (id INTEGER PRIMARY KEY, base TEXT NOT NULL, model TEXT NOT NULL, user TEXT NOT NULL, assistant TEXT NOT NULL); PRAGMA user_version=1;")?;
        }
        // Fail closed on incompatible/corrupt data, without deleting or overwriting it.
        let (count, size): (i64, i64) = tx.query_row(
            "SELECT count(*), coalesce(sum(length(user)+length(assistant)),0) FROM chat_turns",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if count > 6 || size > 12000 {
            return Err(StorageError::Unavailable);
        }
        // Prepare the real read before migrating; a malformed v1 table must not become v2.
        Self::read(&tx, "", "")?;
        if version < 2 {
            tx.execute_batch(include_str!("memory-schema.sql"))?;
        }
        // v3 is additive (personal memory policy tables); replaying the v2
        // file on an existing v2 archive would collide on its tables.
        if version < 3 {
            tx.execute_batch(include_str!("memory-schema-v3.sql"))?;
        }
        // v4 adds the chat_turn_usage ledger and backfills pre-v4 turns from
        // the selections enabled at migration time (conservative direction).
        if version < 4 {
            tx.execute_batch(include_str!("memory-schema-v4.sql"))?;
        }
        crate::memory::validate_schema(&tx)?;
        tx.commit()?;
        Ok(Self(connection))
    }
    pub fn load(&self, base: &str, model: &str) -> Result<Vec<ChatTurn>, StorageError> {
        Self::read(&self.0, base, model)
    }
    fn read(
        connection: &Connection,
        base: &str,
        model: &str,
    ) -> Result<Vec<ChatTurn>, StorageError> {
        let mut statement = connection.prepare(
            "SELECT user, assistant FROM chat_turns WHERE base=?1 AND model=?2 ORDER BY id",
        )?;
        let turns = statement
            .query_map(params![base, model], |r| {
                Ok(ChatTurn {
                    user: r.get(0)?,
                    assistant: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(turns)
    }
    pub fn append_with_usage(
        &mut self,
        base: &str,
        model: &str,
        turn: &ChatTurn,
        usage: &[ChatTurnUsage],
    ) -> Result<Vec<ChatTurn>, StorageError> {
        // Oversized complete pairs are not archived; preserve previous history.
        if turn.user.is_empty()
            || turn.assistant.is_empty()
            || turn.user.chars().count() + turn.assistant.chars().count() > 12000
            || turn.user.contains('\0')
            || turn.assistant.contains('\0')
        {
            return Err(StorageError::Unavailable);
        }
        validate_usage(usage)?;
        let tx = self.0.transaction()?;
        tx.execute(
            "INSERT INTO chat_turns(base,model,user,assistant) VALUES(?1,?2,?3,?4)",
            params![base, model, turn.user, turn.assistant],
        )?;
        let turn_id = tx.last_insert_rowid();
        for entry in usage {
            match entry.kind {
                TurnUsageKind::App => {
                    tx.execute(
                        "INSERT INTO chat_turn_usage(turn_id,memory_kind,memory_id,revision,personal_id,personal_seq)
                         VALUES(?1,'app',?2,?3,NULL,NULL)",
                        params![turn_id, entry.memory_id, entry.revision],
                    )?;
                }
                TurnUsageKind::Personal => {
                    tx.execute(
                        "INSERT INTO chat_turn_usage(turn_id,memory_kind,memory_id,revision,personal_id,personal_seq)
                         VALUES(?1,'personal',NULL,NULL,?2,?3)",
                        params![turn_id, entry.personal_id, entry.personal_seq],
                    )?;
                }
            }
        }
        loop {
            let (count, size): (i64, i64) = tx.query_row(
                "SELECT count(*), coalesce(sum(length(user)+length(assistant)),0) FROM chat_turns",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if count <= 6 && size <= 12000 {
                break;
            }
            tx.execute(
                "DELETE FROM chat_turns WHERE id=(SELECT min(id) FROM chat_turns)",
                [],
            )?;
        }
        let turns = Self::read(&tx, base, model)?;
        tx.commit()?;
        Ok(turns)
    }
    pub fn clear(&mut self) -> Result<(), StorageError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let epoch: i64 = tx.query_row(
            "SELECT context_epoch FROM memory_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        let next = companion_core::memory::next_counter(epoch)?;
        tx.execute("DELETE FROM chat_turns", [])?;
        tx.execute(
            "UPDATE memory_meta SET context_epoch=?1 WHERE singleton=1",
            [next],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use companion_core::memory::{
        MemoryDraft, MemoryKind, MemoryPolicyChange, MemoryScope, PersonalMemoryChange,
        PersonalMemoryPolicy,
    };
    fn turn(n: usize) -> ChatTurn {
        ChatTurn {
            user: format!("问题{n}"),
            assistant: "答复🌱".into(),
        }
    }
    #[test]
    fn reopens_isolates_prunes_and_deletes_all_scopes() {
        let path = std::env::temp_dir().join(format!("history-{}.db", uuid::Uuid::new_v4()));
        {
            let mut store = HistoryStore::open(&path).unwrap();
            for n in 0..8 {
                store
                    .append_with_usage("https://a", "a", &turn(n), &[])
                    .unwrap();
            }
            assert_eq!(store.load("https://a", "a").unwrap()[0].user, "问题2");
            store
                .append_with_usage("https://b", "a", &turn(8), &[])
                .unwrap();
            store
                .append_with_usage("https://a", "b", &turn(9), &[])
                .unwrap();
        }
        {
            let mut store = HistoryStore::open(&path).unwrap();
            assert_eq!(store.load("https://a", "a").unwrap().len(), 4);
            assert_eq!(store.load("https://b", "a").unwrap(), vec![turn(8)]);
            assert_eq!(store.load("https://a", "b").unwrap(), vec![turn(9)]);
            store
                .append_with_usage(
                    "https://a",
                    "b",
                    &ChatTurn {
                        user: "新".into(),
                        assistant: "字".repeat(11999),
                    },
                    &[],
                )
                .unwrap();
            assert!(store.load("https://a", "a").unwrap().is_empty());
            assert!(store
                .append_with_usage(
                    "https://a",
                    "b",
                    &ChatTurn {
                        user: "超".into(),
                        assistant: "字".repeat(12000)
                    },
                    &[],
                )
                .is_err());
            assert_eq!(store.load("https://a", "b").unwrap().len(), 1);
            store.clear().unwrap();
            store.clear().unwrap();
        }
        assert!(HistoryStore::open(&path)
            .unwrap()
            .load("https://a", "b")
            .unwrap()
            .is_empty());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn refuses_future_and_corrupt_schema_without_overwrite() {
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 5).unwrap();
        assert!(matches!(
            HistoryStore::from_connection(connection),
            Err(StorageError::NewerSchema)
        ));
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        assert!(HistoryStore::from_connection(connection).is_err());
    }
    #[test]
    fn usage_ledger_validates_rows_and_cascades_on_eviction_and_clear() {
        let mut store =
            HistoryStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        // Duplicates, non-UUID ids, out-of-range revisions and non-positive
        // personal ids are refused whole, before any turn is written.
        let dup = [
            ChatTurnUsage::app(id.clone(), 1),
            ChatTurnUsage::app(id.clone(), 1),
        ];
        assert!(store.append_with_usage("a", "m", &turn(1), &dup).is_err());
        assert!(store
            .append_with_usage(
                "a",
                "m",
                &turn(1),
                &[ChatTurnUsage::app("not-a-uuid".into(), 1)]
            )
            .is_err());
        assert!(store
            .append_with_usage("a", "m", &turn(1), &[ChatTurnUsage::app(id.clone(), 0)])
            .is_err());
        assert!(store
            .append_with_usage("a", "m", &turn(1), &[ChatTurnUsage::personal(0, 1)])
            .is_err());
        let six_app: Vec<_> = (0..6)
            .map(|_| ChatTurnUsage::app(uuid::Uuid::new_v4().to_string(), 1))
            .collect();
        assert!(store
            .append_with_usage("a", "m", &turn(1), &six_app)
            .is_err());
        // App and personal entries coexist on one turn.
        store
            .append_with_usage(
                "a",
                "m",
                &turn(1),
                &[ChatTurnUsage::app(id, 1), ChatTurnUsage::personal(5, 2)],
            )
            .unwrap();
        let usage: i64 = store
            .0
            .query_row("SELECT count(*) FROM chat_turn_usage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(usage, 2);
        // Evicting the ledgered turn (9 turns keep the last 6) cascades its rows away.
        for n in 2..10 {
            store.append_with_usage("a", "m", &turn(n), &[]).unwrap();
        }
        let usage: i64 = store
            .0
            .query_row("SELECT count(*) FROM chat_turn_usage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(usage, 0);
        // A manual clear cascades too, and the archive stays valid.
        store
            .append_with_usage("a", "m", &turn(10), &[ChatTurnUsage::personal(7, 1)])
            .unwrap();
        store.clear().unwrap();
        let usage: i64 = store
            .0
            .query_row("SELECT count(*) FROM chat_turn_usage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(usage, 0);
        crate::memory::validate_schema(&store.0).unwrap();
    }

    #[test]
    fn migrates_real_v3_file_backfilling_both_usage_families() {
        // A genuine v3 archive the way a v3 binary would have left it: turns,
        // an enabled app policy with one selection, an enabled personal policy
        // with one selection, stamped user_version=3 (no usage table).
        let path = std::env::temp_dir().join(format!("history-v3-{}.db", uuid::Uuid::new_v4()));
        let memory_id = uuid::Uuid::new_v4().to_string();
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE chat_turns(id INTEGER PRIMARY KEY,base TEXT NOT NULL,model TEXT NOT NULL,user TEXT NOT NULL,assistant TEXT NOT NULL);").unwrap();
            db.execute_batch(include_str!("memory-schema.sql")).unwrap();
            db.execute_batch(include_str!("memory-schema-v3.sql"))
                .unwrap();
            db.execute(
                "INSERT INTO chat_turns(base,model,user,assistant) VALUES('https://a','m','旧问题','旧回答')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO memories(id,kind,body,source_kind,source_label,event_date,created_at,confirmed_at,updated_at,revision) \
                 VALUES(?1,'preference','正文','user_manual','用户在记忆面板填写',NULL,10,10,10,1)",
                [&memory_id],
            )
            .unwrap();
            db.execute_batch(
                "INSERT INTO memory_policy VALUES('https://a','m',1,1);                          INSERT INTO personal_memory_policy VALUES('https://a','m',1,1);                          INSERT INTO personal_memory_selection VALUES('https://a','m',5,0);                          PRAGMA user_version=3;",
            )
            .unwrap();
            db.execute(
                "INSERT INTO memory_selection VALUES('https://a','m',?1,0)",
                [&memory_id],
            )
            .unwrap();
        }
        {
            let store = HistoryStore::open(&path).unwrap();
            assert_eq!(
                store
                    .0
                    .pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))
                    .unwrap(),
                4
            );
            let app: (i64, i64) = store
                .0
                .query_row(
                    "SELECT count(*), coalesce(sum(revision),0) FROM chat_turn_usage WHERE memory_kind='app'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(app, (1, 1)); // one row, carrying the send-time revision
            let personal: (i64, i64) = store
                .0
                .query_row(
                    "SELECT count(*), coalesce(sum(personal_id),0) FROM chat_turn_usage WHERE memory_kind='personal' AND personal_seq IS NULL",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(personal, (1, 5)); // seq is not recoverable for backfilled rows
            crate::memory::validate_schema(&store.0).unwrap();
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_writes_keep_previous_history() {
        let mut store =
            HistoryStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store.append_with_usage("a", "a", &turn(1), &[]).unwrap();
        store.0.pragma_update(None, "query_only", true).unwrap();
        assert!(store.clear().is_err());
        assert!(store.append_with_usage("a", "a", &turn(2), &[]).is_err());
        assert_eq!(store.load("a", "a").unwrap(), vec![turn(1)]);
    }

    #[test]
    fn migrates_real_v1_file_preserving_ids_and_all_scopes() {
        let path = std::env::temp_dir().join(format!("migration-{}.db", uuid::Uuid::new_v4()));
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE chat_turns(id INTEGER PRIMARY KEY,base TEXT NOT NULL,model TEXT NOT NULL,user TEXT NOT NULL,assistant TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
            for (id, base, model) in [(7, "a", "one"), (11, "a", "two"), (20, "b", "one")] {
                db.execute(
                    "INSERT INTO chat_turns VALUES(?1,?2,?3,'升级前问题🌱','升级前回答')",
                    params![id, base, model],
                )
                .unwrap();
            }
        }
        for _ in 0..2 {
            let store = HistoryStore::open(&path).unwrap();
            assert_eq!(
                store
                    .0
                    .pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))
                    .unwrap(),
                4
            );
            // Pre-v4 turns had no selections to backfill from: empty ledger.
            let usage: i64 = store
                .0
                .query_row("SELECT count(*) FROM chat_turn_usage", [], |r| r.get(0))
                .unwrap();
            assert_eq!(usage, 0);
            assert_eq!(store.context_epoch().unwrap(), 0);
            assert!(store.memory_list().unwrap().is_empty());
            let policies: i64 = store
                .0
                .query_row("SELECT count(*) FROM memory_policy", [], |r| r.get(0))
                .unwrap();
            assert_eq!(policies, 0);
            for (base, model) in [("a", "one"), ("a", "two"), ("b", "one")] {
                assert_eq!(store.load(base, model).unwrap()[0].user, "升级前问题🌱");
            }
            let ids: Vec<i64> = store
                .0
                .prepare("SELECT id FROM chat_turns ORDER BY id")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(ids, vec![7, 11, 20]);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn interrupted_migration_rolls_back_ddl_version_and_history() {
        let path = std::env::temp_dir().join(format!("migration-fail-{}.db", uuid::Uuid::new_v4()));
        {
            let db = Connection::open(&path).unwrap();
            // Collision halfway through DDL forces a failure after memory_meta creation.
            db.execute_batch("CREATE TABLE chat_turns(id INTEGER PRIMARY KEY,base TEXT NOT NULL,model TEXT NOT NULL,user TEXT NOT NULL,assistant TEXT NOT NULL); INSERT INTO chat_turns VALUES(9,'a','a','keep','answer'); CREATE TABLE memories(marker TEXT); PRAGMA user_version=1;").unwrap();
        }
        assert!(HistoryStore::open(&path).is_err());
        {
            let db = Connection::open(&path).unwrap();
            assert_eq!(
                db.pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))
                    .unwrap(),
                1
            );
            let tables: Vec<String> = db
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(tables, vec!["chat_turns", "memories"]);
            assert_eq!(HistoryStore::read(&db, "a", "a").unwrap()[0].user, "keep");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn memory_deletion_survives_reopen_and_future_version_is_untouched() {
        use companion_core::memory::{MemoryDraft, MemoryKind};
        let path = std::env::temp_dir().join(format!("memory-reopen-{}.db", uuid::Uuid::new_v4()));
        let id;
        {
            let mut s = HistoryStore::open(&path).unwrap();
            id = s
                .memory_create(
                    &MemoryDraft {
                        kind: MemoryKind::Experience,
                        body: "synthetic secret".into(),
                        event_date: Some("2020-02-29".into()),
                    },
                    0,
                )
                .unwrap()
                .value
                .id;
        }
        {
            let mut s = HistoryStore::open(&path).unwrap();
            assert_eq!(s.memory_list().unwrap()[0].id, id);
            s.memory_delete(&id, 1, 1).unwrap();
        }
        {
            let s = HistoryStore::open(&path).unwrap();
            assert!(s.memory_list().unwrap().is_empty());
            assert_eq!(s.context_epoch().unwrap(), 2);
            let marker: i64 =
                s.0.query_row(
                    "SELECT count(*) FROM memories WHERE body IS NULL AND deleted_at IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(marker, 1);
            s.0.pragma_update(None, "user_version", 5).unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(
            HistoryStore::open(&path),
            Err(StorageError::NewerSchema)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn migrates_real_v2_file_preserving_chats_and_both_policy_families() {
        // Build a genuine v2 archive the way a v2 binary would have left it:
        // chats, an app policy with selections, then stamp user_version=2.
        let path = std::env::temp_dir().join(format!("history-v2-{}.db", uuid::Uuid::new_v4()));
        let scope = MemoryScope {
            base_url: "https://a".into(),
            model: "m".into(),
        };
        let turns: Vec<_> = (0..3)
            .map(|n| ChatTurn {
                user: format!("问题{n}"),
                assistant: "答复".into(),
            })
            .collect();
        let mut memories = Vec::new();
        {
            let mut store = HistoryStore::open(&path).unwrap();
            for t in &turns {
                store.append_with_usage("https://a", "m", t, &[]).unwrap();
            }
            for n in 0..2 {
                memories.push(
                    store
                        .memory_create(
                            &MemoryDraft {
                                kind: MemoryKind::Preference,
                                body: format!("记忆{n}"),
                                event_date: None,
                            },
                            n,
                        )
                        .unwrap()
                        .value,
                );
            }
            store
                .memory_policy_set(
                    &scope,
                    &MemoryPolicyChange {
                        expected_scope: scope.clone(),
                        expected_revision: 0,
                        expected_epoch: 2,
                        enabled: true,
                        selected_ids: memories.iter().map(|m| m.id.clone()).collect(),
                        restart_conversation: false,
                    },
                )
                .unwrap();
            drop(store);
            // Rewind the file to a true v2 archive: drop the additive v3/v4
            // tables and stamp user_version=2, exactly what a v2 binary
            // would leave.
            let db = Connection::open(&path).unwrap();
            db.execute_batch(
            "DROP TABLE chat_turn_usage;              DROP TABLE personal_memory_selection;              DROP TABLE personal_memory_policy;              PRAGMA user_version=2;",
        )
        .unwrap();
            let v3_tables: i64 = db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('personal_memory_policy','personal_memory_selection')",
            [], |r| r.get(0),
        ).unwrap();
            assert_eq!(v3_tables, 0);
            db.close().unwrap();
        }
        {
            let mut store = HistoryStore::open(&path).unwrap();
            assert_eq!(
                store
                    .0
                    .pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))
                    .unwrap(),
                4
            );
            // Backfill: 3 turns x 2 selected app memories = 6 ledger rows,
            // attributed to the migration-time selection with send-time
            // revisions; personal policy was empty, so no personal rows.
            let usage: i64 = store
                .0
                .query_row("SELECT count(*) FROM chat_turn_usage", [], |r| r.get(0))
                .unwrap();
            assert_eq!(usage, 6);
            let backfilled = store
                .0
                .query_row(
                    "SELECT count(*) FROM chat_turn_usage u JOIN memories m ON m.id=u.memory_id \
                     WHERE u.memory_kind='app' AND u.revision=m.revision",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap();
            assert_eq!(backfilled, 6);
            assert_eq!(store.load("https://a", "m").unwrap(), turns);
            assert_eq!(store.memory_list().unwrap().len(), 2);
            assert_eq!(
                store.memory_policy(&scope).unwrap().selected_ids,
                memories.iter().map(|m| m.id.clone()).collect::<Vec<_>>()
            );
            // Personal family starts empty and writable post-migration.
            assert_eq!(
                store.personal_memory_policy(&scope).unwrap(),
                PersonalMemoryPolicy::default()
            );
            store
                .personal_memory_policy_set(
                    &scope,
                    &PersonalMemoryChange {
                        expected_scope: scope.clone(),
                        expected_revision: 0,
                        expected_epoch: 3,
                        enabled: true,
                        selected_ids: vec![7, 8],
                        restart_conversation: false,
                    },
                )
                .unwrap();
            assert_eq!(
                store.personal_memory_policy(&scope).unwrap().selected_ids,
                vec![7, 8]
            );
            crate::memory::validate_schema(&store.0).unwrap();
        }
        std::fs::remove_file(path).unwrap();
    }
}

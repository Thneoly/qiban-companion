use super::*;
use companion_core::conversation::ChatTurn;

fn store() -> HistoryStore {
    let mut db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch("CREATE TABLE chat_turns(id INTEGER PRIMARY KEY,base TEXT NOT NULL,model TEXT NOT NULL,user TEXT NOT NULL,assistant TEXT NOT NULL);").unwrap();
    tx.execute_batch(include_str!("memory-schema.sql")).unwrap();
    tx.commit().unwrap();
    HistoryStore(db)
}
fn draft(body: &str) -> MemoryDraft {
    MemoryDraft {
        kind: MemoryKind::Preference,
        body: body.into(),
        event_date: None,
    }
}
fn chat(s: &mut HistoryStore) {
    for scope in ["a", "b"] {
        s.append(
            scope,
            "model",
            &ChatTurn {
                user: "question".into(),
                assistant: "answer".into(),
            },
        )
        .unwrap();
    }
}
fn chats(s: &HistoryStore) -> i64 {
    s.0.query_row("SELECT count(*) FROM chat_turns", [], |r| r.get(0))
        .unwrap()
}
fn select(s: &HistoryStore, base: &str, memories: &[Memory]) {
    s.0.execute(
        "INSERT INTO memory_policy(base_url,model,enabled,revision) VALUES(?1,'model',1,1)",
        [base],
    )
    .unwrap();
    for (position, m) in memories.iter().enumerate() {
        s.0.execute(
            "INSERT INTO memory_selection VALUES(?1,'model',?2,?3)",
            params![base, m.id, position as i64],
        )
        .unwrap();
    }
}

#[test]
fn manual_capacity_source_and_stale_create() {
    let mut s = store();
    chat(&mut s);
    assert_eq!(s.context_epoch().unwrap(), 0);
    for n in 0..30 {
        let commit = s.memory_create(&draft(" user input "), n).unwrap();
        assert!(!commit.chat_cleared);
        assert_eq!(commit.value.source_kind, MemorySource::UserManual);
        assert_eq!(commit.value.source_label, MANUAL_SOURCE_LABEL);
        assert_eq!(commit.value.body, "user input");
        assert_eq!(commit.value.revision, 1);
    }
    assert!(matches!(
        s.memory_create(&draft("overflow"), 30),
        Err(StorageError::Memory(MemoryError::CapacityExceeded))
    ));
    assert!(matches!(
        s.memory_create(&draft("stale"), 0),
        Err(StorageError::Memory(MemoryError::ContextChanged))
    ));
    assert_eq!(s.context_epoch().unwrap(), 30);
    assert_eq!(chats(&s), 2);
    assert_eq!(s.memory_list().unwrap().len(), 30);
}

#[test]
fn policy_is_explicit_scoped_versioned_and_revocation_is_atomic() {
    let mut s = store();
    let scope = MemoryScope {
        base_url: "https://a".into(),
        model: "model".into(),
    };
    let other = MemoryScope {
        base_url: "https://b".into(),
        model: "model".into(),
    };
    let one = s.memory_create(&draft("one"), 0).unwrap().value;
    let two = s.memory_create(&draft("two"), 1).unwrap().value;
    chat(&mut s);
    assert_eq!(s.memory_policy(&scope).unwrap(), MemoryPolicy::default());
    let mut change = MemoryPolicyChange {
        expected_scope: scope.clone(),
        expected_revision: 0,
        expected_epoch: 2,
        enabled: true,
        selected_ids: vec![two.id.clone(), one.id.clone()],
        restart_conversation: false,
    };
    assert!(s.memory_policy_set(&other, &change).is_err());
    let commit = s.memory_policy_set(&scope, &change).unwrap();
    assert!(!commit.chat_cleared);
    assert_eq!(commit.value.selected_ids, change.selected_ids);
    assert_eq!(chats(&s), 2);
    assert_eq!(s.memory_policy(&other).unwrap(), MemoryPolicy::default());
    assert!(s.memory_policy_set(&scope, &change).is_err());
    change.expected_epoch = commit.context_epoch;
    change.expected_revision = commit.value.revision;
    change.selected_ids = vec![one.id.clone()];
    assert!(matches!(
        s.memory_policy_set(&scope, &change),
        Err(StorageError::Memory(MemoryError::ConfirmationRequired))
    ));
    assert_eq!(s.context_epoch().unwrap(), 3);
    assert_eq!(chats(&s), 2);
    // Trigger a real write failure after the transaction has changed policy/epoch.
    s.0.execute_batch("CREATE TRIGGER deny_chat_delete BEFORE DELETE ON chat_turns BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    change.restart_conversation = true;
    assert!(s.memory_policy_set(&scope, &change).is_err());
    assert_eq!(s.context_epoch().unwrap(), 3);
    assert_eq!(s.memory_policy(&scope).unwrap().selected_ids.len(), 2);
    s.0.execute_batch("DROP TRIGGER deny_chat_delete;").unwrap();
    let commit = s.memory_policy_set(&scope, &change).unwrap();
    assert!(commit.chat_cleared);
    assert_eq!(chats(&s), 0);
    change.expected_epoch = commit.context_epoch;
    change.expected_revision = commit.value.revision;
    let retry = s.memory_policy_set(&scope, &change).unwrap();
    assert_eq!(retry.context_epoch, 4);
    assert!(!retry.chat_cleared);
    change.enabled = false;
    change.selected_ids.clear();
    let off = s.memory_policy_set(&scope, &change).unwrap();
    assert!(!off.value.enabled);
    assert_eq!(s.memory_list().unwrap().len(), 2);
    assert!(s.memory_policy(&scope).unwrap().selected_ids.is_empty());
}

#[test]
fn policy_budget_order_and_deleted_selections_survive_reopen() {
    let path = std::env::temp_dir().join(format!("policy-{}.db", uuid::Uuid::new_v4()));
    let scope = MemoryScope {
        base_url: "https://a".into(),
        model: "m".into(),
    };
    let mut s = HistoryStore::open(&path).unwrap();
    let mut ids = Vec::new();
    for n in 0..6 {
        ids.push(
            s.memory_create(&draft(&"🌱".repeat(160)), n)
                .unwrap()
                .value
                .id,
        );
    }
    let mut change = MemoryPolicyChange {
        expected_scope: scope.clone(),
        expected_revision: 0,
        expected_epoch: 6,
        enabled: true,
        selected_ids: ids.clone(),
        restart_conversation: false,
    };
    assert!(s.memory_policy_set(&scope, &change).is_err());
    change.selected_ids = ids[..5].to_vec();
    s.memory_policy_set(&scope, &change).unwrap();
    assert_eq!(
        s.memory_budget_conflicts(&ids[0], &draft(&"a".repeat(161)))
            .unwrap(),
        vec![scope.clone()]
    );
    assert!(matches!(
        s.memory_update(&ids[0], 1, 7, &draft(&"a".repeat(161))),
        Err(StorageError::Memory(MemoryError::SelectionTooLarge))
    ));
    assert_eq!(s.memory_list().unwrap()[0].revision, 1);
    drop(s);
    let mut s = HistoryStore::open(&path).unwrap();
    assert_eq!(s.memory_policy(&scope).unwrap().selected_ids, ids[..5]);
    s.memory_delete(&ids[2], 1, 7).unwrap();
    assert!(!s
        .memory_policy(&scope)
        .unwrap()
        .selected_ids
        .contains(&ids[2]));
    s.memory_delete_all(8).unwrap();
    assert!(!s.memory_policy(&scope).unwrap().enabled);
    drop(s);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn edit_conflicts_and_clear_all_scopes_atomically() {
    let mut s = store();
    let original = s.memory_create(&draft("before"), 0).unwrap().value;
    chat(&mut s);
    assert!(matches!(
        s.memory_update(&original.id, 0, 1, &draft("wrong")),
        Err(StorageError::Memory(MemoryError::Conflict))
    ));
    assert_eq!(chats(&s), 2);
    assert_eq!(s.context_epoch().unwrap(), 1);
    let edited = s
        .memory_update(&original.id, 1, 1, &draft("after"))
        .unwrap();
    assert!(edited.chat_cleared);
    assert_eq!(edited.context_epoch, 2);
    assert_eq!(edited.value.created_at, original.created_at);
    assert_eq!(edited.value.revision, 2);
    assert_eq!(edited.value.body, "after");
    assert_eq!(chats(&s), 0);
    assert!(s
        .memory_update(&original.id, 2, 1, &draft("stale"))
        .is_err());
}

#[test]
fn deletion_scrubs_content_and_retry_preserves_new_chat() {
    let mut s = store();
    let original = s.memory_create(&draft("private text"), 0).unwrap().value;
    select(&s, "a", std::slice::from_ref(&original));
    chat(&mut s);
    let removed = s.memory_delete(&original.id, 1, 1).unwrap();
    assert!(removed.chat_cleared);
    assert!(s.memory_list().unwrap().is_empty());
    assert_eq!(chats(&s), 0);
    let tombstone =
        s.0.query_row(
            "SELECT id,kind,revision,deleted_at FROM memories",
            [],
            |r| {
                Ok(MemoryTombstone {
                    id: r.get(0)?,
                    kind: MemoryKind::Preference,
                    revision: r.get(2)?,
                    deleted_at: r.get(3)?,
                })
            },
        )
        .unwrap();
    assert_eq!(tombstone.id, original.id);
    assert_eq!(tombstone.revision, 2);
    validate_schema(&s.0).unwrap();
    let policy: (i64, i64) =
        s.0.query_row("SELECT enabled,revision FROM memory_policy", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(policy, (0, 2));
    chat(&mut s);
    let retry = s.memory_delete(&original.id, 1, 1).unwrap();
    assert!(!retry.chat_cleared);
    assert_eq!(retry.context_epoch, 2);
    assert_eq!(chats(&s), 2);
    let new = s.memory_create(&draft("new"), 2).unwrap();
    assert_ne!(new.value.id, original.id);
    assert!(s
        .memory_update(&original.id, 2, 3, &draft("resurrect"))
        .is_err());
}

#[test]
fn stale_delete_all_does_not_delete_later_creation() {
    let mut s = store();
    s.memory_create(&draft("old"), 0).unwrap();
    s.memory_delete_all(1).unwrap();
    s.memory_create(&draft("new"), 2).unwrap();
    chat(&mut s);
    assert!(s.memory_delete_all(1).is_err());
    assert_eq!(s.memory_list().unwrap()[0].body, "new");
    assert_eq!(chats(&s), 2);
    s.clear().unwrap();
    assert_eq!(s.context_epoch().unwrap(), 4);
    assert_eq!(s.memory_list().unwrap().len(), 1);
}

#[test]
fn selection_order_limits_and_edit_budget_are_enforced() {
    let mut s = store();
    let mut memories = Vec::new();
    for n in 0..6 {
        memories.push(s.memory_create(&draft(&"🌱".repeat(160)), n).unwrap().value);
    }
    let mut ids: Vec<_> = memories[..5].iter().map(|m| m.id.clone()).collect();
    ids.reverse();
    assert_eq!(
        validate_selection(&ids, &memories).unwrap()[0].id,
        memories[4].id
    );
    select(&s, "a", &memories[..5]);
    select(&s, "b", &memories[..1]);
    chat(&mut s);
    assert!(matches!(
        s.memory_update(&memories[0].id, 1, 6, &draft(&"🌱".repeat(161))),
        Err(StorageError::Memory(MemoryError::SelectionTooLarge))
    ));
    assert_eq!(
        s.memory_list()
            .unwrap()
            .iter()
            .find(|m| m.id == memories[0].id)
            .unwrap()
            .body
            .chars()
            .count(),
        160
    );
    assert_eq!(s.context_epoch().unwrap(), 6);
    assert_eq!(chats(&s), 2);
    ids.push(memories[5].id.clone());
    assert!(validate_selection(&ids, &memories).is_err());
    assert!(validate_selection(&["missing".into()], &memories).is_err());
    assert!(
        validate_selection(&[memories[0].id.clone(), memories[0].id.clone()], &memories).is_err()
    );
}

#[test]
fn late_failure_rolls_back_content_epoch_selection_and_history() {
    let mut s = store();
    let original = s.memory_create(&draft("before"), 0).unwrap().value;
    select(&s, "a", std::slice::from_ref(&original));
    chat(&mut s);
    s.0.execute_batch("CREATE TRIGGER fail_chat_delete BEFORE DELETE ON chat_turns BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(s
        .memory_update(&original.id, 1, 1, &draft("after"))
        .is_err());
    assert!(s.memory_delete(&original.id, 1, 1).is_err());
    assert!(s.memory_delete_all(1).is_err());
    assert_eq!(s.memory_list().unwrap(), vec![original]);
    assert_eq!(s.context_epoch().unwrap(), 1);
    assert_eq!(chats(&s), 2);
    validate_schema(&s.0).unwrap();
    let count: i64 =
        s.0.query_row("SELECT count(*) FROM memory_selection", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 1);
    s.0.pragma_update(None, "query_only", true).unwrap();
    assert!(s.memory_create(&draft("read only"), 1).is_err());
}

#[test]
fn epoch_memory_and_policy_revision_overflow_are_atomic() {
    let mut s = store();
    let original = s.memory_create(&draft("keep"), 0).unwrap().value;
    chat(&mut s);
    s.0.execute("UPDATE memory_meta SET context_epoch=?1", [MAX_COUNTER])
        .unwrap();
    assert!(s.memory_create(&draft("no"), MAX_COUNTER).is_err());
    assert!(s.clear().is_err());
    s.0.execute("UPDATE memory_meta SET context_epoch=1", [])
        .unwrap();
    s.0.execute("UPDATE memories SET revision=?1", [MAX_COUNTER])
        .unwrap();
    assert!(s
        .memory_update(&original.id, MAX_COUNTER, 1, &draft("no"))
        .is_err());
    assert!(s.memory_delete(&original.id, MAX_COUNTER, 1).is_err());
    assert!(s.memory_delete_all(1).is_err());
    s.0.execute("UPDATE memories SET revision=1", []).unwrap();
    select(&s, "a", std::slice::from_ref(&original));
    s.0.execute("UPDATE memory_policy SET revision=?1", [MAX_COUNTER])
        .unwrap();
    assert!(s.memory_delete(&original.id, 1, 1).is_err());
    assert_eq!(s.memory_list().unwrap(), vec![original]);
    assert_eq!(s.context_epoch().unwrap(), 1);
    assert_eq!(chats(&s), 2);
    validate_schema(&s.0).unwrap();
}

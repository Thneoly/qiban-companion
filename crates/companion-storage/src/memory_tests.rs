use super::*;
use companion_core::conversation::{ChatTurn, ChatTurnUsage};

fn store() -> HistoryStore {
    let mut db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch("CREATE TABLE chat_turns(id INTEGER PRIMARY KEY,base TEXT NOT NULL,model TEXT NOT NULL,user TEXT NOT NULL,assistant TEXT NOT NULL);").unwrap();
    tx.execute_batch(include_str!("memory-schema.sql")).unwrap();
    tx.execute_batch(include_str!("memory-schema-v3.sql"))
        .unwrap();
    tx.execute_batch(include_str!("memory-schema-v4.sql"))
        .unwrap();
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
fn turn() -> ChatTurn {
    ChatTurn {
        user: "question".into(),
        assistant: "answer".into(),
    }
}
fn chat(s: &mut HistoryStore) {
    chat_using(s, "a", &[]);
    chat_using(s, "b", &[]);
}
/// One turn whose ledger records the app memories it carried at send time.
fn chat_using(s: &mut HistoryStore, base: &str, usage: &[ChatTurnUsage]) {
    s.append_with_usage(base, "model", &turn(), usage).unwrap();
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
    // Scope "a" turns carried both selected memories at send time; scope "b"
    // never used them — that asymmetry is what precise cleanup keys on.
    chat_using(
        &mut s,
        "a",
        &[
            ChatTurnUsage::app(two.id.clone(), two.revision),
            ChatTurnUsage::app(one.id.clone(), one.revision),
        ],
    );
    chat_using(&mut s, "b", &[]);
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
    assert_eq!(commit.cleared_turns, 1);
    // Precise: scope "a" loses its carrier turn, scope "b" keeps its own.
    assert!(s.load("a", "model").unwrap().is_empty());
    assert_eq!(s.load("b", "model").unwrap(), vec![turn()]);
    assert_eq!(chats(&s), 1);
    change.expected_epoch = commit.context_epoch;
    change.expected_revision = commit.value.revision;
    let retry = s.memory_policy_set(&scope, &change).unwrap();
    assert_eq!(retry.context_epoch, 4);
    assert!(!retry.chat_cleared);
    change.enabled = false;
    change.selected_ids.clear();
    let off = s.memory_policy_set(&scope, &change).unwrap();
    assert!(!off.value.enabled);
    // Nothing still on record carries the removed items: zero turns cleared.
    assert!(!off.chat_cleared);
    assert_eq!(off.cleared_turns, 0);
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
fn edit_conflicts_and_prunes_only_carrier_scopes_atomically() {
    let mut s = store();
    let original = s.memory_create(&draft("before"), 0).unwrap().value;
    chat_using(
        &mut s,
        "a",
        &[ChatTurnUsage::app(original.id.clone(), original.revision)],
    );
    chat_using(&mut s, "b", &[]);
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
    assert_eq!(edited.cleared_turns, 1);
    assert_eq!(edited.context_epoch, 2);
    assert_eq!(edited.value.created_at, original.created_at);
    assert_eq!(edited.value.revision, 2);
    assert_eq!(edited.value.body, "after");
    // Precise: the scope that carried the memory is pruned; the other is not.
    assert!(s.load("a", "model").unwrap().is_empty());
    assert_eq!(s.load("b", "model").unwrap(), vec![turn()]);
    assert_eq!(chats(&s), 1);
    assert!(s
        .memory_update(&original.id, 2, 1, &draft("stale"))
        .is_err());
}

#[test]
fn deletion_scrubs_content_and_retry_preserves_new_chat() {
    let mut s = store();
    let original = s.memory_create(&draft("private text"), 0).unwrap().value;
    select(&s, "a", std::slice::from_ref(&original));
    chat_using(
        &mut s,
        "a",
        &[ChatTurnUsage::app(original.id.clone(), original.revision)],
    );
    chat_using(&mut s, "b", &[]);
    let removed = s.memory_delete(&original.id, 1, 1).unwrap();
    assert!(removed.chat_cleared);
    assert_eq!(removed.cleared_turns, 1);
    assert!(s.memory_list().unwrap().is_empty());
    assert!(s.load("a", "model").unwrap().is_empty());
    assert_eq!(s.load("b", "model").unwrap().len(), 1);
    assert_eq!(chats(&s), 1);
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
    assert_eq!(retry.cleared_turns, 0);
    assert_eq!(retry.context_epoch, 2);
    // The untouched foreign turn plus the two new ones all survive the retry.
    assert_eq!(chats(&s), 3);
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
    chat_using(
        &mut s,
        "a",
        &[ChatTurnUsage::app(original.id.clone(), original.revision)],
    );
    chat_using(&mut s, "b", &[]);
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

#[test]
fn personal_policy_set_mirrors_app_policy_semantics() {
    let mut store = store();
    let scope = MemoryScope {
        base_url: "https://api.test".into(),
        model: "m1".into(),
    };
    let change = |revision: i64, epoch: i64, enabled: bool, ids: Vec<i64>, restart: bool| {
        PersonalMemoryChange {
            expected_scope: scope.clone(),
            expected_revision: revision,
            expected_epoch: epoch,
            enabled,
            selected_ids: ids,
            restart_conversation: restart,
        }
    };

    // Fresh scope defaults to disabled/0/empty.
    assert_eq!(
        store.personal_memory_policy(&scope).unwrap(),
        PersonalMemoryPolicy::default()
    );

    // Wrong scope is rejected before anything else.
    let mut foreign = change(0, 0, true, vec![1], false);
    foreign.expected_scope = MemoryScope {
        base_url: "https://other".into(),
        model: "x".into(),
    };
    assert!(matches!(
        store
            .personal_memory_policy_set(&scope, &foreign)
            .unwrap_err(),
        StorageError::Memory(MemoryError::ContextChanged)
    ));

    // Structural validation: duplicates, non-positive ids, >5.
    for bad in [vec![1, 1], vec![0], vec![-3], vec![1, 2, 3, 4, 5, 6]] {
        assert!(matches!(
            store
                .personal_memory_policy_set(&scope, &change(0, 0, true, bad, false))
                .unwrap_err(),
            StorageError::Memory(_)
        ));
    }

    // Stale epoch and stale revision both refuse.
    assert!(matches!(
        store
            .personal_memory_policy_set(&scope, &change(0, 7, true, vec![1, 2], false))
            .unwrap_err(),
        StorageError::Memory(MemoryError::ContextChanged)
    ));
    assert!(matches!(
        store
            .personal_memory_policy_set(&scope, &change(5, 0, true, vec![1, 2], false))
            .unwrap_err(),
        StorageError::Memory(MemoryError::Conflict)
    ));

    // Enabling with selections bumps epoch, keeps chats.
    store
        .append_with_usage(
            &scope.base_url,
            &scope.model,
            &ChatTurn {
                user: "你好".into(),
                assistant: "在".into(),
            },
            &[ChatTurnUsage::personal(9, 1), ChatTurnUsage::personal(4, 1)],
        )
        .unwrap();
    let commit = store
        .personal_memory_policy_set(&scope, &change(0, 0, true, vec![9, 4], false))
        .unwrap();
    assert_eq!(commit.value.selected_ids, vec![9, 4]);
    assert_eq!(commit.context_epoch, 1);
    assert!(!commit.chat_cleared);
    assert_eq!(
        store.personal_memory_policy(&scope).unwrap().selected_ids,
        vec![9, 4]
    );

    // No-op write short-circuits without bumping the epoch.
    let again = store
        .personal_memory_policy_set(&scope, &change(1, 1, true, vec![9, 4], false))
        .unwrap();
    assert_eq!(again.context_epoch, 1);

    // Adding to the selection also keeps chats.
    store
        .append_with_usage(
            &scope.base_url,
            &scope.model,
            &ChatTurn {
                user: "二".into(),
                assistant: "轮".into(),
            },
            &[
                ChatTurnUsage::personal(9, 1),
                ChatTurnUsage::personal(4, 1),
                ChatTurnUsage::personal(7, 1),
            ],
        )
        .unwrap();
    let commit = store
        .personal_memory_policy_set(&scope, &change(1, 1, true, vec![9, 4, 7], false))
        .unwrap();
    assert!(!commit.chat_cleared);
    assert_eq!(commit.context_epoch, 2);

    // Removing requires the confirmation flag and then clears ALL scopes' chats.
    assert!(matches!(
        store
            .personal_memory_policy_set(&scope, &change(2, 2, true, vec![9], false))
            .unwrap_err(),
        StorageError::Memory(MemoryError::ConfirmationRequired)
    ));
    let other = MemoryScope {
        base_url: "https://api.test".into(),
        model: "m2".into(),
    };
    store
        .append_with_usage(
            &other.base_url,
            &other.model,
            &ChatTurn {
                user: "别".into(),
                assistant: "家".into(),
            },
            // m2 has no personal policy: this turn carried nothing.
            &[],
        )
        .unwrap();
    let commit = store
        .personal_memory_policy_set(&scope, &change(2, 2, true, vec![9], true))
        .unwrap();
    assert!(commit.chat_cleared);
    assert_eq!(commit.cleared_turns, 2);
    assert_eq!(commit.context_epoch, 3);
    // Precise: both m1 turns carried id 4 and go; the m2 turn carried
    // nothing and stays.
    assert!(store
        .load(&scope.base_url, &scope.model)
        .unwrap()
        .is_empty());
    assert_eq!(store.load(&other.base_url, &other.model).unwrap().len(), 1);

    // Disabling requires an empty selection (mirror of app policy).
    assert!(matches!(
        store
            .personal_memory_policy_set(&scope, &change(3, 3, false, vec![9], true))
            .unwrap_err(),
        StorageError::Memory(MemoryError::InvalidInput)
    ));
    let commit = store
        .personal_memory_policy_set(&scope, &change(3, 3, false, vec![], true))
        .unwrap();
    assert_eq!(
        commit.value,
        PersonalMemoryPolicy {
            enabled: false,
            revision: 4,
            selected_ids: vec![]
        }
    );
    assert_eq!(commit.context_epoch, 4);
    // Id 9's ledger rows cascaded away with its turns: disabling now prunes 0.
    assert!(!commit.chat_cleared);
    assert_eq!(commit.cleared_turns, 0);
    validate_schema(&store.0).unwrap();
}

#[test]
fn prune_is_a_suffix_per_scope_and_keeps_the_prior_prefix() {
    let mut s = store();
    let used = s.memory_create(&draft("used"), 0).unwrap().value;
    // Scope "a": an innocent prefix, the first carrier, then a follower that
    // never carried the item itself; scope "b" never used it.
    chat_using(&mut s, "a", &[]);
    chat_using(
        &mut s,
        "a",
        &[ChatTurnUsage::app(used.id.clone(), used.revision)],
    );
    chat_using(&mut s, "a", &[]);
    chat_using(&mut s, "b", &[]);
    let commit = s
        .memory_delete(&used.id, 1, s.context_epoch().unwrap())
        .unwrap();
    assert!(commit.chat_cleared);
    // Carrier and follower go (later turns quote earlier ones); prefix stays.
    assert_eq!(commit.cleared_turns, 2);
    assert_eq!(s.load("a", "model").unwrap(), vec![turn()]);
    assert_eq!(s.load("b", "model").unwrap(), vec![turn()]);
    assert_eq!(chats(&s), 2);
    // Pruned turns took their ledger rows with them (FK cascade).
    let usage: i64 =
        s.0.query_row("SELECT count(*) FROM chat_turn_usage", [], |r| r.get(0))
            .unwrap();
    assert_eq!(usage, 0);
    validate_schema(&s.0).unwrap();
}

#[test]
fn personal_prune_matches_ids_across_seqs_and_scopes() {
    let mut s = store();
    chat_using(&mut s, "a", &[ChatTurnUsage::personal(9, 1)]);
    chat_using(
        &mut s,
        "a",
        &[ChatTurnUsage::personal(9, 7), ChatTurnUsage::personal(4, 1)],
    );
    chat_using(&mut s, "b", &[ChatTurnUsage::personal(4, 3)]);
    let scope = MemoryScope {
        base_url: "a".into(),
        model: "model".into(),
    };
    let change = |revision: i64, epoch: i64, ids: Vec<i64>| PersonalMemoryChange {
        expected_scope: scope.clone(),
        expected_revision: revision,
        expected_epoch: epoch,
        enabled: true,
        selected_ids: ids,
        restart_conversation: true,
    };
    s.personal_memory_policy_set(&scope, &change(0, 0, vec![9, 4]))
        .unwrap();
    // Removing id 9 must reach both of its turns regardless of recorded seq,
    // while id 4's turn in scope "b" stays selected and untouched.
    let commit = s
        .personal_memory_policy_set(&scope, &change(1, 1, vec![4]))
        .unwrap();
    assert!(commit.chat_cleared);
    assert_eq!(commit.cleared_turns, 2);
    assert!(s.load("a", "model").unwrap().is_empty());
    assert_eq!(s.load("b", "model").unwrap(), vec![turn()]);
    validate_schema(&s.0).unwrap();
}

#[test]
fn deselect_spares_scopes_that_still_select_the_item() {
    let mut s = store();
    let shared = s.memory_create(&draft("shared"), 0).unwrap().value;
    // Both scopes select the item and both archives carry a turn on it.
    select(&s, "a", std::slice::from_ref(&shared));
    select(&s, "b", std::slice::from_ref(&shared));
    chat_using(
        &mut s,
        "a",
        &[ChatTurnUsage::app(shared.id.clone(), shared.revision)],
    );
    chat_using(
        &mut s,
        "b",
        &[ChatTurnUsage::app(shared.id.clone(), shared.revision)],
    );
    let scope = MemoryScope {
        base_url: "a".into(),
        model: "model".into(),
    };
    // Dropping the selection in scope "a" prunes only "a": scope "b" still
    // injects the item every turn, so deleting its history would remove
    // nothing the next request will not carry again.
    let commit = s
        .memory_policy_set(
            &scope,
            &MemoryPolicyChange {
                expected_scope: scope.clone(),
                expected_revision: 1,
                expected_epoch: 1,
                enabled: false,
                selected_ids: vec![],
                restart_conversation: true,
            },
        )
        .unwrap();
    assert!(commit.chat_cleared);
    assert_eq!(commit.cleared_turns, 1);
    assert!(s.load("a", "model").unwrap().is_empty());
    assert_eq!(s.load("b", "model").unwrap(), vec![turn()]);
    // A real delete still reaches scope "b": the ledger rows survived.
    let removed = s.memory_delete(&shared.id, 1, 2).unwrap();
    assert_eq!(removed.cleared_turns, 1);
    assert!(s.load("b", "model").unwrap().is_empty());
    validate_schema(&s.0).unwrap();
}

#[test]
fn personal_deselect_spares_still_selecting_scopes() {
    let mut s = store();
    chat_using(&mut s, "a", &[ChatTurnUsage::personal(9, 1)]);
    chat_using(&mut s, "b", &[ChatTurnUsage::personal(9, 1)]);
    let select_both = |scope: &MemoryScope, revision: i64, epoch: i64| PersonalMemoryChange {
        expected_scope: scope.clone(),
        expected_revision: revision,
        expected_epoch: epoch,
        enabled: true,
        selected_ids: vec![9],
        restart_conversation: false,
    };
    let a = MemoryScope {
        base_url: "a".into(),
        model: "model".into(),
    };
    let b = MemoryScope {
        base_url: "b".into(),
        model: "model".into(),
    };
    s.personal_memory_policy_set(&a, &select_both(&a, 0, 0))
        .unwrap();
    s.personal_memory_policy_set(&b, &select_both(&b, 0, 1))
        .unwrap();
    let commit = s
        .personal_memory_policy_set(
            &a,
            &PersonalMemoryChange {
                expected_scope: a.clone(),
                expected_revision: 1,
                expected_epoch: 2,
                enabled: false,
                selected_ids: vec![],
                restart_conversation: true,
            },
        )
        .unwrap();
    assert_eq!(commit.cleared_turns, 1);
    assert!(s.load("a", "model").unwrap().is_empty());
    // Scope "b" still selects id 9 and keeps both the turn and the policy.
    assert_eq!(s.load("b", "model").unwrap(), vec![turn()]);
    assert_eq!(s.personal_memory_policy(&b).unwrap().selected_ids, vec![9]);
    validate_schema(&s.0).unwrap();
}

#[test]
fn usage_impact_reports_per_scope_suffix_counts_without_touching_rows() {
    let mut s = store();
    let used = s.memory_create(&draft("used"), 0).unwrap().value;
    chat_using(&mut s, "a", &[]); // prefix: would survive
    chat_using(
        &mut s,
        "a",
        &[ChatTurnUsage::app(used.id.clone(), used.revision)],
    );
    chat_using(&mut s, "a", &[]); // follower: would go with the carrier
    chat_using(&mut s, "b", &[]);
    let report = s.usage_impact(&[used.id], &[]).unwrap();
    assert_eq!(report.affected_turns_total, 2);
    assert_eq!(report.scopes.len(), 1);
    assert_eq!(report.scopes[0].scope.base_url, "a");
    assert_eq!(report.scopes[0].scope.model, "model");
    assert_eq!(report.scopes[0].affected_turns, 2);
    assert_eq!(report.scopes[0].kept_turns, 1);
    // Unknown ids and empty sets report nothing.
    let empty = s.usage_impact(&[], &[99]).unwrap();
    assert_eq!(empty.affected_turns_total, 0);
    assert!(empty.scopes.is_empty());
    // Consultative only: nothing was pruned.
    assert_eq!(chats(&s), 4);
}

#[test]
fn personal_policy_schema_corruption_is_rejected() {
    let mut store = store();
    let scope = MemoryScope {
        base_url: "https://api.test".into(),
        model: "m1".into(),
    };
    store
        .personal_memory_policy_set(
            &scope,
            &PersonalMemoryChange {
                expected_scope: scope.clone(),
                expected_revision: 0,
                expected_epoch: 0,
                enabled: true,
                selected_ids: vec![5],
                restart_conversation: false,
            },
        )
        .unwrap();
    // Enabled policy with a dangling (removed) selection row breaks the invariant.
    store
        .0
        .execute("DELETE FROM personal_memory_selection", [])
        .unwrap();
    assert!(validate_schema(&store.0).is_err());
}

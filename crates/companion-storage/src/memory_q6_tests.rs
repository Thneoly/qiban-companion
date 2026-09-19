//! Fixed Q6 storage cases. Crash injection is compiled only in test binaries.
use super::*;
use companion_core::conversation::ChatTurn;
fn draft(body: &str) -> MemoryDraft {
    MemoryDraft {
        kind: MemoryKind::Preference,
        body: body.into(),
        event_date: None,
    }
}
fn scope() -> MemoryScope {
    MemoryScope {
        base_url: "https://q6.invalid".into(),
        model: "fixture".into(),
    }
}
fn fixture() -> (HistoryStore, Memory) {
    let mut db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch("CREATE TABLE chat_turns(id INTEGER PRIMARY KEY,base TEXT NOT NULL,model TEXT NOT NULL,user TEXT NOT NULL,assistant TEXT NOT NULL);").unwrap();
    tx.execute_batch(include_str!("memory-schema.sql")).unwrap();
    tx.commit().unwrap();
    let mut store = HistoryStore(db);
    let memory = store.memory_create(&draft("Q6 合成正文"), 0).unwrap().value;
    policy(&mut store, vec![memory.id.clone()], true);
    append(&mut store);
    (store, memory)
}
fn append(s: &mut HistoryStore) {
    for base in ["https://q6.invalid", "https://other.invalid"] {
        s.append(
            base,
            "fixture",
            &ChatTurn {
                user: "Q6 合成问题".into(),
                assistant: "Q6 合成旧回答".into(),
            },
        )
        .unwrap();
    }
}
fn chats(s: &HistoryStore) -> i64 {
    s.0.query_row("SELECT count(*) FROM chat_turns", [], |r| r.get(0))
        .unwrap()
}
fn policy(s: &mut HistoryStore, ids: Vec<String>, confirm: bool) -> MemoryCommit<MemoryPolicy> {
    s.memory_policy_set(
        &scope(),
        &MemoryPolicyChange {
            expected_scope: scope(),
            expected_epoch: s.context_epoch().unwrap(),
            expected_revision: s.memory_policy(&scope()).unwrap().revision,
            enabled: !ids.is_empty(),
            selected_ids: ids,
            restart_conversation: confirm,
        },
    )
    .unwrap()
}

#[test]
fn q6_del01_single_delete() {
    let (mut s, m) = fixture();
    let epoch = s.context_epoch().unwrap();
    assert!(s.memory_delete(&m.id, 1, epoch).unwrap().chat_cleared);
    assert_eq!(chats(&s), 0);
    assert!(s.memory_list().unwrap().is_empty());
    let content: Option<String> =
        s.0.query_row("SELECT body FROM memories WHERE id=?1", [m.id], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(content.is_none());
    assert!(!s.memory_policy(&scope()).unwrap().enabled);
}
#[test]
fn q6_del02_delete_all() {
    let (mut s, _) = fixture();
    s.memory_create(&draft("second"), s.context_epoch().unwrap())
        .unwrap();
    assert!(
        s.memory_delete_all(s.context_epoch().unwrap())
            .unwrap()
            .chat_cleared
    );
    assert!(s.memory_list().unwrap().is_empty());
    assert_eq!(chats(&s), 0);
    assert!(s.memory_policy(&scope()).unwrap().selected_ids.is_empty());
}
#[test]
fn q6_del03_retry_preserves_new_chat() {
    let (mut s, m) = fixture();
    let epoch = s.context_epoch().unwrap();
    s.memory_delete(&m.id, 1, epoch).unwrap();
    append(&mut s);
    assert!(!s.memory_delete(&m.id, 1, epoch).unwrap().chat_cleared);
    assert_eq!(chats(&s), 2);
}
#[test]
fn q6_del04_stale_all_preserves_new_memory() {
    let (mut s, _) = fixture();
    let epoch = s.context_epoch().unwrap();
    s.memory_delete_all(epoch).unwrap();
    s.memory_create(&draft("new"), s.context_epoch().unwrap())
        .unwrap();
    append(&mut s);
    assert!(s.memory_delete_all(epoch).is_err());
    assert_eq!(s.memory_list().unwrap()[0].body, "new");
    assert_eq!(chats(&s), 2);
}
#[test]
fn q6_del05_correct_clears_all_scopes() {
    let (mut s, m) = fixture();
    s.memory_update(&m.id, 1, s.context_epoch().unwrap(), &draft("corrected"))
        .unwrap();
    assert_eq!(chats(&s), 0);
    let new = &s.memory_list().unwrap()[0];
    assert_eq!(new.body, "corrected");
    assert_eq!(new.revision, 2);
    assert_eq!(s.memory_policy(&scope()).unwrap().selected_ids, vec![m.id]);
}
#[test]
fn q6_del06_disable_keeps_memory() {
    let (mut s, m) = fixture();
    let r = policy(&mut s, vec![], true);
    assert!(r.chat_cleared);
    assert_eq!(chats(&s), 0);
    assert_eq!(s.memory_list().unwrap(), vec![m]);
    assert!(!s.memory_policy(&scope()).unwrap().enabled);
}
#[test]
fn q6_del07_remove_one_selection() {
    let (mut s, m) = fixture();
    let other = s
        .memory_create(&draft("other"), s.context_epoch().unwrap())
        .unwrap()
        .value;
    policy(&mut s, vec![m.id, other.id.clone()], true);
    let r = policy(&mut s, vec![other.id.clone()], true);
    assert!(r.chat_cleared);
    assert_eq!(chats(&s), 0);
    assert_eq!(
        s.memory_policy(&scope()).unwrap().selected_ids,
        vec![other.id]
    );
    assert_eq!(s.memory_list().unwrap().len(), 2);
}
#[test]
fn q6_del08_clear_chat_keeps_memory_and_policy() {
    let (mut s, m) = fixture();
    let p = s.memory_policy(&scope()).unwrap();
    let epoch = s.context_epoch().unwrap();
    s.clear().unwrap();
    assert_eq!(chats(&s), 0);
    assert_eq!(s.memory_list().unwrap(), vec![m]);
    assert_eq!(s.memory_policy(&scope()).unwrap(), p);
    assert_eq!(s.context_epoch().unwrap(), epoch + 1);
}

pub(super) fn crash_before_commit() {
    if std::env::var("QIBAN_Q6_CRASH").as_deref() == Ok("before-commit") {
        std::process::exit(23);
    }
}
#[test]
fn crash_child() {
    let Ok(path) = std::env::var("QIBAN_Q6_CHILD_DB") else {
        return;
    };
    let mut s = HistoryStore::open(std::path::Path::new(&path)).unwrap();
    let m = s.memory_list().unwrap().remove(0);
    s.memory_delete(&m.id, m.revision, s.context_epoch().unwrap())
        .unwrap();
    panic!("crash barrier not reached");
}
#[test]
fn q6_del13_crash_before_commit_rolls_back() {
    let path = std::env::temp_dir().join(format!("q6-before-{}.db", uuid::Uuid::new_v4()));
    let mut s = HistoryStore::open(&path).unwrap();
    let m = s.memory_create(&draft("survives crash"), 0).unwrap().value;
    policy(&mut s, vec![m.id.clone()], true);
    append(&mut s);
    let epoch = s.context_epoch().unwrap();
    drop(s);
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "memory::q6_tests::crash_child", "--nocapture"])
        .env("QIBAN_Q6_CHILD_DB", &path)
        .env("QIBAN_Q6_CRASH", "before-commit")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(23));
    let s = HistoryStore::open(&path).unwrap();
    assert_eq!(s.context_epoch().unwrap(), epoch);
    assert_eq!(s.memory_list().unwrap(), vec![m]);
    assert_eq!(chats(&s), 2);
    assert!(s.memory_policy(&scope()).unwrap().enabled);
    drop(s);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn q6_del15_read_only_failure_keeps_data() {
    let (mut s, m) = fixture();
    let epoch = s.context_epoch().unwrap();
    s.0.pragma_update(None, "query_only", true).unwrap();
    assert!(s.memory_delete(&m.id, 1, epoch).is_err());
    assert_eq!(s.memory_list().unwrap(), vec![m]);
    assert_eq!(chats(&s), 2);
    assert_eq!(s.context_epoch().unwrap(), epoch);
}
#[test]
fn q6_del19_future_and_corrupt_files_untouched() {
    for corrupt in [false, true] {
        let path = std::env::temp_dir().join(format!("q6-refuse-{}.db", uuid::Uuid::new_v4()));
        if corrupt {
            std::fs::write(&path, b"not a sqlite database").unwrap();
        } else {
            let db = Connection::open(&path).unwrap();
            db.pragma_update(None, "user_version", 999).unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        assert!(HistoryStore::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(path).unwrap();
    }
}

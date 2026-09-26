use super::*;
fn identity(subject: &str, session: &str) -> VerifiedIdentity {
    VerifiedIdentity::from_verified_provider("test", subject, session, 100, 10_000_000).unwrap()
}
fn fixture() -> AccountStore {
    let mut s = AccountStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
    s.clock = || 1000;
    s
}
fn connected(s: &AccountStore) -> (VerifiedIdentity, VerifiedIdentity, Pairing) {
    let d = identity("alice", "desktop");
    let m = identity("alice", "mobile");
    let o = s.offer_pairing(&d, "我的电脑").unwrap();
    let p = s
        .accept_pairing(&m, &o.code, &o.pairing.id, "我的手机")
        .unwrap();
    (d, m, p)
}
fn binding(p: &Pairing) -> ActionBinding {
    ActionBinding {
        action_id: uuid::Uuid::new_v4().to_string(),
        resource_id: uuid::Uuid::new_v4().to_string(),
        resource_version: 1,
        parameters_digest: "a".repeat(64),
        pair_revision: p.revision,
        scope: ActionScope::DocumentExcerpt,
    }
}
#[test]
fn pairing_is_owner_bound_one_time_and_needs_another_session() {
    let s = fixture();
    let d = identity("alice", "d");
    let m = identity("alice", "m");
    let b = identity("bob", "b");
    let old = s.offer_pairing(&d, "电脑").unwrap();
    let o = s.offer_pairing(&d, "电脑").unwrap();
    assert!(s.preview_pairing(&m, &old.code).is_err());
    assert!(s.preview_pairing(&d, &o.code).is_err());
    assert!(s.preview_pairing(&b, &o.code).is_err());
    assert!(s.accept_pairing(&m, &o.code, "wrong", "手机").is_err());
    let preview = s.preview_pairing(&m, &o.code).unwrap();
    assert_eq!(preview.desktop_name, "电脑");
    let p = s.accept_pairing(&m, &o.code, &preview.id, "手机").unwrap();
    assert_eq!(p.status, "active");
    assert!(s.accept_pairing(&m, &o.code, &p.id, "手机").is_err());
    assert!(s.list_pairings(&b).unwrap().is_empty());
    assert!(s.revoke_pairing(&b, &p.id, p.revision).is_err());
    let db = s.connection.lock().unwrap();
    let hashes: Vec<Option<String>> = db
        .prepare("SELECT code_hash FROM device_pairings")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(hashes.iter().all(Option::is_none));
}
#[test]
fn expiration_logout_and_new_login_never_restore_old_pairing() {
    let mut s = fixture();
    let d = identity("alice", "d");
    let m = identity("alice", "m");
    let o = s.offer_pairing(&d, "电脑").unwrap();
    s.clock = || 301000;
    assert!(s.preview_pairing(&m, &o.code).is_err());
    s.clock = || 1000;
    let (d, m, p) = connected(&s);
    s.sign_out(&d, false).unwrap();
    assert_eq!(s.list_pairings(&m).unwrap()[0].status, "expired");
    let fresh = identity("alice", "fresh");
    let mut b = binding(&p);
    b.resource_id = uuid::Uuid::new_v4().to_string();
    assert!(s.prepare_action(&fresh, &p.id, &b).is_err());
}
#[test]
fn exact_confirmation_single_admission_and_revocation_distinguish_inflight() {
    let s = fixture();
    let (d, m, p) = connected(&s);
    let b = binding(&p);
    assert!(s.prepare_action(&d, &p.id, &b).is_err());
    s.set_authorized_resource(&d, &p.id, &b.resource_id, 1, true)
        .unwrap();
    s.prepare_action(&d, &p.id, &b).unwrap();
    assert!(s.admit_action(&d, &b).is_err());
    assert!(s.confirm_action(&d, &b).is_err());
    let mut changed = b.clone();
    changed.parameters_digest = "b".repeat(64);
    assert!(s.confirm_action(&m, &changed).is_err());
    s.confirm_action(&m, &b).unwrap();
    assert!(s.confirm_action(&m, &b).is_err());
    assert_eq!(s.admit_action(&d, &b).unwrap().state, "admitted");
    assert!(s.admit_action(&d, &b).is_err());
    let mut queued = b.clone();
    queued.action_id = uuid::Uuid::new_v4().to_string();
    s.prepare_action(&d, &p.id, &queued).unwrap();
    s.confirm_action(&m, &queued).unwrap();
    s.revoke_pairing(&m, &p.id, p.revision).unwrap();
    assert_eq!(
        s.get_action_authorization(&d, &b.action_id).unwrap().state,
        "cancel_requested"
    );
    assert_eq!(
        s.get_action_authorization(&d, &queued.action_id)
            .unwrap()
            .state,
        "cancelled"
    );
    assert!(s.admit_action(&d, &queued).is_err());
    assert!(s.prepare_action(&d, &p.id, &queued).is_err());
}
#[test]
fn resource_change_expiry_and_unpaired_session_invalidate_confirmation() {
    let mut s = fixture();
    let (d, m, p) = connected(&s);
    let mut b = binding(&p);
    s.set_authorized_resource(&d, &p.id, &b.resource_id, 1, true)
        .unwrap();
    s.prepare_action(&d, &p.id, &b).unwrap();
    s.confirm_action(&m, &b).unwrap();
    s.set_authorized_resource(&d, &p.id, &b.resource_id, 2, true)
        .unwrap();
    assert!(s.admit_action(&d, &b).is_err());
    assert!(s
        .set_authorized_resource(&d, &p.id, &b.resource_id, 1, true)
        .is_err());
    b.action_id = uuid::Uuid::new_v4().to_string();
    b.resource_version = 2;
    s.prepare_action(&d, &p.id, &b).unwrap();
    assert!(s.confirm_action(&identity("alice", "other"), &b).is_err());
    assert!(s
        .get_action_authorization(&identity("bob", "other"), &b.action_id)
        .is_err());
    s.clock = || 301000;
    assert!(s.confirm_action(&m, &b).is_err());
    assert_eq!(
        s.get_action_authorization(&d, &b.action_id).unwrap().state,
        "cancelled"
    );
}
#[test]
fn migration_preserves_identity_tasks_and_pairing_survives_reopen() {
    let path = std::env::temp_dir().join(format!("qiban-pairing-{}.db", uuid::Uuid::new_v4()));
    let d = identity("alice", "desktop");
    let m = identity("alice", "mobile");
    let profile;
    let pid;
    {
        let mut s = AccountStore::open(&path).unwrap();
        s.clock = || 1000;
        profile = s.profile(&d).unwrap();
        s.create_task(
            &d,
            &CreateAccountTask {
                request_id: uuid::Uuid::new_v4().to_string(),
                title: "保留".into(),
            },
        )
        .unwrap();
        s.connection.lock().unwrap().execute_batch("DROP TABLE shared_documents;DROP TABLE paired_devices;DROP TABLE device_pairings;DROP TABLE action_authorizations;DROP TABLE authorized_resources;PRAGMA user_version=2;").unwrap();
    }
    {
        let mut s = AccountStore::open(&path).unwrap();
        s.clock = || 1000;
        assert_eq!(s.profile(&d).unwrap(), profile);
        assert_eq!(s.list_tasks(&d).unwrap().len(), 1);
        let o = s.offer_pairing(&d, "电脑").unwrap();
        pid = s
            .accept_pairing(&m, &o.code, &o.pairing.id, "手机")
            .unwrap()
            .id;
        s.connection
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE shared_documents; PRAGMA user_version=3;")
            .unwrap();
    }
    {
        let mut s = AccountStore::open(&path).unwrap();
        s.clock = || 1000;
        let p = s.list_pairings(&d).unwrap().remove(0);
        assert_eq!(p.id, pid);
        assert_eq!(p.status, "active");
        s.revoke_pairing(&d, &pid, p.revision).unwrap();
    }
    {
        let mut s = AccountStore::open(&path).unwrap();
        s.clock = || 1000;
        assert_eq!(s.list_pairings(&d).unwrap()[0].status, "revoked");
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn concurrent_revoke_and_admit_have_one_durable_order() {
    let s = std::sync::Arc::new(fixture());
    let (d, m, p) = connected(&s);
    let b = binding(&p);
    s.set_authorized_resource(&d, &p.id, &b.resource_id, 1, true)
        .unwrap();
    s.prepare_action(&d, &p.id, &b).unwrap();
    s.confirm_action(&m, &b).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let s2 = s.clone();
    let b2 = b.clone();
    let barrier2 = barrier.clone();
    let admit = std::thread::spawn(move || {
        barrier2.wait();
        s2.admit_action(&identity("alice", "desktop"), &b2).is_ok()
    });
    barrier.wait();
    s.revoke_pairing(&m, &p.id, p.revision).unwrap();
    let admitted = admit.join().unwrap();
    assert_eq!(
        s.get_action_authorization(&d, &b.action_id).unwrap().state,
        if admitted {
            "cancel_requested"
        } else {
            "cancelled"
        }
    );
    assert!(s.admit_action(&d, &b).is_err());
}

#[test]
fn shared_document_confirmation_receipt_and_cancellation_are_exact() {
    use companion_core::authorization::{document_digest, ShareDocument};
    let mut s = fixture();
    let (d, m, p) = connected(&s);
    let mut b = binding(&p);
    let preview = "# fixed excerpt".to_string();
    b.parameters_digest = document_digest("demo.txt", &preview);
    let request = ShareDocument {
        pairing_id: p.id.clone(),
        binding: b.clone(),
        source_name: "demo.txt".into(),
        preview,
    };
    let first = s.share_document(&d, &request).unwrap();
    assert_eq!(
        s.share_document(&d, &request)
            .unwrap()
            .authorization
            .binding,
        b
    );
    assert!(s.share_document(&m, &request).is_err());
    assert!(s
        .list_documents(&identity("alice", "outsider"))
        .unwrap()
        .is_empty());
    assert!(s
        .document_action(&identity("bob", "outsider"), &b.action_id)
        .is_err());
    assert!(s
        .receipt_document(&d, &b, "completed", &first.artifact_hash)
        .is_err());
    s.confirm_action(&m, &b).unwrap();
    s.admit_action(&d, &b).unwrap();
    assert!(s
        .receipt_document(&m, &b, "completed", &first.artifact_hash)
        .is_err());
    assert!(s
        .receipt_document(&d, &b, "completed", &"b".repeat(64))
        .is_err());
    s.receipt_document(&d, &b, "completed", &first.artifact_hash)
        .unwrap();
    assert_eq!(
        s.receipt_document(&d, &b, "completed", &first.artifact_hash)
            .unwrap()
            .authorization
            .state,
        "completed"
    );
    let mut cancelled = request.clone();
    cancelled.binding.action_id = uuid::Uuid::new_v4().to_string();
    cancelled.binding.resource_id = uuid::Uuid::new_v4().to_string();
    s.share_document(&d, &cancelled).unwrap();
    s.confirm_action(&m, &cancelled.binding).unwrap();
    s.cancel_document(&m, &cancelled.binding.action_id).unwrap();
    assert!(s.admit_action(&d, &cancelled.binding).is_err());
    s.revoke_pairing(&d, &p.id, p.revision).unwrap();
    s.clock = || 301001;
    assert_eq!(
        s.document_action(&m, &b.action_id)
            .unwrap()
            .authorization
            .state,
        "completed"
    );
}
#[test]
fn shared_document_rejects_changed_preview_and_rolls_back_partial_writes() {
    use companion_core::authorization::{document_digest, ShareDocument};
    let s = fixture();
    let (d, _, p) = connected(&s);
    let mut b = binding(&p);
    b.parameters_digest = document_digest("demo.txt", "preview");
    let mut request = ShareDocument {
        pairing_id: p.id,
        binding: b.clone(),
        source_name: "demo.txt".into(),
        preview: "changed".into(),
    };
    assert!(s.share_document(&d, &request).is_err());
    request.preview = "preview".into();
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_shared BEFORE INSERT ON shared_documents BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(s.share_document(&d, &request).is_err());
    assert!(s.get_action_authorization(&d, &b.action_id).is_err());
    s.connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_shared;")
        .unwrap();
    s.share_document(&d, &request).unwrap();
    request.preview = "other".into();
    request.binding.parameters_digest = document_digest("demo.txt", "other");
    assert!(s.share_document(&d, &request).is_err());
}

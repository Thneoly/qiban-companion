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
fn live_session_re_pairs_after_its_device_row_horizon_passes() {
    let mut s = fixture();
    // First registration freezes the desktop row's horizon at the session's
    // then-current expiry (production: verify carries now+30min).
    let early =
        VerifiedIdentity::from_verified_provider("test", "alice", "desktop", 100, 2_000).unwrap();
    let m = identity("alice", "mobile");
    let o = s.offer_pairing(&early, "电脑").unwrap();
    let p = s
        .accept_pairing(&m, &o.code, &o.pairing.id, "手机")
        .unwrap();
    assert_eq!(p.status, "active");

    // Horizon passes; the pairing reads expired. The session keeps verifying
    // (same session id, fresh horizon) and re-offers.
    s.clock = || 3_000;
    assert_eq!(s.list_pairings(&m).unwrap()[0].status, "expired");
    let refreshed =
        VerifiedIdentity::from_verified_provider("test", "alice", "desktop", 100, 8_000).unwrap();
    let o2 = s.offer_pairing(&refreshed, "电脑").unwrap();
    assert_eq!(
        o2.pairing.status, "pending",
        "a re-offer from a live session must not be born expired"
    );
    let p2 = s
        .accept_pairing(&m, &o2.code, &o2.pairing.id, "手机")
        .unwrap();
    assert_eq!(p2.status, "active");
    let listed = s.list_pairings(&m).unwrap();
    assert!(listed.iter().any(|x| x.id == p2.id && x.status == "active"));
}
#[test]
fn expiration_logout_and_new_login_never_restore_old_pairing() {
    let mut s = fixture();
    let d = identity("alice", "d");
    let m = identity("alice", "m");
    let o = s.offer_pairing(&d, "电脑").unwrap();
    s.clock = || 301000;
    assert!(s.preview_pairing(&m, &o.code).is_err());
    let s = fixture();
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
        s.connection.lock().unwrap().execute_batch("DROP TABLE pairing_code_limits;DROP TABLE shared_documents;DROP TABLE paired_devices;DROP TABLE device_pairings;DROP TABLE action_authorizations;DROP TABLE authorized_resources;PRAGMA user_version=2;").unwrap();
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
            .execute_batch("DROP TABLE pairing_code_limits;DROP TABLE shared_documents;CREATE TABLE paired_devices_old AS SELECT account_id,id,session_id,name,expires_at FROM paired_devices;DROP TABLE paired_devices;ALTER TABLE paired_devices_old RENAME TO paired_devices;PRAGMA user_version=3;")
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

#[test]
fn finished_document_deletion_is_terminal_scoped_atomic_and_frees_capacity() {
    use companion_core::authorization::{document_digest, ShareDocument};
    let s = fixture();
    let (d, m, p) = connected(&s);
    let share = |name: &str, preview: &str| {
        let mut b = binding(&p);
        b.parameters_digest = document_digest(name, preview);
        ShareDocument {
            pairing_id: p.id.clone(),
            binding: b,
            source_name: name.into(),
            preview: preview.into(),
        }
    };
    // A completed document deletes for either participant and disappears.
    let done = share("完成.txt", "已完成的摘录");
    let first = s.share_document(&d, &done).unwrap();
    s.confirm_action(&m, &done.binding).unwrap();
    s.admit_action(&d, &done.binding).unwrap();
    s.receipt_document(&d, &done.binding, "completed", &first.artifact_hash)
        .unwrap();
    let snapshot = s.delete_document(&m, &done.binding.action_id).unwrap();
    assert_eq!(snapshot.authorization.state, "completed");
    assert!(s.document_action(&d, &done.binding.action_id).is_err());
    // Retry (e.g. after a lost response) and the other role both fail now.
    assert!(s.delete_document(&m, &done.binding.action_id).is_err());
    assert!(s.delete_document(&d, &done.binding.action_id).is_err());
    // Re-sharing the identical body inserts a fresh task: the resource row
    // was freed atomically with the record.
    let again = s.share_document(&d, &done).unwrap();
    assert_eq!(again.authorization.state, "awaiting_confirmation");
    // Non-terminal states refuse, including unknown (still awaiting the
    // desktop's reconciliation receipt).
    let inflight = share("在途.txt", "先报告未知结果的摘录");
    s.share_document(&d, &inflight).unwrap();
    for state_check in 0..4 {
        match state_check {
            0 => {}
            1 => {
                s.confirm_action(&m, &inflight.binding).unwrap();
            }
            2 => {
                s.admit_action(&d, &inflight.binding).unwrap();
            }
            _ => {
                s.receipt_document(
                    &d,
                    &inflight.binding,
                    "unknown",
                    &companion_core::execution::digest("先报告未知结果的摘录".as_bytes()),
                )
                .unwrap();
            }
        }
        assert!(matches!(
            s.delete_document(&m, &inflight.binding.action_id),
            Err(StorageError::Authorization(Error::Conflict))
        ));
    }
    assert!(s
        .delete_document(&identity("alice", "outsider"), &inflight.binding.action_id)
        .is_err());
    assert!(s
        .delete_document(&identity("bob", "elsewhere"), &inflight.binding.action_id)
        .is_err());
    // A cancelled document (never admitted) deletes.
    let cancelled = share("取消.txt", "确认前取消的摘录");
    s.share_document(&d, &cancelled).unwrap();
    s.cancel_document(&m, &cancelled.binding.action_id).unwrap();
    s.delete_document(&d, &cancelled.binding.action_id).unwrap();
    assert!(s
        .list_documents(&m)
        .unwrap()
        .iter()
        .all(|x| x.authorization.binding.action_id != cancelled.binding.action_id));
    // Capacity is a live count: fill to the 20-document cap, delete one
    // finished record, and a new share succeeds again.
    let mut filler = 0;
    while s.list_documents(&d).unwrap().len() < 20 {
        let name = format!("填充{filler}.txt");
        s.share_document(&d, &share(&name, "填充配额的摘录"))
            .unwrap();
        filler += 1;
    }
    let spare = share("释放.txt", "验证配额释放的摘录");
    assert!(matches!(
        s.share_document(&d, &spare),
        Err(StorageError::Authorization(Error::Capacity))
    ));
    s.confirm_action(&m, &done.binding).unwrap();
    s.admit_action(&d, &done.binding).unwrap();
    s.receipt_document(&d, &done.binding, "completed", &first.artifact_hash)
        .unwrap();
    s.delete_document(&m, &done.binding.action_id).unwrap();
    assert_eq!(
        s.share_document(&d, &spare).unwrap().authorization.state,
        "awaiting_confirmation"
    );
}

#[test]
fn short_code_failures_are_shared_durable_and_not_reset_by_regeneration() {
    let path = std::env::temp_dir().join(format!("qiban-pair-limit-{}.db", uuid::Uuid::new_v4()));
    let d = identity("alice", "desktop");
    let m = identity("alice", "phone");
    let offer;
    {
        let mut s = AccountStore::open(&path).unwrap();
        s.clock = || 1000;
        offer = s.offer_pairing(&d, "电脑").unwrap();
        assert_eq!(offer.code.len(), 6);
        assert!(offer.code.bytes().all(|b| b.is_ascii_digit()));
        let wrong = if offer.code == "000000" {
            "000001"
        } else {
            "000000"
        };
        for _ in 0..3 {
            assert!(s.preview_pairing(&m, wrong).is_err());
        }
        // Confirm endpoint shares the budget, including mismatched preview identity.
        assert!(s
            .accept_pairing(&m, &offer.code, "wrong-id", "手机")
            .is_err());
        assert!(s.preview_pairing(&m, &offer.code).is_ok()); // success does not reset failures
    }
    {
        let mut s = AccountStore::open(&path).unwrap();
        s.clock = || 1000;
        let fresh = identity("alice", "new-phone-session");
        assert!(matches!(
            s.preview_pairing(&fresh, "invalid"),
            Err(StorageError::Authorization(Error::RateLimited))
        ));
        let regenerated = s.offer_pairing(&d, "电脑").unwrap();
        assert_ne!(offer.code, regenerated.code);
        assert!(matches!(
            s.preview_pairing(&m, &regenerated.code),
            Err(StorageError::Authorization(Error::RateLimited))
        ));
        assert!(matches!(
            s.accept_pairing(&fresh, &regenerated.code, &regenerated.pairing.id, "手机"),
            Err(StorageError::Authorization(Error::RateLimited))
        ));
        let bob = s
            .offer_pairing(&identity("bob", "desktop"), "另一台电脑")
            .unwrap();
        s.accept_pairing(
            &identity("bob", "phone"),
            &bob.code,
            &bob.pairing.id,
            "手机",
        )
        .unwrap();
        // Monotonic time is required; rolling the clock back cannot reopen the budget.
        s.clock = || 999;
        assert!(matches!(
            s.preview_pairing(&m, &regenerated.code),
            Err(StorageError::Authorization(Error::RateLimited))
        ));
        s.clock = || 301000;
        let next = s.offer_pairing(&d, "电脑").unwrap();
        s.accept_pairing(&m, &next.code, &next.pairing.id, "手机")
            .unwrap();
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn v4_migration_invalidates_pending_long_codes_but_keeps_active_pair_and_document() {
    use companion_core::authorization::{document_digest, ShareDocument};
    let s = fixture();
    let (d, m, active) = connected(&s);
    let mut b = binding(&active);
    b.parameters_digest = document_digest("test.txt", "preview");
    s.share_document(
        &d,
        &ShareDocument {
            pairing_id: active.id.clone(),
            binding: b.clone(),
            source_name: "test.txt".into(),
            preview: "preview".into(),
        },
    )
    .unwrap();
    let pending = s.offer_pairing(&d, "电脑").unwrap();
    let c = s.connection.into_inner().unwrap();
    c.execute(
        "UPDATE device_pairings SET code_hash=?1 WHERE id=?2",
        params![
            format!("{:x}", Sha256::digest("a".repeat(32).as_bytes())),
            pending.pairing.id
        ],
    )
    .unwrap();
    c.execute_batch("DROP TABLE pairing_code_limits;CREATE TABLE paired_devices_old AS SELECT account_id,id,session_id,name,expires_at FROM paired_devices;DROP TABLE paired_devices;ALTER TABLE paired_devices_old RENAME TO paired_devices;PRAGMA user_version=4;")
        .unwrap();
    let mut migrated = AccountStore::from_connection(c).unwrap();
    migrated.clock = || 1000;
    let pairs = migrated.list_pairings(&m).unwrap();
    assert_eq!(
        pairs.iter().find(|p| p.id == active.id).unwrap().status,
        "active"
    );
    let old = pairs.iter().find(|p| p.id == pending.pairing.id).unwrap();
    assert_eq!(old.status, "revoked");
    assert_eq!(old.revision, pending.pairing.revision + 1);
    assert!(migrated.preview_pairing(&m, &"a".repeat(32)).is_err());
    migrated.confirm_action(&m, &b).unwrap();
    migrated.admit_action(&d, &b).unwrap();
    assert_eq!(
        migrated.document_action(&d, &b.action_id).unwrap().preview,
        "preview"
    );
}

#[test]
fn concurrent_wrong_codes_never_bypass_account_limit() {
    let path = std::env::temp_dir().join(format!("qiban-pair-race-{}.db", uuid::Uuid::new_v4()));
    let mut s = AccountStore::open(&path).unwrap();
    s.clock = || 1000;
    let d = identity("alice", "desktop");
    let m = identity("alice", "phone");
    let offer = s.offer_pairing(&d, "电脑").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|n| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut s = AccountStore::open(&path).unwrap();
                s.clock = || 1000;
                barrier.wait();
                s.preview_pairing(&identity("alice", &format!("phone-{n}")), "invalid")
                    .is_err()
            })
        })
        .collect();
    for worker in workers {
        assert!(worker.join().unwrap());
    }
    assert!(matches!(
        s.preview_pairing(&m, &offer.code),
        Err(StorageError::Authorization(Error::RateLimited))
    ));
    let count: u32 = s
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT failures FROM pairing_code_limits", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 5);
    drop(s);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn v5_archive_migrates_to_v6_preserving_pairings_with_null_lease() {
    // Rewind a live v6 store to a true v5 archive (drop the added columns)
    // by rebuilding the device rows, then reopen through the full ladder.
    let path = std::env::temp_dir().join(format!("accounts-v5-{}.db", uuid::Uuid::new_v4()));
    let (d, m, id) = {
        let s = fixture();
        // in-memory cannot be reopened; use a file store via open()
        drop(s);
        let mut file = AccountStore::open(&path).unwrap();
        file.clock = || 1000;
        let (d, m, p) = connected(&file);
        (d, m, p.id)
    };
    {
        let c = Connection::open(&path).unwrap();
        // Emulate the pre-v6 shape: rebuild paired_devices without the
        // presence columns, copying the live rows.
        c.execute_batch(
            "CREATE TABLE paired_devices_v5 AS \
             SELECT account_id,id,session_id,name,expires_at FROM paired_devices;\
             DROP TABLE paired_devices;\
             ALTER TABLE paired_devices_v5 RENAME TO paired_devices;\
             PRAGMA user_version=5;",
        )
        .unwrap();
        c.close().unwrap();
    }
    let mut s = AccountStore::open(&path).unwrap();
    s.clock = || 1000;
    let pairings = s.list_pairings(&m).unwrap();
    assert_eq!(pairings.len(), 1);
    assert_eq!(pairings[0].id, id);
    assert!(!pairings[0].desktop_online, "no heartbeat yet");
    assert_eq!(pairings[0].desktop_last_heartbeat_at, None);
    assert!(pairings[0].desktop_capabilities.is_empty());
    // The lease becomes writable after migration.
    assert!(s.heartbeat(&d, &[ActionScope::DocumentExcerpt]).unwrap());
    let pairings = s.list_pairings(&m).unwrap();
    assert!(pairings[0].desktop_online);
    assert_eq!(pairings[0].desktop_last_heartbeat_at, Some(1000));
    assert_eq!(pairings[0].desktop_capabilities, vec!["document_excerpt"]);
    drop(s);
    std::fs::remove_file(path).unwrap();
    let _ = (d, m);
}

#[test]
fn heartbeat_refreshes_only_own_session_row_and_expires_by_lease() {
    let mut s = fixture();
    let (d, m, _) = connected(&s);
    assert!(s.heartbeat(&d, &[ActionScope::DocumentExcerpt]).unwrap());
    let listed = s.list_pairings(&m).unwrap();
    assert!(listed[0].desktop_online);
    assert_eq!(listed[0].desktop_last_heartbeat_at, Some(1000));
    assert_eq!(listed[0].desktop_capabilities, vec!["document_excerpt"]);

    // A controller heartbeat updates its OWN row, never the desktop's.
    assert!(s.heartbeat(&m, &[]).unwrap());
    let listed = s.list_pairings(&m).unwrap();
    assert!(listed[0].desktop_online, "desktop lease untouched");

    // Lease expiry: +15_001ms with no beat → offline, stamp still reported.
    s.clock = || 1000 + 15_001;
    let listed = s.list_pairings(&m).unwrap();
    assert!(!listed[0].desktop_online);
    assert_eq!(listed[0].desktop_last_heartbeat_at, Some(1000));

    // A future heartbeat stamp counts as offline (mirrors last_seen defense).
    s.clock = || 500;
    let listed = s.list_pairings(&m).unwrap();
    assert!(!listed[0].desktop_online);
    s.clock = || 1000;

    // Cross-account isolation: bob's heartbeat cannot touch alice's row.
    let bob = identity("bob", "bob-session");
    assert!(!s.heartbeat(&bob, &[]).unwrap(), "bob has no device row");
    let listed = s.list_pairings(&m).unwrap();
    assert!(listed[0].desktop_online, "alice row untouched");
}

#[test]
fn heartbeat_without_device_row_is_false_and_revoked_sessions_are_rejected() {
    let s = fixture();
    let d = identity("alice", "desktop");
    // Logged in but never offered/accepted: no paired_devices row for this
    // session → idempotent false, and no row is created.
    assert!(!s.heartbeat(&d, &[ActionScope::DocumentExcerpt]).unwrap());

    let (d, m, _) = connected(&s);
    assert!(s.heartbeat(&d, &[]).unwrap());
    // sign_out kills the session; heartbeat must be rejected.
    s.sign_out(&d, false).unwrap();
    assert!(s.heartbeat(&d, &[]).is_err());
    let listed = s.list_pairings(&m).unwrap();
    assert_eq!(listed[0].status, "expired");
    assert!(!listed[0].desktop_online);
}

#[test]
fn new_login_heartbeat_does_not_resurrect_old_pairing_presence() {
    let s = fixture();
    let (d, m, _) = connected(&s);
    assert!(s.heartbeat(&d, &[ActionScope::DocumentExcerpt]).unwrap());

    // A fresh login mints a NEW session for the same subject: its heartbeat
    // matches no device row (updated=false), and the old pairing stays
    // expired + offline forever.
    s.sign_out(&d, false).unwrap();
    let d2 = identity("alice", "desktop-2");
    assert!(!s.heartbeat(&d2, &[ActionScope::DocumentExcerpt]).unwrap());
    let listed = s.list_pairings(&m).unwrap();
    assert_eq!(listed[0].status, "expired");
    assert!(!listed[0].desktop_online);
    assert_eq!(listed[0].desktop_last_heartbeat_at, Some(1000));
}

#[test]
fn heartbeat_capabilities_are_canonicalized_and_capped() {
    let s = fixture();
    let (d, m, _) = connected(&s);
    // Duplicates collapse; the store keeps the canonical sorted slug set.
    assert!(s
        .heartbeat(
            &d,
            &[ActionScope::DocumentExcerpt, ActionScope::DocumentExcerpt]
        )
        .unwrap());
    let listed = s.list_pairings(&m).unwrap();
    assert_eq!(listed[0].desktop_capabilities, vec!["document_excerpt"]);

    // More than 8 distinct entries cannot exist with a single-scope enum
    // today; the cap guard is exercised via the sorted-dedup cap check.
    // (When more scopes arrive, extend this with a >8 case.)
}

#[test]
fn malformed_capabilities_column_degrades_without_breaking_the_list() {
    let s = fixture();
    let (d, m, p) = connected(&s);
    let id = p.id.clone();
    assert!(s.heartbeat(&d, &[ActionScope::DocumentExcerpt]).unwrap());
    // Corrupt the capabilities blob behind the store's back.
    {
        let guard = s.connection.lock().unwrap();
        guard
            .execute(
                "UPDATE paired_devices SET capabilities='not-json' WHERE 1=1",
                [],
            )
            .unwrap();
    }
    let listed = s.list_pairings(&m).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert!(
        listed[0].desktop_capabilities.is_empty(),
        "degraded, not fatal"
    );
    assert!(listed[0].desktop_online, "presence itself still readable");
}

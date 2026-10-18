use super::*;
use companion_coordinator::{
    auth::{CodeMailer, MailResult, NativeAuth},
    router, AppState,
};
use companion_storage::accounts::AccountStore;
use companion_storage::remote_jobs::RemoteJobStore;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex as StdMutex,
    },
};

#[derive(Default)]
struct MemoryVault {
    entries: StdMutex<HashMap<String, String>>,
    fail_write: AtomicBool,
}
impl SessionVault for MemoryVault {
    fn read(&self, scope: &str) -> std::result::Result<Option<SessionSecret>, ()> {
        self.entries
            .lock()
            .unwrap()
            .get(scope)
            .map(|s| serde_json::from_str(s).map_err(|_| ()))
            .transpose()
    }
    fn write(&self, scope: &str, session: &SessionSecret) -> std::result::Result<(), ()> {
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(());
        }
        self.entries
            .lock()
            .unwrap()
            .insert(scope.into(), serde_json::to_string(session).unwrap());
        Ok(())
    }
    fn delete(&self, scope: &str) -> std::result::Result<(), ()> {
        self.entries.lock().unwrap().remove(scope);
        Ok(())
    }
}
#[derive(Default)]
struct Mail {
    codes: StdMutex<HashMap<String, String>>,
}
impl CodeMailer for Mail {
    fn send<'a>(&'a self, email: &'a str, code: &'a str) -> MailResult<'a> {
        Box::pin(async move {
            self.codes.lock().unwrap().insert(email.into(), code.into());
            Ok(())
        })
    }
}
struct Fixture {
    config: std::path::PathBuf,
    accounts: std::path::PathBuf,
    mail: Arc<Mail>,
    app: axum::Router,
    port: u16,
    server: Option<tokio::task::JoinHandle<()>>,
}
impl Fixture {
    async fn new() -> Self {
        let id = uuid::Uuid::new_v4();
        let config = std::env::temp_dir().join(format!("qiban-native-config-{id}.db"));
        let accounts = std::env::temp_dir().join(format!("qiban-native-accounts-{id}.db"));
        let store = Arc::new(AccountStore::open(&accounts).unwrap());
        let mail = Arc::new(Mail::default());
        let auth = NativeAuth::new(
            store.clone(),
            mail.clone(),
            [37; 32],
            &["alice@example.com".into(), "bob@example.com".into()],
        )
        .unwrap();
        let code_mail = mail.clone();
        // Only this cfg(test) fixture exposes its in-memory test mailbox.
        let app = router(AppState::new(auth, store)).route("/__test/code", axum::routing::get(move || {
            let mail = code_mail.clone();
            async move { axum::Json(json!({"code": mail.codes.lock().unwrap().get("alice@example.com").cloned()})) }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let serving = app.clone();
        let server = Some(tokio::spawn(async move {
            axum::serve(listener, serving).await.unwrap();
        }));
        Self {
            config,
            accounts,
            mail,
            app,
            port,
            server,
        }
    }
    fn client(&self, vault: Arc<dyn SessionVault>) -> AccountClient {
        let mut client = AccountClient::open(&self.config, vault).unwrap();
        // On first open only. Reopen must not overwrite settings or credentials.
        if client.port != self.port {
            client.set_port(self.port).unwrap();
        }
        client
    }
    fn code(&self, email: &str) -> String {
        self.mail.codes.lock().unwrap().get(email).unwrap().clone()
    }
    async fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
            let _ = server.await;
        }
    }
    async fn restart(&mut self) {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, self.port))
            .await
            .unwrap();
        let app = self.app.clone();
        self.server = Some(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            server.abort();
        }
        let _ = std::fs::remove_file(&self.config);
        // Router connections may still be dropping; temp DBs contain test data only.
        let _ = std::fs::remove_file(&self.accounts);
    }
}

#[tokio::test]
async fn credential_failure_retry_reopen_dedup_and_scope_are_safe() {
    let f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let mut c = f.client(vault.clone());
    c.request_code("alice@example.com".into()).await.unwrap();
    vault.fail_write.store(true, Ordering::SeqCst);
    assert_eq!(
        c.login(f.code("alice@example.com"))
            .await
            .err()
            .unwrap()
            .code,
        "credentials"
    );
    vault.fail_write.store(false, Ordering::SeqCst);
    let first = c.login(f.code("alice@example.com")).await.unwrap();
    let profile = first.profile.unwrap();
    let request_id = uuid::Uuid::new_v4().to_string();
    let task = c
        .create_task(request_id.clone(), "共享但不执行".into())
        .await
        .unwrap();
    assert_eq!(
        task.id,
        c.create_task(request_id, "共享但不执行".into())
            .await
            .unwrap()
            .id
    );
    assert_eq!(c.set_port(4321).err().unwrap().code, "already_signed_in");
    assert_eq!(
        c.request_code("bob@example.com".into())
            .await
            .err()
            .unwrap()
            .code,
        "already_signed_in"
    );
    let secret = vault.read(&c.scope()).unwrap().unwrap();
    assert!(!std::fs::read(&f.config)
        .unwrap()
        .windows(secret.token.len())
        .any(|b| b == secret.token.as_bytes()));
    assert!(vault.read(&(c.scope() + "-other")).unwrap().is_none());
    drop(c);
    let mut restored = f.client(vault.clone());
    let snapshot = restored.snapshot().await.unwrap();
    assert_eq!(snapshot.profile.unwrap().companion_id, profile.companion_id);
    assert_eq!(snapshot.tasks.len(), 1);
    restored.cancel_task(task.id, task.revision).await.unwrap();
    assert!(matches!(
        restored.snapshot().await.unwrap().tasks[0].status,
        companion_core::TaskStatus::Cancelled
    ));
    let scope = restored.scope();
    vault.fail_write.store(true, Ordering::SeqCst);
    assert_eq!(
        restored.logout(false).await.err().unwrap().code,
        "logout_not_saved"
    );
    assert_eq!(restored.snapshot().await.unwrap().status, "authenticated");
    vault.fail_write.store(false, Ordering::SeqCst);
    assert_eq!(restored.logout(false).await.unwrap().status, "signed_out");
    assert!(vault.read(&scope).unwrap().is_none());
    drop(restored);
    assert_eq!(
        f.client(vault).snapshot().await.unwrap().status,
        "signed_out"
    );
}

#[tokio::test]
async fn offline_logout_survives_restart_and_cannot_restore_or_send_data() {
    let mut f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let mut c = f.client(vault.clone());
    c.request_code("alice@example.com".into()).await.unwrap();
    c.login(f.code("alice@example.com")).await.unwrap();
    f.stop().await;
    let snapshot = c.logout(true).await.unwrap();
    assert_eq!(snapshot.status, "logout_pending");
    assert!(snapshot.profile.is_none() && snapshot.tasks.is_empty());
    drop(c);
    let mut restored = f.client(vault.clone());
    assert_eq!(restored.snapshot().await.unwrap().status, "logout_pending");
    assert_eq!(
        restored
            .create_task(uuid::Uuid::new_v4().to_string(), "不得发送".into())
            .await
            .err()
            .unwrap()
            .code,
        "logout_pending"
    );
    assert_eq!(
        restored.set_port(4321).err().unwrap().code,
        "logout_pending"
    );
    f.restart().await;
    assert_eq!(restored.snapshot().await.unwrap().status, "signed_out");
    assert!(vault.read(&restored.scope()).unwrap().is_none());
}

#[tokio::test]
async fn remote_revocation_clears_local_session_and_other_account_is_separate() {
    let f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let mut c = f.client(vault.clone());
    c.request_code("alice@example.com".into()).await.unwrap();
    let alice = c
        .login(f.code("alice@example.com"))
        .await
        .unwrap()
        .profile
        .unwrap();
    c.create_task(uuid::Uuid::new_v4().to_string(), "Alice 私有待办".into())
        .await
        .unwrap();
    let secret = vault.read(&c.scope()).unwrap().unwrap();
    c.request(
        Method::POST,
        "/v1/logout",
        Some(&secret.token),
        Some(json!({"allSessions":true})),
    )
    .await
    .unwrap();
    assert_eq!(
        c.snapshot().await.err().unwrap().code,
        "authentication_required"
    );
    assert!(vault.read(&c.scope()).unwrap().is_none());
    c.request_code("bob@example.com".into()).await.unwrap();
    let bob = c.login(f.code("bob@example.com")).await.unwrap();
    assert_ne!(bob.profile.unwrap().account_id, alice.account_id);
    assert!(bob.tasks.is_empty());
    let secret = vault.read(&c.scope()).unwrap().unwrap();
    c.request(
        Method::POST,
        "/v1/logout",
        Some(&secret.token),
        Some(json!({"allSessions":false})),
    )
    .await
    .unwrap();
    let result = c.logout(true).await.unwrap();
    assert_eq!(result.status, "signed_out");
    assert!(
        result.notice.is_some(),
        "an invalid token cannot confirm revocation of other sessions"
    );
}

#[tokio::test]
async fn metadata_validates_schema_port_and_rejects_redirects() {
    let f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let mut c = f.client(vault);
    assert_eq!(c.set_port(80).err().unwrap().code, "invalid_request");
    c.db.lock()
        .unwrap()
        .pragma_update(None, "user_version", 99)
        .unwrap();
    drop(c);
    assert!(AccountClient::open(&f.config, Arc::new(MemoryVault::default())).is_err());
    let redirected = Arc::new(AtomicBool::new(false));
    let sink = redirected.clone();
    let app = axum::Router::new()
        .route(
            "/v1/auth/request-code",
            axum::routing::post(|| async {
                axum::response::Redirect::temporary("/should-not-be-followed")
            }),
        )
        .route(
            "/should-not-be-followed",
            axum::routing::any(move || {
                let sink = sink.clone();
                async move {
                    sink.store(true, Ordering::SeqCst);
                    axum::Json(json!({"challengeId": uuid::Uuid::new_v4()}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let fresh = Fixture::new().await;
    let mut client = fresh.client(Arc::new(MemoryVault::default()));
    client.set_port(port).unwrap();
    assert_eq!(
        client
            .request_code("alice@example.com".into())
            .await
            .err()
            .unwrap()
            .code,
        "unavailable"
    );
    assert!(!redirected.load(Ordering::SeqCst));
    server.abort();
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "Explicit Windows Credential Manager + real coordinator integration; uses test-only accounts/mail"]
async fn windows_vault_reopen_and_revoke_integration() {
    let mut f = Fixture::new().await;
    let vault: Arc<dyn SessionVault> = Arc::new(SystemVault);
    let mut c = f.client(vault.clone());
    let scope = c.scope();
    // Always clean up only this unique test credential, including on assertion failure.
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = SystemVault.delete(&self.0);
        }
    }
    let _cleanup = Cleanup(scope.clone());
    c.request_code("alice@example.com".into()).await.unwrap();
    let identity = c
        .login(f.code("alice@example.com"))
        .await
        .unwrap()
        .profile
        .unwrap();
    c.create_task(
        uuid::Uuid::new_v4().to_string(),
        "Windows 凭据重开验收".into(),
    )
    .await
    .unwrap();
    drop(c);
    let mut reopened = f.client(vault.clone());
    assert_eq!(
        reopened
            .snapshot()
            .await
            .unwrap()
            .profile
            .unwrap()
            .companion_id,
        identity.companion_id
    );
    f.stop().await;
    assert_eq!(
        reopened.logout(false).await.unwrap().status,
        "logout_pending"
    );
    drop(reopened);
    let mut reopened = f.client(vault.clone());
    assert_eq!(reopened.snapshot().await.unwrap().status, "logout_pending");
    f.restart().await;
    assert_eq!(reopened.snapshot().await.unwrap().status, "signed_out");
    assert!(vault.read(&scope).unwrap().is_none());
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Real native account + mobile Web + Windows vault; npm run test:desktop-account"]
async fn desktop_mobile_same_companion_integration() {
    let f = Fixture::new().await;
    let mut c = f.client(Arc::new(SystemVault));
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = SystemVault.delete(&self.0);
        }
    }
    let _cleanup = Cleanup(c.scope());
    c.request_code("alice@example.com".into()).await.unwrap();
    let snapshot = c.login(f.code("alice@example.com")).await.unwrap();
    let profile = snapshot.profile.unwrap();
    let native_task = c
        .create_task(
            uuid::Uuid::new_v4().to_string(),
            "来自原生桌面的共享待办".into(),
        )
        .await
        .unwrap();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let port = f.port;
    let offer = c.pairing_offer("集成测试电脑".into()).await.unwrap();
    let pairing_id = offer.pairing.id.clone();
    let remote_root =
        std::env::temp_dir().join(format!("qiban-remote-test-{}", uuid::Uuid::new_v4()));
    let worker_root = remote_root.clone();
    let worker_client = f.client(Arc::new(SystemVault));
    let worker = tokio::spawn(async move {
        let remote = crate::remote_documents::RemoteDocuments::new(worker_root.clone());
        let mut shared = false;
        loop {
            if !shared {
                if let Some(pair) = worker_client
                    .pairings()
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|p| p.status == "active")
                {
                    for name in [
                        "保存测试.txt",
                        "取消测试.txt",
                        "准入丢失.txt",
                        "回执恢复.txt",
                    ] {
                        let prepared = remote
                            .prepare(
                                &worker_client,
                                pair.id.clone(),
                                uuid::Uuid::new_v4().to_string(),
                                name.into(),
                                "测试摘录，不读取用户文件".into(),
                            )
                            .await
                            .unwrap();
                        remote
                            .share(&worker_client, &prepared.task.id)
                            .await
                            .unwrap();
                    }
                    shared = true;
                }
            }
            if shared {
                for d in remote.list(&worker_client).await.unwrap() {
                    if d.authorization.state == "confirmed"
                        && ["准入丢失.txt", "回执恢复.txt"].contains(&d.source_name.as_str())
                    {
                        // Fault injection at protocol boundaries: discard admission reply,
                        // or persist a real artifact then reopen without sending a receipt.
                        worker_client
                            .request(
                                Method::POST,
                                &format!(
                                    "/v1/documents/{}/admit",
                                    d.authorization.binding.action_id
                                ),
                                Some(&worker_client.active_secret().unwrap().token),
                                Some(serde_json::to_value(&d.authorization.binding).unwrap()),
                            )
                            .await
                            .unwrap();
                        if d.source_name == "回执恢复.txt" {
                            let owner = worker_client.remote_owner().await.unwrap();
                            let ledger =
                                crate::execution::ExecutionState::open(&worker_root.join(owner))
                                    .unwrap();
                            ledger
                                .execute(&d.authorization.binding.resource_id, 0)
                                .unwrap();
                        }
                    }
                }
                let _ = remote.sync(&worker_client).await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    });
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new("node")
            .arg("apps/desktop/tests/browser/mobile-peer.mjs")
            .current_dir(workspace)
            .env("QIBAN_TEST_UPSTREAM", format!("http://127.0.0.1:{port}"))
            .env("QIBAN_TEST_PAIRING_CODE", offer.code)
            .env("QIBAN_EXPECT_DESKTOP", offer.pairing.desktop_id)
            .env("QIBAN_EXPECT_ACCOUNT", profile.account_id)
            .env("QIBAN_EXPECT_COMPANION", profile.companion_id)
            .status()
            .unwrap()
    })
    .await
    .unwrap();
    worker.abort();
    let _ = worker.await;
    assert!(status.success(), "mobile peer verification failed");
    let owner = c.remote_owner().await.unwrap();
    let ledger = crate::execution::ExecutionState::open(&remote_root.join(owner)).unwrap();
    let tasks = ledger.store.list().unwrap();
    assert_eq!(tasks.len(), 4);
    for task in &tasks {
        let expected = if ["保存测试.txt", "回执恢复.txt"].contains(&task.source_name.as_str())
        {
            companion_core::execution::ExecutionStatus::Completed
        } else {
            companion_core::execution::ExecutionStatus::Cancelled
        };
        assert_eq!(task.status, expected);
        let attempts = ledger.store.detail(&task.id).unwrap().attempts.len();
        assert_eq!(
            attempts,
            if expected == companion_core::execution::ExecutionStatus::Completed {
                1
            } else {
                0
            }
        );
    }
    drop(ledger);
    // --- Deletion sweep: a server-deleted record converges locally, and a
    // reported action can never be silently re-shared. The controller needs a
    // second alice login, so wait out the real 60s OTP mail cooldown once ---
    tokio::time::sleep(std::time::Duration::from_secs(61)).await;
    let mut phone = f.client(Arc::new(MemoryVault::default()));
    phone
        .request_code("alice@example.com".into())
        .await
        .unwrap();
    phone.login(f.code("alice@example.com")).await.unwrap();
    let offer2 = c.pairing_offer("清理测试电脑".into()).await.unwrap();
    let phone_secret = phone.active_secret().unwrap().token.clone();
    phone
        .request(
            Method::POST,
            "/v1/pairings/accept",
            Some(&phone_secret),
            Some(json!({
                "code": offer2.code,
                "pairingId": offer2.pairing.id,
                "name": "清理手机"
            })),
        )
        .await
        .unwrap();
    let remote2 = crate::remote_documents::RemoteDocuments::new(remote_root.clone());
    let prepared = remote2
        .prepare(
            &c,
            offer2.pairing.id.clone(),
            uuid::Uuid::new_v4().to_string(),
            "清理.txt".into(),
            "清理路径回归的摘录".into(),
        )
        .await
        .unwrap();
    let cleanup_id = prepared.task.id.clone();
    let action_id = prepared.task.action_id.clone();
    let artifact_hash = prepared.task.artifact_hash.clone();
    let binding = serde_json::to_value(&prepared.share.binding).unwrap();
    remote2.share(&c, &cleanup_id).await.unwrap();
    let desktop_secret = c.active_secret().unwrap().token.clone();
    for (path, body) in [
        (
            format!("/v1/documents/{action_id}/confirm"),
            binding.clone(),
        ),
        (format!("/v1/documents/{action_id}/admit"), binding.clone()),
        (
            format!("/v1/documents/{action_id}/receipt"),
            json!({"binding":binding,"state":"failed","artifactHash":artifact_hash}),
        ),
        (format!("/v1/documents/{action_id}/delete"), json!({})),
    ] {
        // Confirm is controller-only; the rest use the desktop session.
        let token = if path.ends_with("/confirm") {
            &phone_secret
        } else {
            &desktop_secret
        };
        c.request(Method::POST, &path, Some(token), Some(body))
            .await
            .unwrap();
    }
    let owner2 = c.remote_owner().await.unwrap();
    let jobs = RemoteJobStore::open(&remote_root.join(&owner2).join("remote-jobs.db")).unwrap();
    // The share POST reset the phase to sharing; admitting went through raw
    // HTTP, so set the proof phase exactly as a crashed sync would leave it.
    jobs.phase(&cleanup_id, "admitted").unwrap();
    remote2.sync(&c).await.unwrap();
    assert_eq!(jobs.load(&cleanup_id).unwrap().1, "reported");
    let ledger2 = crate::execution::ExecutionState::open(&remote_root.join(&owner2)).unwrap();
    assert_eq!(
        ledger2.store.detail(&cleanup_id).unwrap().task.status,
        companion_core::execution::ExecutionStatus::Cancelled
    );
    assert_eq!(ledger2.store.list().unwrap().len(), 5);
    assert!(!remote_root
        .join(&owner2)
        .join("document-drafts")
        .join(format!("{action_id}.md"))
        .exists());
    // A reported action must refuse to re-share instead of expiring silently.
    assert!(remote2.share(&c, &cleanup_id).await.is_err());
    drop(ledger2);
    drop(jobs);
    assert!(
        remote_root.starts_with(std::env::temp_dir())
            && remote_root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("qiban-remote-test-")
    );
    std::fs::remove_dir_all(&remote_root).unwrap();
    drop(c);
    let mut reopened = f.client(Arc::new(SystemVault));
    let snapshot = reopened.snapshot().await.unwrap();
    assert!(snapshot
        .tasks
        .iter()
        .any(|t| t.title == "来自手机的共享待办"));
    assert!(snapshot
        .tasks
        .iter()
        .any(|t| t.id == native_task.id && t.status == companion_core::TaskStatus::Cancelled));
    let pairs = reopened.pairings().await.unwrap();
    assert!(pairs
        .iter()
        .any(|p| p.id == pairing_id && p.status == "revoked"));
    assert_eq!(reopened.logout(true).await.unwrap().status, "signed_out");
}

#[tokio::test]
async fn heartbeat_is_idempotent_presence_and_pairings_report_it() {
    let f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let mut c = f.client(vault.clone());
    c.request_code("alice@example.com".into()).await.unwrap();
    c.login(f.code("alice@example.com")).await.unwrap();

    // Logged in but no pairing yet: no device row → updated=false, no error.
    assert!(!c
        .heartbeat(&["document_excerpt".to_string()])
        .await
        .unwrap());

    // After an offer the desktop session owns a device row → updated=true.
    let offer = c.pairing_offer("心跳测试电脑".into()).await.unwrap();
    assert_eq!(offer.pairing.current_role, "desktop");
    assert!(!offer.pairing.desktop_online, "no beat yet");
    assert!(c
        .heartbeat(&["document_excerpt".to_string()])
        .await
        .unwrap());

    // The pairing payload carries the presence family.
    let listed = c.pairings().await.unwrap();
    let mine = listed.iter().find(|p| p.id == offer.pairing.id).unwrap();
    assert!(mine.desktop_online);
    assert!(mine.desktop_last_heartbeat_at.is_some());
    assert_eq!(mine.desktop_capabilities, vec!["document_excerpt"]);
}

#[tokio::test]
async fn heartbeat_requires_an_active_session() {
    let f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let c = f.client(vault);
    // Never logged in: no secret → Err, and no network request is made.
    assert!(c.heartbeat(&[]).await.is_err());
}

// ---------------------------------------------------------------------------
// Cross-device memory sync: real HTTP against the coordinator router, a real
// AccountClient, and a real HistoryStore. The phone peer is driven through
// the same routes via raw requests (a second login would sit in the 61s code
// throttle), and "another desktop" is a second HistoryStore resolving through
// the same session — the server sees one device, which only shares the pull
// cursor, not the local stores' convergence.
use companion_core::memory::{MemoryDraft, MemoryKind};
use companion_storage::history::HistoryStore;
use companion_storage::memory_sync_outbox::{OutboxOp, RemoteApplyReport};

fn memory_draft(body: &str) -> MemoryDraft {
    MemoryDraft {
        kind: MemoryKind::Preference,
        body: body.into(),
        event_date: None,
    }
}

/// One iteration of the sync loop's core sequence (pull -> apply -> push ->
/// ack/drop), against real components. Returns what the pull applied. The
/// pull loop re-reads the persisted cursor every round exactly like the real
/// loop — reusing a stale cursor would re-serve the same page forever.
async fn sync_tick(c: &AccountClient, store: &mut HistoryStore) -> RemoteApplyReport {
    let owner = c.remote_owner().await.unwrap();
    let mut total = RemoteApplyReport::default();
    loop {
        let (generation, cursor) = store.memory_sync_state(&owner).unwrap();
        let response = c.memory_pull(generation, cursor).await.unwrap();
        let pending = response.pending;
        let report = if let Some(items) = &response.snapshot {
            store
                .memory_apply_snapshot(&owner, items, response.generation, response.cursor)
                .unwrap()
        } else {
            store
                .memory_apply_remote(
                    &owner,
                    &response.events,
                    response.generation,
                    response.cursor,
                )
                .unwrap()
        };
        total.changed |= report.changed;
        total.cleared_turns += report.cleared_turns;
        total.skipped_kind += report.skipped_kind;
        if !pending {
            break;
        }
    }
    for entry in store.memory_outbox_peek().unwrap() {
        let base = store.memory_push_base(&owner, &entry.memory_id).unwrap();
        let (op, base) = match entry.op {
            OutboxOp::Create => ("create", 0i64),
            OutboxOp::Update => match base {
                Some(base) => ("update", base),
                None => ("create", 0),
            },
            OutboxOp::Delete => match base {
                Some(base) => ("delete", base),
                None => {
                    store
                        .memory_outbox_ack(&owner, &entry.request_id, &entry.memory_id, 0)
                        .unwrap();
                    continue;
                }
            },
        };
        match c.memory_push(&entry, op, base).await {
            Ok(receipt) => store
                .memory_outbox_ack(
                    &owner,
                    &entry.request_id,
                    &entry.memory_id,
                    receipt.revision,
                )
                .unwrap(),
            Err(error) => {
                assert!(
                    matches!(
                        error.code,
                        "memory_revision_conflict" | "memory_resurrection" | "memory_capacity"
                    ),
                    "unexpected push error: {}",
                    error.code
                );
                store.memory_outbox_drop(&owner, &entry.request_id).unwrap();
            }
        }
    }
    total
}

async fn phone_write(
    c: &AccountClient,
    path: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let secret = c.active_secret().unwrap();
    let url = format!("http://127.0.0.1:{}{path}", c_port(c));
    let response = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&secret.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap())
}

fn c_port(c: &AccountClient) -> u16 {
    // Tests live inside the account module, so the private field is visible.
    c.port
}

#[tokio::test]
async fn memory_sync_end_to_end_create_push_peer_delete_and_resurrect() {
    let f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let mut c = f.client(vault.clone());
    c.request_code("alice@example.com".into()).await.unwrap();
    c.login(f.code("alice@example.com")).await.unwrap();

    let mut desktop = HistoryStore::open(
        &std::env::temp_dir().join(format!("qiban-desktop-sync-{}.db", uuid::Uuid::new_v4())),
    )
    .unwrap();
    // A local create lands in the outbox and reaches the server on the next
    // tick; the peer list then sees it.
    let epoch = desktop.context_epoch().unwrap();
    let memory = desktop
        .memory_create(&memory_draft("prefer tea"), epoch)
        .unwrap()
        .value;
    assert_eq!(desktop.memory_outbox_peek().unwrap().len(), 1);
    sync_tick(&c, &mut desktop).await;
    assert!(desktop.memory_outbox_peek().unwrap().is_empty());

    // The phone corrects the entry through the peer route; the desktop's
    // next tick converges its local wording through the domain update path.
    let (status, _) = phone_write(
        &c,
        &format!("/v1/memories/{}/update", memory.id),
        serde_json::json!({
            "revision": 1,
            "requestId": uuid::Uuid::new_v4().to_string(),
            "kind": "preference",
            "body": "prefer coffee"
        }),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let report = sync_tick(&c, &mut desktop).await;
    assert!(report.changed);
    let view = desktop.memory_list().unwrap();
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].body, "prefer coffee");

    // The phone deletes; the desktop tombstones locally and its chats that
    // quoted the entry are pruned precisely (none here — no turns yet).
    let (status, _) = phone_write(
        &c,
        &format!("/v1/memories/{}/delete", memory.id),
        serde_json::json!({
            "revision": 2,
            "requestId": uuid::Uuid::new_v4().to_string()
        }),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let report = sync_tick(&c, &mut desktop).await;
    assert!(report.changed);
    assert!(desktop.memory_list().unwrap().is_empty());

    // A second desktop (a second HistoryStore) converges from the snapshot
    // and sees the same tombstone — it can no longer resurrect the id either.
    let mut laptop = HistoryStore::open(
        &std::env::temp_dir().join(format!("qiban-desktop-sync-{}.db", uuid::Uuid::new_v4())),
    )
    .unwrap();
    sync_tick(&c, &mut laptop).await;
    assert!(laptop.memory_list().unwrap().is_empty());
    let epoch = laptop.context_epoch().unwrap();
    let replaced = laptop
        .memory_create(&memory_draft("fresh entry"), epoch)
        .unwrap()
        .value;
    sync_tick(&c, &mut laptop).await;
    // The second store converges to the tombstone-only world: it cannot
    // resurrect the deleted id either (the server's resurrection rejection is
    // asserted at the coordinator layer), and its own fresh entry simply
    // round-trips.
    assert!(laptop
        .memory_list()
        .unwrap()
        .iter()
        .all(|m| m.id != memory.id));
    assert!(laptop
        .memory_list()
        .unwrap()
        .iter()
        .any(|m| m.id == replaced.id));
    assert!(laptop.memory_outbox_peek().unwrap().is_empty());
}

#[tokio::test]
async fn memory_sync_conflict_drops_local_edit_and_converges_to_server() {
    let f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let mut c = f.client(vault.clone());
    c.request_code("alice@example.com".into()).await.unwrap();
    c.login(f.code("alice@example.com")).await.unwrap();

    let mut desktop = HistoryStore::open(
        &std::env::temp_dir().join(format!("qiban-desktop-sync-{}.db", uuid::Uuid::new_v4())),
    )
    .unwrap();
    let epoch = desktop.context_epoch().unwrap();
    let memory = desktop
        .memory_create(&memory_draft("prefer tea"), epoch)
        .unwrap()
        .value;
    sync_tick(&c, &mut desktop).await;

    // The phone wins revision 2 while the desktop's edit sits unpushed with
    // base 1. A normal pull-first tick would rebase that pending row onto 2
    // and let the later local write win (last-writer-wins per revision); the
    // 409/drop path covers the race window where the push leaves BEFORE the
    // pull applies the phone's event — pushed here directly.
    let (status, _) = phone_write(
        &c,
        &format!("/v1/memories/{}/update", memory.id),
        serde_json::json!({
            "revision": 1,
            "requestId": uuid::Uuid::new_v4().to_string(),
            "kind": "preference",
            "body": "phone wins"
        }),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let epoch = desktop.context_epoch().unwrap();
    desktop
        .memory_update(
            &memory.id,
            memory.revision,
            epoch,
            &memory_draft("desktop edit"),
        )
        .unwrap();
    let owner = c.remote_owner().await.unwrap();
    let entry = desktop.memory_outbox_peek().unwrap().remove(0);
    let Err(error) = c.memory_push(&entry, "update", 1).await else {
        panic!("stale base must be rejected");
    };
    assert_eq!(error.code, "memory_revision_conflict");
    // Drop clears the row, the entry's mirror, and the pull position; the
    // next tick's snapshot diff is what lands the phone's wording.
    desktop
        .memory_outbox_drop(&owner, &entry.request_id)
        .unwrap()
        .then_some(())
        .expect("row dropped");
    assert_eq!(
        desktop.memory_push_base(&owner, &memory.id).unwrap(),
        None,
        "mirror cleared for the dropped entry"
    );
    sync_tick(&c, &mut desktop).await;
    let converged = desktop.memory_list().unwrap();
    assert_eq!(converged[0].body, "phone wins");
    assert!(desktop.memory_outbox_peek().unwrap().is_empty());
}

#[tokio::test]
async fn memory_sync_offline_delete_replays_and_is_idempotent_on_the_wire() {
    let mut f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let mut c = f.client(vault.clone());
    c.request_code("alice@example.com".into()).await.unwrap();
    c.login(f.code("alice@example.com")).await.unwrap();

    let mut desktop = HistoryStore::open(
        &std::env::temp_dir().join(format!("qiban-desktop-sync-{}.db", uuid::Uuid::new_v4())),
    )
    .unwrap();
    let epoch = desktop.context_epoch().unwrap();
    let memory = desktop
        .memory_create(&memory_draft("offline delete me"), epoch)
        .unwrap()
        .value;
    sync_tick(&c, &mut desktop).await;

    // The coordinator goes away: the local delete still commits (outbox
    // row), and the restart replays it exactly once semantically.
    f.stop().await;
    let epoch = desktop.context_epoch().unwrap();
    desktop
        .memory_delete(&memory.id, memory.revision, epoch)
        .unwrap();
    assert_eq!(desktop.memory_list().unwrap().len(), 0);
    f.restart().await;
    sync_tick(&c, &mut desktop).await;
    assert!(desktop.memory_outbox_peek().unwrap().is_empty());

    // The phone list no longer carries the entry; a second push of the same
    // request replays the stored receipt (lost-response recovery).
    let (_, listed) = list_remote(&c).await;
    assert!(listed.as_array().unwrap().is_empty());
    let owner = c.remote_owner().await.unwrap();
    let receipt = desktop.memory_push_base(&owner, &memory.id).unwrap();
    assert_eq!(
        receipt,
        Some(2),
        "mirror advanced to the tombstone revision"
    );
}

async fn list_remote(c: &AccountClient) -> (reqwest::StatusCode, serde_json::Value) {
    let secret = c.active_secret().unwrap();
    let url = format!("http://127.0.0.1:{}/v1/memories", c_port(c));
    let response = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&secret.token)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap())
}

#[tokio::test]
async fn executed_task_settles_a_fact_and_syncs_to_the_peer_list() {
    let f = Fixture::new().await;
    let vault = Arc::new(MemoryVault::default());
    let mut c = f.client(vault.clone());
    c.request_code("alice@example.com".into()).await.unwrap();
    c.login(f.code("alice@example.com")).await.unwrap();

    // A real execution store walked to Completed through the real
    // transitions, plus a real history store — the settle bridge's core
    // sequence (ledger -> projection -> deposit -> settled) against live
    // components, then the ordinary sync tick carries the fact to the server
    // and the phone list sees it.
    let exec = companion_storage::execution::ExecutionStore::open(
        &std::env::temp_dir().join(format!("qiban-tf-exec-{}.db", uuid::Uuid::new_v4())),
    )
    .unwrap();
    let mut desktop = HistoryStore::open(
        &std::env::temp_dir().join(format!("qiban-tf-hist-{}.db", uuid::Uuid::new_v4())),
    )
    .unwrap();
    let request_id = uuid::Uuid::new_v4().to_string();
    let task = exec
        .prepare(&request_id, "路线图.txt", "第一行\n第二行")
        .unwrap();
    let attempt = exec
        .claim(&task.id, task.revision, "acceptance")
        .unwrap()
        .unwrap();
    let done = exec
        .resolve(
            &task.id,
            &attempt.id,
            companion_core::execution::ExecutionStatus::Completed,
            "done",
        )
        .unwrap();
    // The settle bridge: drain, project, deposit, clear (idempotent retry).
    for (fingerprint, task_id, pending) in exec.fact_pending().unwrap() {
        let (body, date) = companion_core::execution::task_fact_projection(&pending).unwrap();
        desktop
            .task_fact_deposit(&fingerprint, &body, date.as_deref())
            .unwrap();
        exec.fact_settled(&task_id).unwrap();
    }
    assert!(exec.fact_pending().unwrap().is_empty());
    let facts: Vec<_> = desktop
        .memory_list()
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == companion_core::memory::MemoryKind::TaskFact)
        .collect();
    assert_eq!(facts.len(), 1);
    assert_eq!(
        facts[0].body,
        format!("把《{}》的摘录保存为草稿：已核验完成", "路线图.txt")
    );
    let _ = done;
    // Sync to the server; the peer list shows the fact; the phone cannot
    // correct it (400) but can delete it — the server route contract.
    sync_tick(&c, &mut desktop).await;
    let (_, listed) = list_remote(&c).await;
    let items = listed.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "task_fact");
    assert_eq!(items[0]["body"], facts[0].body);
    let fact_id = items[0]["id"].as_str().unwrap().to_string();
    let (status, _) = phone_write(
        &c,
        &format!("/v1/memories/{fact_id}/update"),
        serde_json::json!({
            "revision": 1,
            "requestId": uuid::Uuid::new_v4().to_string(),
            "kind": "task_fact",
            "body": "篡改"
        }),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    let (status, _) = phone_write(
        &c,
        &format!("/v1/memories/{fact_id}/delete"),
        serde_json::json!({"revision": 1, "requestId": uuid::Uuid::new_v4().to_string()}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // The desktop converges to the tombstone; the fact row is gone but the
    // memories table keeps its content-free tombstone (never resurrectable).
    sync_tick(&c, &mut desktop).await;
    assert!(desktop
        .memory_list()
        .unwrap()
        .iter()
        .all(|m| m.kind != companion_core::memory::MemoryKind::TaskFact));
}

#[tokio::test]
async fn embedded_mint_self_heals_after_sign_out_everywhere() {
    // Embedded composition (P1-b PR-1): start the real embedded server on a
    // process-unique port, mint the local account through it, then kill the
    // session server-side ("sign out everywhere" from a phone) and prove the
    // desktop re-mints itself into a working session.
    let dir = std::env::temp_dir().join(format!("embedded-heal-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let preferred = 46_000 + (std::process::id() % 2_000) as u16;
    let started = crate::embedded::start(&dir.join("coordinator.db"), preferred, None)
        .await
        .unwrap();
    let crate::embedded::EmbeddedCoordinator::Started {
        port,
        store,
        shutdown,
    } = started
    else {
        panic!("an external service owns the test port; rerun to re-randomize");
    };
    let vault = Arc::new(MemoryVault::default());
    let mut client = AccountClient::open(&dir.join("account-settings.db"), vault.clone()).unwrap();
    client.set_port(port).unwrap();
    // First boot: mint opens the local account and a working session.
    assert!(client.ensure_local_session(&store).await.unwrap());
    assert_eq!(client.snapshot().await.unwrap().status, "authenticated");
    // Server-side "sign out everywhere" (as a phone would trigger): the
    // desktop session dies and the vault secret is now worthless.
    let secret = vault.read(&client.scope()).unwrap().unwrap();
    let hash: [u8; 32] = {
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&secret.token[4..])
            .unwrap();
        <sha2::Sha256 as sha2::Digest>::digest(&raw).into()
    };
    let identity = store.authenticate_native_token(&hash).unwrap();
    store.sign_out(&identity, true).unwrap();
    // The next snapshot hits 401: the request layer clears the vault (same
    // as any remote revocation) and reports authentication_required.
    let revoked = client.snapshot().await;
    assert!(matches!(revoked, Err(e) if e.code == "authentication_required"));
    // Self-healing: the embedded host re-mints a NEW session (login time
    // postdates the revocation cutoff) without any user-facing login.
    assert!(client.ensure_local_session(&store).await.unwrap());
    assert_eq!(client.snapshot().await.unwrap().status, "authenticated");
    shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    // Let the serve task observe the flag and drop the SQLite handle; the
    // cleanup is best-effort (a flaked delete never fails the test).
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let _ = std::fs::remove_dir_all(&dir);
}

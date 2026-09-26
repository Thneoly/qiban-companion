use super::*;
use companion_coordinator::{
    auth::{CodeMailer, MailResult, NativeAuth},
    router, AppState,
};
use companion_storage::accounts::AccountStore;
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
    assert!(status.success(), "mobile peer verification failed");
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

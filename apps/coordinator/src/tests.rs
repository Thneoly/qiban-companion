use super::*;
use auth::{CodeMailer, MailResult};
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex,
};
use tower::ServiceExt;

fn nonce() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&COUNTER.fetch_add(1, Ordering::SeqCst).to_le_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Default)]
struct TestMail {
    sent: Mutex<Vec<(String, String)>>,
    fail: AtomicBool,
}
impl CodeMailer for TestMail {
    fn send<'a>(&'a self, email: &'a str, code: &'a str) -> MailResult<'a> {
        Box::pin(async move {
            self.sent.lock().unwrap().push((email.into(), code.into()));
            if self.fail.load(Ordering::SeqCst) {
                Err(())
            } else {
                Ok(())
            }
        })
    }
}
struct Fixture {
    path: std::path::PathBuf,
    mail: Arc<TestMail>,
}

// Explicit opt-in only: a real HTTP/SQLite/auth chain with an in-memory mail sink.
// This route and mail access are compiled solely into the Rust test executable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires built mobile Web and local Microsoft Edge; npm run test:mobile:integration"]
async fn mobile_web_browser_integration() {
    let fixture = Fixture::new();
    let mail = fixture.mail.clone();
    let app = fixture.app().route(
        "/__test/code/{email}",
        get(move |Path(email): Path<String>| {
            let mail = mail.clone();
            async move {
                let sent = mail.sent.lock().unwrap();
                let code = sent
                    .iter()
                    .rev()
                    .find(|(to, _)| to == &email)
                    .map(|(_, code)| code.clone());
                Json(json!({"code": code}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new("node")
            .arg("apps/mobile-web/tests/browser/continuity.mjs")
            .current_dir(workspace)
            .env("QIBAN_TEST_UPSTREAM", format!("http://{address}"))
            .status()
            .expect("Node is required for the mobile integration test")
    })
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    assert!(status.success(), "mobile browser integration failed");
}
impl Fixture {
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!("qiban-http-auth-{}.db", uuid::Uuid::new_v4())),
            mail: Arc::new(TestMail::default()),
        }
    }
    fn app(&self) -> Router {
        let store = Arc::new(AccountStore::open(&self.path).unwrap());
        let auth = NativeAuth::new(
            store.clone(),
            self.mail.clone(),
            [42; 32],
            &["alice@example.com".into(), "bob@example.com".into()],
        )
        .unwrap();
        router(AppState::new(auth, store))
    }
    fn code(&self, email: &str) -> String {
        self.mail
            .sent
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(to, _)| to == email)
            .unwrap()
            .1
            .clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status();
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}
async fn login(f: &Fixture, app: &Router, email: &str) -> String {
    let (status, receipt) = call(
        app,
        "POST",
        "/v1/auth/request-code",
        None,
        json!({"email":email}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, grant) = call(
        app,
        "POST",
        "/v1/auth/verify-code",
        None,
        json!({"challengeId":receipt["challengeId"],"code":f.code(email),"nonce":nonce()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(grant["idleTimeoutSeconds"], 1800);
    grant["accessToken"].as_str().unwrap().to_string()
}
#[tokio::test]
async fn real_otp_flow_enforces_two_account_ownership_and_task_dedup() {
    let f = Fixture::new();
    let app = f.app();
    let a = login(&f, &app, "alice@example.com").await;
    let b = login(&f, &app, "bob@example.com").await;
    let pa = call(&app, "GET", "/v1/me", Some(&a), Value::Null).await.1;
    let pb = call(&app, "GET", "/v1/me", Some(&b), Value::Null).await.1;
    assert_ne!(pa, pb);
    let command = json!({"requestId":uuid::Uuid::new_v4().to_string(),"title":"private task"});
    let (status, task) = call(&app, "POST", "/v1/tasks", Some(&a), command.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        task,
        call(&app, "POST", "/v1/tasks", Some(&a), command.clone())
            .await
            .1
    );
    assert_eq!(
        call(&app, "GET", "/v1/tasks", Some(&b), Value::Null)
            .await
            .1,
        json!([])
    );
    let path = format!("/v1/tasks/{}", task["id"].as_str().unwrap());
    assert_eq!(
        call(&app, "GET", &path, Some(&b), Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/cancel"),
            Some(&b),
            json!({"revision":0})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut forged = command.clone();
    forged["accountId"] = pa["accountId"].clone();
    assert_eq!(
        call(&app, "POST", "/v1/tasks", Some(&b), forged).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut changed = command.clone();
    changed["title"] = json!("changed");
    assert_eq!(
        call(&app, "POST", "/v1/tasks", Some(&a), changed).await.0,
        StatusCode::CONFLICT
    );
    assert_ne!(
        task["id"],
        call(&app, "POST", "/v1/tasks", Some(&b), command).await.1["id"]
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/cancel"),
            Some(&a),
            json!({"revision":0})
        )
        .await
        .1["status"],
        "cancelled"
    );
}
#[tokio::test]
async fn same_nonce_replays_recover_one_session_and_other_nonces_are_rejected() {
    let f = Fixture::new();
    let app = f.app();
    let (_, receipt) = call(
        &app,
        "POST",
        "/v1/auth/request-code",
        None,
        json!({"email":"alice@example.com"}),
    )
    .await;
    let shared = nonce();
    let body = json!({"challengeId":receipt["challengeId"],"code":f.code("alice@example.com"),"nonce":shared});
    // A retried verify whose first response was lost re-derives the same
    // session token; concurrent retries with one nonce also converge.
    let (a, b) = tokio::join!(
        call(&app, "POST", "/v1/auth/verify-code", None, body.clone()),
        call(&app, "POST", "/v1/auth/verify-code", None, body.clone())
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert_eq!(a.1["accessToken"], b.1["accessToken"]);
    assert_eq!(a.1["sessionId"], b.1["sessionId"]);
    assert_eq!(
        call(&app, "POST", "/v1/auth/verify-code", None, body)
            .await
            .0,
        StatusCode::OK
    );
    // The consumed code cannot mint a session for a different nonce.
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/auth/verify-code",
            None,
            json!({"challengeId":receipt["challengeId"],"code":f.code("alice@example.com"),"nonce":nonce()})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn concurrent_same_email_code_requests_allow_exactly_one() {
    let f = Fixture::new();
    let app = f.app();
    let (a, b) = tokio::join!(
        call(
            &app,
            "POST",
            "/v1/auth/request-code",
            None,
            json!({"email":"alice@example.com"})
        ),
        call(
            &app,
            "POST",
            "/v1/auth/request-code",
            None,
            json!({"email":"alice@example.com"})
        )
    );
    let statuses = [a.0, b.0];
    assert_eq!(
        statuses.iter().filter(|s| **s == StatusCode::OK).count(),
        1,
        "expected exactly one accepted request, got {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::TOO_MANY_REQUESTS)
            .count(),
        1
    );
}
#[tokio::test]
async fn wrong_codes_consume_attempts_and_resend_is_throttled() {
    let f = Fixture::new();
    let app = f.app();
    let (_, receipt) = call(
        &app,
        "POST",
        "/v1/auth/request-code",
        None,
        json!({"email":"alice@example.com"}),
    )
    .await;
    let correct = f.code("alice@example.com");
    let wrong = if correct == "00000000" {
        "11111111"
    } else {
        "00000000"
    };
    for _ in 0..5 {
        assert_eq!(
            call(
                &app,
                "POST",
                "/v1/auth/verify-code",
                None,
                json!({"challengeId":receipt["challengeId"],"code":wrong,"nonce":nonce()})
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/auth/verify-code",
            None,
            json!({"challengeId":receipt["challengeId"],"code":correct,"nonce":nonce()})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/auth/request-code",
            None,
            json!({"email":"ALICE@example.com"})
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(f.mail.sent.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn delivery_failure_never_activates_a_code_or_returns_secrets() {
    let f = Fixture::new();
    let app = f.app();
    f.mail.fail.store(true, Ordering::SeqCst);
    let (status, body) = call(
        &app,
        "POST",
        "/v1/auth/request-code",
        None,
        json!({"email":"alice@example.com"}),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, json!({"error":{"code":"authentication_unavailable"}}));
    assert!(!body.to_string().contains(&f.code("alice@example.com")));
    let (status, body) = call(
        &app,
        "POST",
        "/v1/auth/request-code",
        None,
        json!({"email":"not-invited@example.com"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_object().unwrap().len(), 1);
    assert_eq!(f.mail.sent.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn session_reopens_revocation_persists_and_raw_token_is_not_stored() {
    let f = Fixture::new();
    let app = f.app();
    let token = login(&f, &app, "alice@example.com").await;
    let profile = call(&app, "GET", "/v1/me", Some(&token), Value::Null)
        .await
        .1;
    drop(app);
    let bytes = std::fs::read(&f.path).unwrap();
    assert!(!bytes.windows(token.len()).any(|w| w == token.as_bytes()));
    let app = f.app();
    assert_eq!(
        profile,
        call(&app, "GET", "/v1/me", Some(&token), Value::Null)
            .await
            .1
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/logout",
            Some(&token),
            json!({"allSessions":true})
        )
        .await
        .1,
        json!({"revoked":true,"allSessions":true})
    );
    drop(app);
    let app = f.app();
    assert_eq!(
        call(&app, "GET", "/v1/me", Some(&token), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/logout",
            Some(&token),
            json!({"allSessions":false})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn invalid_authorization_and_extra_fields_are_rejected() {
    let f = Fixture::new();
    let app = f.app();
    assert_eq!(
        call(&app, "GET", "/v1/me", None, Value::Null).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/v1/me",
            Some("unsigned.jwt.claims"),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let token = login(&f, &app, "alice@example.com").await;
    let req = Request::builder()
        .uri("/v1/me")
        .header("Authorization", format!("Bearer {token}"))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/auth/request-code",
            None,
            json!({"email":"bob@example.com","accountId":"victim"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/tasks",
            Some(&token),
            json!({"requestId":"r","title":"x".repeat(9000)})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn tcp_server_handles_authenticated_profile_request() {
    let f = Fixture::new();
    let app = f.app();
    let token = login(&f, &app, "alice@example.com").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/me", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn pairing_http_rejects_owner_injection_self_pairing_and_cross_account() {
    let f = Fixture::new();
    let app = f.app();
    let a = login(&f, &app, "alice@example.com").await;
    let b = login(&f, &app, "bob@example.com").await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/pairings/offer",
            None,
            json!({"name":"电脑"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/pairings/offer",
            Some(&a),
            json!({"name":"电脑","accountId":"other"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, o) = call(
        &app,
        "POST",
        "/v1/pairings/offer",
        Some(&a),
        json!({"name":"电脑"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for token in [&a, &b] {
        assert_eq!(
            call(
                &app,
                "POST",
                "/v1/pairings/preview",
                Some(token),
                json!({"code":o["code"]})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    let path = format!(
        "/v1/pairings/{}/revoke",
        o["pairing"]["id"].as_str().unwrap()
    );
    assert_eq!(
        call(&app, "POST", &path, Some(&b), json!({"revision":1}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "GET", "/v1/pairings", Some(&b), Value::Null)
            .await
            .1,
        json!([])
    );
    let listed = call(&app, "GET", "/v1/pairings", Some(&a), Value::Null)
        .await
        .1;
    assert!(!listed.to_string().contains(o["code"].as_str().unwrap()));
    assert_eq!(
        call(&app, "POST", &path, Some(&a), json!({"revision":1}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", &path, Some(&a), json!({"revision":1}))
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn short_pairing_code_http_limit_is_durable_and_has_distinct_error() {
    let f = Fixture::new();
    let app = f.app();
    let token = login(&f, &app, "alice@example.com").await;
    let (status, offer) = call(
        &app,
        "POST",
        "/v1/pairings/offer",
        Some(&token),
        json!({"name":"电脑"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = offer["code"].as_str().unwrap();
    assert_eq!(code.len(), 6);
    assert!(code.bytes().all(|b| b.is_ascii_digit()));
    for n in 1..=5 {
        let (status, body) = call(
            &app,
            "POST",
            "/v1/pairings/preview",
            Some(&token),
            json!({"code":"invalid"}),
        )
        .await;
        assert_eq!(
            status,
            if n == 5 {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::FORBIDDEN
            }
        );
        if n == 5 {
            assert_eq!(body["error"]["code"], "pairing_rate_limited");
        }
    }
    drop(app);
    let reopened = f.app();
    let (status, body) = call(
        &reopened,
        "POST",
        "/v1/pairings/accept",
        Some(&token),
        json!({"code":code,"pairingId":offer["pairing"]["id"],"name":"手机"}),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "pairing_rate_limited");
    assert_eq!(
        call(&reopened, "GET", "/v1/me", Some(&token), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn device_heartbeat_updates_own_row_and_pairings_report_presence() {
    let f = Fixture::new();
    let app = f.app();
    let alice = login(&f, &app, "alice@example.com").await;

    // No bearer → 401.
    let (status, _) = call(
        &app,
        "POST",
        "/v1/devices/heartbeat",
        None,
        json!({"capabilities":["document_excerpt"]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Logged in but never offered/accepted: no device row → idempotent false.
    let (status, body) = call(
        &app,
        "POST",
        "/v1/devices/heartbeat",
        Some(&alice),
        json!({"capabilities":["document_excerpt"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["updated"], false);

    // Offer from the desktop session creates its device row.
    let (status, offer) = call(
        &app,
        "POST",
        "/v1/pairings/offer",
        Some(&alice),
        json!({"name":"我的电脑"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(
        &app,
        "POST",
        "/v1/devices/heartbeat",
        Some(&alice),
        json!({"capabilities":["document_excerpt"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["updated"], true);

    // The pairing payload now carries the presence family.
    let (status, pairings) = call(&app, "GET", "/v1/pairings", Some(&alice), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let row = &pairings[0];
    assert_eq!(row["id"], offer["pairing"]["id"]);
    assert_eq!(row["desktopOnline"], true);
    assert!(row["desktopLastHeartbeatAt"].is_u64());
    assert_eq!(row["desktopCapabilities"], json!(["document_excerpt"]));

    // Validation: unknown slug, over-cap list, unknown field all 400.
    let (status, _) = call(
        &app,
        "POST",
        "/v1/devices/heartbeat",
        Some(&alice),
        json!({"capabilities":["teleport"]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/devices/heartbeat",
        Some(&alice),
        json!({"capabilities":["document_excerpt","document_excerpt","document_excerpt",
                               "document_excerpt","document_excerpt","document_excerpt",
                               "document_excerpt","document_excerpt","document_excerpt"]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/devices/heartbeat",
        Some(&alice),
        json!({"capabilities":[],"extra":1}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Revoked session (logout) → 401: an old session can never report online.
    let (status, _) = call(
        &app,
        "POST",
        "/v1/logout",
        Some(&alice),
        json!({"allSessions":false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "POST",
        "/v1/devices/heartbeat",
        Some(&alice),
        json!({"capabilities":[]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// Document endpoints previously had zero HTTP coverage. The storage-layer
// tests stay authoritative for the state-machine semantics (including the
// 5-minute expiry and same-account observer isolation — no HTTP clock or
// third-session seam exists); this covers routes, status codes and payloads.
// The controller session needs a second alice login, so the real 60-second
// OTP mail cooldown is waited out once — same rationale as mobile-peer.mjs.
#[tokio::test]
async fn documents_http_enforce_roles_transitions_cancel_and_unknown() {
    let f = Fixture::new();
    let app = f.app();
    let desktop = login(&f, &app, "alice@example.com").await;
    tokio::time::sleep(std::time::Duration::from_secs(61)).await;
    let controller = login(&f, &app, "alice@example.com").await;
    let (_, offer) = call(
        &app,
        "POST",
        "/v1/pairings/offer",
        Some(&desktop),
        json!({"name":"客厅电脑"}),
    )
    .await;
    let (status, pairing) = call(
        &app,
        "POST",
        "/v1/pairings/accept",
        Some(&controller),
        json!({"code":offer["code"],"pairingId":offer["pairing"]["id"],"name":"手机"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pairing["status"], "active");
    let share = |source: &str, preview: &str| {
        json!({
            "pairingId": pairing["id"],
            "binding": {
                "actionId": uuid::Uuid::new_v4().to_string(),
                "resourceId": uuid::Uuid::new_v4().to_string(),
                "resourceVersion": 1,
                "parametersDigest": companion_core::authorization::document_digest(source, preview),
                "pairRevision": pairing["revision"],
                "scope": "document_excerpt"
            },
            "sourceName": source,
            "preview": preview
        })
    };

    // Happy chain: share → confirm → admit → receipt, with every role and
    // idempotency boundary asserted at the HTTP layer.
    let (status, doc) = call(
        &app,
        "POST",
        "/v1/documents",
        Some(&desktop),
        share("转移.txt", "手机端六态回归的第一份摘录"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = doc["authorization"]["binding"]["actionId"]
        .as_str()
        .unwrap();
    let binding = doc["authorization"]["binding"].clone();
    let artifact_hash = doc["artifactHash"].as_str().unwrap().to_string();
    assert_eq!(doc["authorization"]["state"], "awaiting_confirmation");
    assert_eq!(doc["currentRole"], "desktop");
    let path = format!("/v1/documents/{id}");
    let (status, list) = call(&app, "GET", "/v1/documents", Some(&controller), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["authorization"]["state"], "awaiting_confirmation");
    assert_eq!(list[0]["currentRole"], "controller");
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/confirm"),
            Some(&desktop),
            binding.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/confirm"),
            Some(&controller),
            binding.clone()
        )
        .await
        .1["authorization"]["state"],
        "confirmed"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/confirm"),
            Some(&controller),
            binding.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/admit"),
            Some(&controller),
            binding.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/admit"),
            Some(&desktop),
            binding.clone()
        )
        .await
        .1["authorization"]["state"],
        "admitted"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/admit"),
            Some(&desktop),
            binding.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/receipt"),
            Some(&controller),
            json!({"binding":binding,"state":"completed","artifactHash":artifact_hash})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/receipt"),
            Some(&desktop),
            json!({"binding":binding,"state":"completed","artifactHash":"c".repeat(64)})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let receipt = json!({"binding":binding,"state":"completed","artifactHash":artifact_hash});
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/receipt"),
            Some(&desktop),
            receipt.clone()
        )
        .await
        .1["authorization"]["state"],
        "completed"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/receipt"),
            Some(&desktop),
            receipt.clone()
        )
        .await
        .1["authorization"]["state"],
        "completed"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/receipt"),
            Some(&desktop),
            json!({"binding":receipt["binding"],"state":"failed","artifactHash":artifact_hash})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );

    // A cancelled-before-admission task never reaches the desktop executor.
    let (_, cancelled) = call(
        &app,
        "POST",
        "/v1/documents",
        Some(&desktop),
        share("取消.txt", "确认前取消的摘录"),
    )
    .await;
    let cancelled_id = cancelled["authorization"]["binding"]["actionId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/v1/documents/{cancelled_id}/cancel"),
            Some(&controller),
            json!({})
        )
        .await
        .1["authorization"]["state"],
        "cancelled"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/v1/documents/{cancelled_id}/admit"),
            Some(&desktop),
            cancelled["authorization"]["binding"].clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );

    // An admitted task can be stopped by request, report unknown, and still
    // resolve to a verified outcome afterwards.
    let (_, doc) = call(
        &app,
        "POST",
        "/v1/documents",
        Some(&desktop),
        share("未知.txt", "先未知后补齐的摘录"),
    )
    .await;
    let id = doc["authorization"]["binding"]["actionId"]
        .as_str()
        .unwrap();
    let binding = doc["authorization"]["binding"].clone();
    let artifact_hash = doc["artifactHash"].as_str().unwrap().to_string();
    let path = format!("/v1/documents/{id}");
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/confirm"),
            Some(&controller),
            binding.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/admit"),
            Some(&desktop),
            binding.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/cancel"),
            Some(&controller),
            json!({})
        )
        .await
        .1["authorization"]["state"],
        "cancel_requested"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/receipt"),
            Some(&desktop),
            json!({"binding":binding.clone(),"state":"unknown","artifactHash":artifact_hash})
        )
        .await
        .1["authorization"]["state"],
        "unknown"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{path}/receipt"),
            Some(&desktop),
            json!({"binding":binding,"state":"completed","artifactHash":artifact_hash})
        )
        .await
        .1["authorization"]["state"],
        "completed"
    );

    // Cross-account sessions never see another account's tasks (same-account
    // observer isolation is asserted at the storage layer).
    let bob = login(&f, &app, "bob@example.com").await;
    assert_eq!(
        call(&app, "GET", "/v1/documents", Some(&bob), Value::Null)
            .await
            .1,
        json!([])
    );
}

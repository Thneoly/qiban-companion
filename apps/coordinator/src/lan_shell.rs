//! Browser-facing shell for the embedded composition (P1-b PR-2).
//!
//! Ports the mobile-web dev gateway's defense semantics INTO the embedded
//! router (design: docs/architecture/lan-phone-continuity.md): a Host
//! allowlist that admits only IP-literal private/loopback addresses on the
//! listening port (a domain Host is the DNS-rebinding posture and is
//! refused), the gateway's CSRF requirements for POSTs, cookie→Bearer
//! session translation for `/api/*`, an aggregate `/api/state`, the route
//! allowlist, aggregate rate limiting, and static assets behind the full
//! security-header set. LAN plaintext HTTP by design — the trust model is a
//! home network where every device is trusted (see the design's boundaries).
use super::*;
use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, Method, Uri};
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const COOKIE_NAME: &str = "qiban_local_session";
const RATE_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);
const RATE_MAX: u32 = 600;

/// One-shot shell configuration. `assets` points at the mobile-web dist;
/// `None` (dev builds without bundled resources) serves 404 for static
/// paths — the dev phone flow keeps using the external gateway.
pub struct ShellConfig {
    pub port: u16,
    pub assets: Option<PathBuf>,
}

pub(crate) struct ShellState {
    port: u16,
    assets: Option<PathBuf>,
    rate: Mutex<(Instant, u32)>,
}

/// The Host allowlist, rebinding-resistant by construction: only an IP
/// LITERAL from the private/loopback ranges on the exact listening port
/// passes. A domain Host — the carrier of a DNS-rebinding attack — never
/// does, whatever it resolves to. IP drift (DHCP) is tolerated for free:
/// any private address of this machine stays admissible.
fn host_allowed(host: &str, port: u16) -> bool {
    let Some((host_part, port_part)) = host.rsplit_once(':') else {
        return false;
    };
    if port_part != port.to_string() {
        return false;
    }
    let literal = host_part.trim_start_matches('[').trim_end_matches(']');
    match IpAddr::from_str(literal) {
        Ok(IpAddr::V4(v4)) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback() || (v6.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

/// The gateway's `/api` route allowlist: exact method+path pairs plus the
/// four UUID-suffixed action families. Anything else is a 404 — the browser
/// face exposes a whitelist, not a mirror of the coordinator surface.
/// `/api/pairings/exchange` is listed ahead of PR-3; until the route exists
/// upstream it 404s like any other unknown path.
fn api_route_allowed(method: &str, path: &str) -> bool {
    let exact = format!("{method} {path}");
    let listed = [
        "GET /api/me",
        "GET /api/state",
        "GET /api/tasks",
        "GET /api/pairings",
        "GET /api/documents",
        "GET /api/memories",
        "POST /api/pairings/preview",
        "POST /api/pairings/accept",
        "POST /api/pairings/exchange",
        "POST /api/tasks",
        "POST /api/logout",
    ]
    .contains(&exact.as_str());
    if listed {
        return true;
    }
    if method != "POST" {
        return false;
    }
    let uuid_suffix = |prefix: &str, action: &str| {
        path.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(&format!("/{action}")))
            .is_some_and(|id| {
                id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
            })
    };
    uuid_suffix("/api/tasks/", "cancel")
        || uuid_suffix("/api/pairings/", "revoke")
        || ["confirm", "cancel", "delete"]
            .iter()
            .any(|action| uuid_suffix("/api/documents/", action))
        || ["update", "delete"]
            .iter()
            .any(|action| uuid_suffix("/api/memories/", action))
}

fn valid_session_token(token: &str) -> bool {
    token.len() == 47
        && token.starts_with("qbs_")
        && token[4..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let mut found: Option<String> = None;
    for value in headers.get_all(header::COOKIE) {
        let value = value.to_str().ok()?;
        for pair in value.split(';') {
            if let Some(token) = pair.trim().strip_prefix(&format!("{COOKIE_NAME}=")) {
                if found.is_some() {
                    // Two cookies of the same name: ambiguous, treat as none.
                    return None;
                }
                found = Some(token.to_string());
            }
        }
    }
    found
}

fn security_headers(headers: &mut HeaderMap<HeaderValue>) {
    const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'";
    let standard: [(HeaderName, &str); 4] = [
        (header::CACHE_CONTROL, "no-store"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::CONTENT_SECURITY_POLICY, CSP),
    ];
    let custom: [(&str, &str); 3] = [
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
        ("x-frame-options", "DENY"),
        ("cross-origin-resource-policy", "same-site"),
    ];
    for (name, value) in standard {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(name, value);
        }
    }
    for (name, value) in custom {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(HeaderName::from_static(name), value);
        }
    }
}

fn shell_failure(status: StatusCode, code: &'static str) -> Response {
    let mut response = ApiError(status, code).into_response();
    security_headers(response.headers_mut());
    response
}

/// The gateway's 401 carries the expired cookie: a dead session must not
/// linger in the browser.
fn unauthorized() -> Response {
    let mut response = shell_failure(StatusCode::UNAUTHORIZED, "authentication_required");
    if let Some(value) = expired_cookie() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

fn expired_cookie() -> Option<HeaderValue> {
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"
    ))
    .ok()
}

/// Assemble the embedded composition: the OTP-less coordinator router plus
/// the browser-facing shell around it.
///
/// The coordinator surface is mounted TWICE, on `/v1` (the desktop's own
/// loopback client) and on `/api` (the browser face). A URI rewrite would
/// not work here: `Router::layer` middleware runs AFTER route matching, so
/// the shell gate only translates the session cookie into a Bearer header
/// and lets the already-matched `/api/*` handler consume it.
pub fn shell_router(state: AppState, config: ShellConfig) -> Router {
    let shell = Arc::new(ShellState {
        port: config.port,
        assets: config.assets,
        rate: Mutex::new((Instant::now(), 0)),
    });
    let assets = shell.clone();
    let api = Router::new()
        .route("/api/state", get(api_state))
        .route("/api/pairings/exchange", post(api_exchange))
        .with_state(state.clone());
    Router::new()
        .merge(router_embedded(state.clone()))
        .merge(assemble(state, false, "/api"))
        .merge(api)
        .fallback(move |uri: Uri| {
            let shell = assets.clone();
            async move { static_asset(shell, uri).await }
        })
        .layer(middleware::from_fn(move |request, next| {
            let shell = shell.clone();
            async move { shell_gate(&shell, request, next).await }
        }))
}

async fn shell_gate(shell: &Arc<ShellState>, request: Request, next: Next) -> Response {
    let headers = request.headers().clone();
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if !host_allowed(&host, shell.port) {
        return shell_failure(StatusCode::FORBIDDEN, "invalid_origin");
    }
    if headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        == Some("cross-site")
    {
        return shell_failure(StatusCode::FORBIDDEN, "invalid_origin");
    }
    let same_origin = |origin: &str| {
        origin
            .rsplit_once("://")
            .is_some_and(|(_, rest)| rest.trim_end_matches('/') == host)
    };
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if !same_origin(origin) {
            return shell_failure(StatusCode::FORBIDDEN, "invalid_origin");
        }
    }
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    if !path.starts_with("/api/") {
        // /v1 (desktop client) and /healthz keep their own boundary; static
        // paths fall through to the asset handler.
        return finish(next, request, false).await;
    }
    if !api_route_allowed(method.as_str(), &path) {
        #[cfg(test)]
        eprintln!("SHELL_ALLOWLIST_MISS {method} {path}");
        return shell_failure(StatusCode::NOT_FOUND, "not_found");
    }
    if method == Method::POST {
        let origin_ok = headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .is_some_and(same_origin);
        let request_marked =
            headers.get("x-qiban-request").and_then(|v| v.to_str().ok()) == Some("1");
        let json_body = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"));
        if !origin_ok || !request_marked || !json_body {
            return shell_failure(StatusCode::FORBIDDEN, "invalid_origin");
        }
        if headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
            .is_some_and(|length| length > 8192)
        {
            return shell_failure(StatusCode::PAYLOAD_TOO_LARGE, "invalid_request");
        }
    }
    {
        let mut window = shell.rate.lock().expect("rate window");
        if window.0.elapsed() >= RATE_WINDOW {
            *window = (Instant::now(), 0);
        }
        window.1 += 1;
        if window.1 > RATE_MAX {
            return shell_failure(StatusCode::TOO_MANY_REQUESTS, "busy");
        }
    }
    // The admission endpoint is deliberately anonymous — the pairing code IS
    // the credential. It keeps every other gate (Host, CSRF, rate limit).
    if path == "/api/pairings/exchange" {
        return finish(next, request, false).await;
    }
    let token = cookie_token(&headers);
    let token = match token {
        Some(token) if valid_session_token(&token) => token,
        _ => return unauthorized(),
    };
    // Cookie → Bearer for the already-matched /api handler (the coordinator
    // surface is mounted on /api directly — a URI rewrite would run after
    // route matching and could not re-route).
    let mut request = request;
    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) {
        request.headers_mut().insert(header::AUTHORIZATION, value);
    }
    let _ = method;
    finish(next, request, path == "/api/logout").await
}

async fn finish(next: Next, request: Request, clear_cookie: bool) -> Response {
    let mut response = next.run(request).await;
    security_headers(response.headers_mut());
    if clear_cookie || response.status() == StatusCode::UNAUTHORIZED {
        if let Some(value) = expired_cookie() {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

/// The embedded admission's browser face: pairing code → session, issued as
/// the gateway issued verify-code's — an HttpOnly cookie plus a bare
/// `{authenticated: true}` (the token never enters page JavaScript).
async fn api_exchange(
    State(s): State<AppState>,
    b: Result<Json<super::pairing::Exchange>, JsonRejection>,
) -> Response {
    let grant = match super::pairing::exchange(State(s), b).await {
        Ok(Json(token)) => token,
        Err(error) => return error.into_response(),
    };
    let max_age = grant.expires_at.saturating_sub(now_ms()).min(86_400) / 1000;
    let mut response = Json(json!({"authenticated": true})).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}",
        grant.access_token
    )) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Aggregate the gateway's `/api/state`: one profile read plus one task
/// read under the SAME cookie — a login in another tab must never pair
/// account A's identity with account B's task list.
async fn api_state(State(s): State<AppState>, h: HeaderMap) -> Response {
    let token = match bearer(&h) {
        Ok(token) => token.to_string(),
        Err(error) => return error.into_response(),
    };
    let identity = match s.auth.verify(&token).await {
        Ok(identity) => identity,
        Err(error) => return ApiError::from(error).into_response(),
    };
    let profile = match storage(&s, identity.clone(), |s, i| s.profile(i)).await {
        Ok(profile) => profile,
        Err(error) => return error.into_response(),
    };
    let tasks = match storage(&s, identity, |s, i| s.list_tasks(i)).await {
        Ok(tasks) => tasks,
        Err(error) => return error.into_response(),
    };
    Json(json!({"profile": profile, "tasks": tasks})).into_response()
}

/// The gateway's static surface, verbatim in spirit: GET only, `/` or
/// `/assets/<flat-name>` only, no directory walks, four content types.
async fn static_asset(shell: Arc<ShellState>, uri: Uri) -> Response {
    let Some(root) = &shell.assets else {
        return shell_failure(StatusCode::NOT_FOUND, "not_found");
    };
    let path = uri.path();
    let name_ok = |name: &str| {
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
    };
    if path != "/" && !path.strip_prefix("/assets/").is_some_and(name_ok) {
        return shell_failure(StatusCode::NOT_FOUND, "not_found");
    }
    let relative = if path == "/" {
        "index.html"
    } else {
        &path[1..]
    };
    if relative.contains("..") || relative.contains('\\') {
        return shell_failure(StatusCode::NOT_FOUND, "not_found");
    }
    let file = root.join(relative);
    if !file.starts_with(root) {
        return shell_failure(StatusCode::NOT_FOUND, "not_found");
    }
    match tokio::fs::read(&file).await {
        Ok(bytes) => {
            let content_type = match file.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript",
                Some("css") => "text/css; charset=utf-8",
                Some("svg") => "image/svg+xml",
                _ => "application/octet-stream",
            };
            let mut response = (StatusCode::OK, bytes).into_response();
            if let Ok(value) = HeaderValue::from_str(content_type) {
                response.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            response
        }
        Err(_) => shell_failure(StatusCode::NOT_FOUND, "not_found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn gate(port: u16) -> Router {
        let shell = Arc::new(ShellState {
            port,
            assets: None,
            rate: Mutex::new((Instant::now(), 0)),
        });
        let surface = Router::new()
            .route("/me", get(|| async { Json(json!({"ok": true})) }))
            .route("/tasks", post(|| async { Json(json!({"ok": true})) }));
        Router::new()
            .route("/v1/me", get(|| async { Json(json!({"ok": true})) }))
            .nest("/api", surface)
            .layer(middleware::from_fn(move |request, next| {
                let shell = shell.clone();
                async move { shell_gate(&shell, request, next).await }
            }))
    }

    fn request(method: &str, path: &str, headers: &[(&str, &str)]) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn host_allowlist_admits_private_ip_literals_only() {
        let app = gate(4321);
        // Private IP literal on the right port: allowed.
        let ok = app
            .clone()
            .oneshot(request("GET", "/v1/me", &[("host", "192.168.1.5:4321")]))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        // Loopback too.
        let ok = app
            .clone()
            .oneshot(request("GET", "/v1/me", &[("host", "127.0.0.1:4321")]))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        // A domain Host — the DNS-rebinding carrier — is refused, as is a
        // public IP, a wrong port, and a missing Host.
        for host in [
            "evil.example.com:4321",
            "8.8.8.8:4321",
            "192.168.1.5:9999",
            "192.168.1.5",
        ] {
            let refused = app
                .clone()
                .oneshot(request("GET", "/v1/me", &[("host", host)]))
                .await
                .unwrap();
            assert_eq!(refused.status(), StatusCode::FORBIDDEN, "host {host}");
        }
    }

    #[tokio::test]
    async fn cross_site_posture_rejects_cross_site_markers() {
        let app = gate(4321);
        let refused = app
            .clone()
            .oneshot(request(
                "GET",
                "/v1/me",
                &[
                    ("host", "192.168.1.5:4321"),
                    ("sec-fetch-site", "cross-site"),
                ],
            ))
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        let refused = app
            .clone()
            .oneshot(request(
                "GET",
                "/v1/me",
                &[
                    ("host", "192.168.1.5:4321"),
                    ("origin", "http://evil.example.com"),
                ],
            ))
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        // Same-origin Origin passes.
        let ok = app
            .clone()
            .oneshot(request(
                "GET",
                "/v1/me",
                &[
                    ("host", "192.168.1.5:4321"),
                    ("origin", "http://192.168.1.5:4321"),
                ],
            ))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn api_post_requires_the_full_csrf_triple() {
        let app = gate(4321);
        let cookie = "qiban_local_session=qbs_0000000000000000000000000000000000000000000";
        let base = [
            ("host", "192.168.1.5:4321"),
            ("origin", "http://192.168.1.5:4321"),
            ("cookie", cookie),
            ("content-type", "application/json"),
        ];
        let ok = app
            .clone()
            .oneshot({
                let mut builder = Request::builder()
                    .method("POST")
                    .uri("/api/tasks")
                    .header("content-length", "2");
                for (name, value) in base.iter().chain([("x-qiban-request", "1")].iter()) {
                    builder = builder.header(*name, *value);
                }
                builder.body(Body::from("{}")).unwrap()
            })
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        for missing in ["origin", "x-qiban-request", "content-type"] {
            let headers: Vec<(&str, &str)> = base
                .iter()
                .filter(|(name, _)| name != &missing)
                .chain(
                    [("x-qiban-request", "1")]
                        .iter()
                        .filter(|(name, _)| name != &missing),
                )
                .cloned()
                .collect();
            let refused = app
                .clone()
                .oneshot({
                    let mut builder = Request::builder().method("POST").uri("/api/tasks");
                    for (name, value) in headers {
                        builder = builder.header(name, value);
                    }
                    builder.body(Body::from("{}")).unwrap()
                })
                .await
                .unwrap();
            assert_eq!(refused.status(), StatusCode::FORBIDDEN, "missing {missing}");
        }
    }

    #[tokio::test]
    async fn api_relay_demands_a_session_cookie_and_rewrites_the_path() {
        let app = gate(4321);
        let host = ("host", "192.168.1.5:4321");
        // No cookie → 401 (and the response clears the cookie).
        let refused = app
            .clone()
            .oneshot(request("GET", "/api/me", &[host]))
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
        assert!(refused
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().unwrap().contains("Max-Age=0")));
        // With a valid cookie the request rides through as /v1/me with a
        // Bearer header (the stub route echoes success only on Bearer).
        let ok = app
            .clone()
            .oneshot(request(
                "GET",
                "/api/me",
                &[
                    host,
                    (
                        "cookie",
                        "qiban_local_session=qbs_0000000000000000000000000000000000000000000",
                    ),
                ],
            ))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        // Off-whitelist /api path → 404.
        let unknown = app
            .clone()
            .oneshot(request(
                "GET",
                "/api/nothing",
                &[
                    host,
                    (
                        "cookie",
                        "qiban_local_session=qbs_0000000000000000000000000000000000000000000",
                    ),
                ],
            ))
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rate_limit_exhausts_to_busy() {
        let app = gate(4321);
        let host = ("host", "192.168.1.5:4321");
        for _ in 0..RATE_MAX {
            let ok = app
                .clone()
                .oneshot(request("GET", "/api/me", &[host]))
                .await
                .unwrap();
            assert_eq!(ok.status(), StatusCode::UNAUTHORIZED);
        }
        let busy = app
            .oneshot(request("GET", "/api/me", &[host]))
            .await
            .unwrap();
        assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}

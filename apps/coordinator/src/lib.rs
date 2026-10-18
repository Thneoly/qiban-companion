pub mod auth;
mod devices;
mod documents;
pub mod lan_shell;
mod memories;
mod pairing;

use auth::{AuthError, NativeAuth};
use axum::{
    extract::{rejection::JsonRejection, DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use companion_core::{
    identity::{CreateAccountTask, IdentityError, VerifiedIdentity},
    Task,
};
use companion_storage::{accounts::AccountStore, StorageError};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct AppState {
    auth: Arc<NativeAuth>,
    store: Arc<AccountStore>,
    requests: Arc<Semaphore>,
}

impl AppState {
    pub fn new(auth: NativeAuth, store: Arc<AccountStore>) -> Self {
        Self {
            auth: Arc::new(auth),
            store,
            requests: Arc::new(Semaphore::new(16)),
        }
    }
}

pub fn router(state: AppState) -> Router {
    with_health(assemble(state, true, "/v1"))
}

/// The embedded composition (P1-b): the desktop process hosts the router
/// itself. Email OTP routes are NOT registered — the embedded form has no
/// mailer and mints its local session in-process; not mounting them (rather
/// than disabling them) keeps the attack surface a routing fact, and the
/// closed-invitation allowlist in NativeAuth stays as the second backstop.
pub fn router_embedded(state: AppState) -> Router {
    with_health(assemble(state, false, "/v1"))
}

// Health checks stay outside the request limiter so a saturated mail
// pipeline cannot hide process liveness from operators and launch scripts.
fn with_health(router: Router) -> Router {
    router.route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
}

/// Mount the coordinator surface under `prefix` ("/v1" for the desktop's
/// loopback client; "/api" for the embedded browser face — same handlers,
/// two mounts, because a URI rewrite cannot re-route after matching).
pub(crate) fn assemble(state: AppState, email_auth: bool, prefix: &str) -> Router {
    let at = |path: &str| format!("{prefix}{path}");
    let gated = Router::new();
    let gated = if email_auth {
        gated
            .route(at("/auth/request-code").as_str(), post(request_code))
            .route(at("/auth/verify-code").as_str(), post(verify_code))
    } else {
        gated
    };
    let gated = gated
        .route(at("/me").as_str(), get(profile))
        .route(at("/tasks").as_str(), get(list_tasks).post(create_task))
        .route(at("/tasks/{id}").as_str(), get(get_task))
        .route(at("/tasks/{id}/cancel").as_str(), post(cancel_task))
        .route(at("/logout").as_str(), post(sign_out))
        .route(at("/devices/heartbeat").as_str(), post(devices::heartbeat))
        .route(at("/pairings").as_str(), get(pairing::list))
        .route(at("/pairings/offer").as_str(), post(pairing::offer))
        .route(at("/pairings/preview").as_str(), post(pairing::preview))
        .route(at("/pairings/accept").as_str(), post(pairing::accept))
        .route(at("/pairings/{id}/revoke").as_str(), post(pairing::revoke))
        .route(
            at("/documents").as_str(),
            get(documents::list).post(documents::share),
        )
        .route(at("/documents/{id}").as_str(), get(documents::get_one))
        .route(
            at("/documents/{id}/confirm").as_str(),
            post(documents::confirm),
        )
        .route(at("/documents/{id}/admit").as_str(), post(documents::admit))
        .route(
            at("/documents/{id}/cancel").as_str(),
            post(documents::cancel),
        )
        .route(
            at("/documents/{id}/delete").as_str(),
            post(documents::delete),
        )
        .route(
            at("/documents/{id}/receipt").as_str(),
            post(documents::receipt),
        )
        .route(
            at("/memories").as_str(),
            get(memories::list).post(memories::push),
        )
        .route(at("/memories/sync").as_str(), get(memories::sync))
        .route(at("/memories/{id}/update").as_str(), post(memories::update))
        .route(at("/memories/{id}/delete").as_str(), post(memories::delete))
        .layer(DefaultBodyLimit::max(16384))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            response_boundary,
        ))
        .with_state(state);
    gated
}

// There is deliberately no cookie authentication, CORS wildcard, remote
// execution endpoint, static mobile UI, or client-selected provider/database.
async fn response_boundary(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let mut response = match state.requests.try_acquire() {
        Ok(_permit) => next.run(request).await,
        Err(_) => ApiError(StatusCode::TOO_MANY_REQUESTS, "busy").into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    response
}

pub struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":{"code":self.1}}))).into_response()
    }
}
impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Invalid => Self(StatusCode::UNAUTHORIZED, "authentication_required"),
            AuthError::InvalidCode => Self(StatusCode::UNAUTHORIZED, "invalid_code"),
            AuthError::InvalidInput => Self(StatusCode::BAD_REQUEST, "invalid_request"),
            AuthError::RateLimited => Self(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            _ => Self(
                StatusCode::SERVICE_UNAVAILABLE,
                "authentication_unavailable",
            ),
        }
    }
}
impl From<StorageError> for ApiError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Authorization(error) => {
                use companion_core::authorization::AuthorizationError::*;
                match error {
                    Denied => Self(StatusCode::FORBIDDEN, "pairing_denied"),
                    RateLimited => Self(StatusCode::TOO_MANY_REQUESTS, "pairing_rate_limited"),
                    Conflict => Self(StatusCode::CONFLICT, "pairing_conflict"),
                    Invalid => Self(StatusCode::BAD_REQUEST, "invalid_request"),
                    Capacity => Self(StatusCode::CONFLICT, "pairing_capacity"),
                }
            }
            StorageError::NotFound => Self(StatusCode::NOT_FOUND, "not_found"),
            StorageError::Memory(error) => match error {
                companion_core::memory::MemoryError::InvalidInput
                | companion_core::memory::MemoryError::UnsupportedKind => {
                    Self(StatusCode::BAD_REQUEST, "invalid_request")
                }
                _ => Self(StatusCode::CONFLICT, "memory_conflict"),
            },
            StorageError::MemorySync(error) => match error {
                companion_core::memory::MemorySyncError::RevisionConflict => {
                    Self(StatusCode::CONFLICT, "memory_revision_conflict")
                }
                companion_core::memory::MemorySyncError::Resurrection => {
                    Self(StatusCode::CONFLICT, "memory_resurrection")
                }
                companion_core::memory::MemorySyncError::Capacity => {
                    Self(StatusCode::CONFLICT, "memory_capacity")
                }
            },
            StorageError::Identity(IdentityError::SessionEnded | IdentityError::InvalidSession) => {
                Self(StatusCode::UNAUTHORIZED, "authentication_required")
            }
            StorageError::Identity(IdentityError::Conflict) => {
                Self(StatusCode::CONFLICT, "conflict")
            }
            StorageError::Identity(IdentityError::Capacity) => {
                Self(StatusCode::CONFLICT, "capacity")
            }
            StorageError::Domain(_) | StorageError::Identity(IdentityError::InvalidRequest) => {
                Self(StatusCode::BAD_REQUEST, "invalid_request")
            }
            _ => Self(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable"),
        }
    }
}

fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    let invalid = || ApiError::from(AuthError::Invalid);
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .ok_or_else(invalid)?
        .to_str()
        .map_err(|_| invalid())?;
    if values.next().is_some() {
        return Err(invalid());
    }
    let (scheme, token) = value.split_once(' ').ok_or_else(invalid)?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
        return Err(invalid());
    }
    Ok(token)
}

fn request_body<T>(input: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    input
        .map(|Json(value)| value)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "invalid_request"))
}

async fn storage<T: Send + 'static>(
    state: &AppState,
    identity: VerifiedIdentity,
    action: impl FnOnce(&AccountStore, &VerifiedIdentity) -> Result<T, StorageError> + Send + 'static,
) -> Result<T, ApiError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || action(&store, &identity))
        .await
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable"))?
        .map_err(ApiError::from)
}

async fn profile(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<companion_core::identity::AccountProfile>, ApiError> {
    let identity = state.auth.verify(bearer(&headers)?).await?;
    Ok(Json(
        storage(&state, identity, |store, identity| store.profile(identity)).await?,
    ))
}
async fn list_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Task>>, ApiError> {
    let identity = state.auth.verify(bearer(&headers)?).await?;
    Ok(Json(
        storage(&state, identity, |store, identity| {
            store.list_tasks(identity)
        })
        .await?,
    ))
}
async fn create_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    input: Result<Json<CreateAccountTask>, JsonRejection>,
) -> Result<Json<Task>, ApiError> {
    let request = request_body(input)?;
    let identity = state.auth.verify(bearer(&headers)?).await?;
    Ok(Json(
        storage(&state, identity, move |store, identity| {
            store.create_task(identity, &request)
        })
        .await?,
    ))
}
async fn get_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Task>, ApiError> {
    let identity = state.auth.verify(bearer(&headers)?).await?;
    Ok(Json(
        storage(&state, identity, move |store, identity| {
            store.get_task(identity, &id)
        })
        .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelRequest {
    revision: u32,
}
async fn cancel_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    input: Result<Json<CancelRequest>, JsonRejection>,
) -> Result<Json<Task>, ApiError> {
    let request = request_body(input)?;
    let identity = state.auth.verify(bearer(&headers)?).await?;
    Ok(Json(
        storage(&state, identity, move |store, identity| {
            store.cancel_task(identity, &id, request.revision)
        })
        .await?,
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignOutRequest {
    all_sessions: bool,
}
async fn sign_out(
    State(state): State<AppState>,
    headers: HeaderMap,
    input: Result<Json<SignOutRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = request_body(input)?;
    let token = bearer(&headers)?;
    let identity = state.auth.verify(token).await?;
    storage(&state, identity, move |store, identity| {
        // A revoked token must not escalate a prior local logout to global logout
        // or terminate newly authenticated sessions by replaying this endpoint.
        store.sign_out(identity, request.all_sessions)
    })
    .await?;
    Ok(Json(
        json!({"revoked":true,"allSessions":request.all_sessions}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeRequest {
    email: String,
}
async fn request_code(
    State(state): State<AppState>,
    input: Result<Json<CodeRequest>, JsonRejection>,
) -> Result<Json<auth::CodeReceipt>, ApiError> {
    let request = request_body(input)?;
    Ok(Json(state.auth.request_code(&request.email).await?))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VerifyCodeRequest {
    challenge_id: String,
    code: String,
    // 43-character base64url of 32 random bytes, generated by the client.
    // Retrying with the same nonce recovers a lost verify response.
    nonce: String,
}
async fn verify_code(
    State(state): State<AppState>,
    input: Result<Json<VerifyCodeRequest>, JsonRejection>,
) -> Result<Json<auth::SessionToken>, ApiError> {
    let request = request_body(input)?;
    Ok(Json(
        state
            .auth
            .verify_code(&request.challenge_id, &request.code, &request.nonce)
            .await?,
    ))
}

#[cfg(test)]
mod tests;

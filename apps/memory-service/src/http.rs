//! Loopback HTTP API: the same archive semantics as the MCP tools, shaped
//! for future remote evolution (versioned paths, sync reads, static error
//! codes). The surface intentionally mirrors the coordinator service.

use crate::{
    errors::{ApiError, MemoryError},
    store::{MemoryKind, MemoryRecord, MemoryStore, NewMemory, RecallFilter},
};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct AppState {
    store: Arc<MemoryStore>,
    requests: Arc<Semaphore>,
}

/// Builds the full router; tests drive this directly without a socket.
/// Phase 2 (LAN) will insert an authentication middleware right before
/// `response_boundary` on the gated sub-router; keep that slot in mind when
/// adding layers here.
pub fn router(state: AppState) -> Router {
    let gated = Router::new()
        .route("/v1/memories", get(list_memories).post(create_memory))
        .route("/v1/memories/{id}", get(get_memory).patch(update_memory))
        .route("/v1/memories/{id}/supersede", post(supersede_memory))
        .route("/v1/memories/{id}/forget", post(forget_memory))
        .route("/v1/stats", get(stats))
        .route("/v1/personality/summary", get(personality))
        .route("/v1/sync", get(sync))
        // 258 KiB covers the 20000-char content cap even when the client
        // \u-escapes every astral character (up to 12 bytes each) plus the
        // JSON envelope.
        .layer(DefaultBodyLimit::max(264_192))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            response_boundary,
        ))
        .with_state(state);
    Router::new().route("/healthz", get(healthz)).merge(gated)
}

async fn response_boundary(
    State(state): State<AppState>,
    request: Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let permit = state.requests.clone().try_acquire_owned();
    let mut response = if permit.is_ok() {
        next.run(request).await
    } else {
        ApiError(StatusCode::TOO_MANY_REQUESTS, "busy").into_response()
    };
    let headers = response.headers_mut();
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn healthz() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

fn request_body<T>(
    body: Result<Json<T>, axum::extract::rejection::JsonRejection>,
) -> Result<T, ApiError> {
    match body {
        Ok(Json(value)) => Ok(value),
        Err(_) => Err(ApiError(StatusCode::BAD_REQUEST, "invalid_request")),
    }
}

/// Bare `Path<i64>` rejections would render axum's plain-text 400, bypassing
/// the error envelope; route them through the same shape.
fn request_path(
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> Result<Path<i64>, ApiError> {
    match path {
        Ok(value) => Ok(value),
        Err(_) => Err(ApiError(StatusCode::BAD_REQUEST, "invalid_request")),
    }
}

fn parse_query_i64(values: &HashMap<String, String>, key: &str) -> Result<Option<i64>, ApiError> {
    match values.get(key) {
        None => Ok(None),
        Some(raw) => raw
            .trim()
            .parse::<i64>()
            .map(Some)
            .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "invalid_request")),
    }
}

async fn storage<T, F>(state: &AppState, work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&MemoryStore) -> Result<T, MemoryError> + Send + 'static,
{
    let store = state.store.clone();
    let joined = tokio::task::spawn_blocking(move || work(&store))
        .await
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable"))?;
    joined.map_err(ApiError::from)
}

fn memory_dto(record: &MemoryRecord) -> Value {
    json!({
        "id": record.id,
        "seq": record.seq,
        "type": record.kind.as_str(),
        "project": record.project,
        "title": record.title,
        "content": record.content,
        "importance": record.importance,
        "createdAt": record.created_at,
        "updatedAt": record.updated_at,
        "validUntil": record.valid_until,
        "supersededBy": record.superseded_by,
        "contradicts": record.contradicts,
        "tags": record.tags,
        "origin": record.origin,
    })
}

async fn list_memories(
    State(state): State<AppState>,
    Query(values): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let limit = parse_query_i64(&values, "limit")?;
    let kind = match values.get("type") {
        None => None,
        Some(raw) => Some(
            MemoryKind::parse(raw).ok_or(ApiError(StatusCode::BAD_REQUEST, "invalid_request"))?,
        ),
    };
    let filter = RecallFilter {
        query: values.get("query").cloned(),
        project: values.get("project").cloned(),
        kind,
        limit,
    };
    let records = storage(&state, move |store| store.recall(&filter)).await?;
    Ok(Json(json!({
        "count": records.len(),
        "memories": records.iter().map(memory_dto).collect::<Vec<_>>()
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CreateMemoryRequest {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    title: String,
    content: String,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    importance: Option<i64>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

async fn create_memory(
    State(state): State<AppState>,
    body: Result<Json<CreateMemoryRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let request = request_body(body)?;
    let kind = match request.kind.as_deref() {
        None => MemoryKind::Fact,
        Some(raw) => {
            MemoryKind::parse(raw).ok_or(ApiError(StatusCode::BAD_REQUEST, "invalid_request"))?
        }
    };
    let draft = NewMemory {
        kind,
        title: request.title.clone(),
        content: request.content.clone(),
        project: request.project.clone(),
        importance: request.importance.unwrap_or(3),
        tags: request.tags.clone().unwrap_or_default(),
    };
    let title = request.title;
    let id = storage(&state, move |store| store.remember(&draft)).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "stored": true, "title": title})),
    ))
}

async fn get_memory(
    State(state): State<AppState>,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> Result<Json<Value>, ApiError> {
    let Path(id) = request_path(path)?;
    let (memory, chain) = storage(&state, move |store| {
        store.get_with_chain(id)?.ok_or(MemoryError::NotFound)
    })
    .await?;
    Ok(Json(json!({
        "memory": memory_dto(&memory),
        "chain": chain.iter().map(memory_dto).collect::<Vec<_>>()
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UpdateMemoryRequest {
    content: String,
}

async fn update_memory(
    State(state): State<AppState>,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
    body: Result<Json<UpdateMemoryRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Path(id) = request_path(path)?;
    let request = request_body(body)?;
    let content = request.content;
    storage(&state, move |store| store.update(id, &content)).await?;
    Ok(Json(json!({"id": id, "updated": true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SupersedeMemoryRequest {
    title: String,
    content: String,
}

async fn supersede_memory(
    State(state): State<AppState>,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
    body: Result<Json<SupersedeMemoryRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Path(id) = request_path(path)?;
    let request = request_body(body)?;
    let (title, content) = (request.title.clone(), request.content.clone());
    let (old_id, new_id) =
        storage(&state, move |store| store.supersede(id, &title, &content)).await?;
    // The supersede is already committed; a failed cosmetic title fetch must
    // not turn the response into a 5xx that invites a retry.
    let old_title = storage(&state, move |store| Ok(store.get(old_id)?.map(|r| r.title)))
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok(Json(json!({
        "oldId": old_id, "newId": new_id,
        "oldTitle": old_title, "newTitle": request.title
    })))
}

async fn forget_memory(
    State(state): State<AppState>,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> Result<Json<Value>, ApiError> {
    let Path(id) = request_path(path)?;
    storage(&state, move |store| store.forget(id)).await?;
    Ok(Json(json!({"id": id, "forgotten": true})))
}

async fn stats(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let stats = storage(&state, move |store| store.stats()).await?;
    Ok(Json(json!({
        "total": stats.total,
        "active": stats.active,
        "superseded": stats.superseded,
        "byType": stats.by_type
            .into_iter()
            .map(|(k, v)| (k, json!(v)))
            .collect::<serde_json::Map<String, Value>>(),
        "byProject": stats.by_project
            .into_iter()
            .map(|(k, v)| (k, json!(v)))
            .collect::<serde_json::Map<String, Value>>(),
    })))
}

async fn personality(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let sections = storage(&state, move |store| store.personality_summary()).await?;
    Ok(Json(json!({
        "sections": sections
            .into_iter()
            .map(|(kind, entries)| json!({
                "kind": kind,
                "entries": entries.iter().map(memory_dto).collect::<Vec<_>>()
            }))
            .collect::<Vec<_>>()
    })))
}

async fn sync(
    State(state): State<AppState>,
    Query(values): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let since = parse_query_i64(&values, "since")?.unwrap_or(0);
    let limit = parse_query_i64(&values, "limit")?.unwrap_or(200);
    let (records, current_seq) =
        storage(&state, move |store| store.changes_since(since, limit)).await?;
    Ok(Json(json!({
        "memories": records.iter().map(memory_dto).collect::<Vec<_>>(),
        "currentSeq": current_seq
    })))
}

/// Binds `addr` and serves until Ctrl-C. Loopback only by default; anything
/// wider belongs to the Phase 2 LAN plan together with authentication.
pub async fn serve(addr: std::net::SocketAddr, store: Arc<MemoryStore>) -> Result<(), String> {
    let state = AppState {
        store,
        requests: Arc::new(Semaphore::new(16)),
    };
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| format!("cannot bind {addr}: {error}"))?;
    println!("personal memory service listening on http://{addr} (loopback only)");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async { tokio::signal::ctrl_c().await.expect("ctrl_c") })
        .await
        .map_err(|error| format!("server error: {error}"))
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;

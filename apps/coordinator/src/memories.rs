//! Memory sync routes. Auth is account-level (like /v1/tasks): the pairing
//! chain authorizes a phone to command a desktop's side effects, while these
//! routes manage the account's own mirrored data — a separate authorization
//! axis by design (docs/architecture/device-continuity.md).
use super::*;
use axum::extract::{rejection::QueryRejection, Query};
use companion_core::memory::MemoryKind;
use companion_storage::accounts::{
    MemorySnapshotItem, MemorySyncResponse, MemoryWrite, MemoryWriteReceipt,
};

pub(super) async fn list(
    State(s): State<AppState>,
    h: HeaderMap,
) -> Result<Json<Vec<MemorySnapshotItem>>, ApiError> {
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(storage(&s, i, |s, i| s.list_memories(i)).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SyncQuery {
    cursor: i64,
    generation: i64,
}

/// Desktop pull: ordered events since the caller's cursor, degrading to a
/// full snapshot when the caller fell behind the retention window.
pub(super) async fn sync(
    State(s): State<AppState>,
    h: HeaderMap,
    query: Result<Query<SyncQuery>, QueryRejection>,
) -> Result<Json<MemorySyncResponse>, ApiError> {
    let Query(q) = query.map_err(|_| ApiError(StatusCode::BAD_REQUEST, "invalid_request"))?;
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| {
            s.pull_memories(i, q.generation, q.cursor)
        })
        .await?,
    ))
}

/// One flat write shape shared by push and the peer routes; `op` selects the
/// variant and the unused fields must be absent. Kept manual (not a tagged
/// enum) because internally-tagged enums cannot enforce deny_unknown_fields.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MemoryWriteRequest {
    op: String,
    base_revision: i64,
    kind: Option<MemoryKind>,
    body: Option<String>,
    event_date: Option<String>,
}

impl MemoryWriteRequest {
    fn into_write(self) -> Result<MemoryWrite, ApiError> {
        let bad = || ApiError(StatusCode::BAD_REQUEST, "invalid_request");
        match self.op.as_str() {
            "create" if self.base_revision == 0 && self.kind.is_some() && self.body.is_some() => {
                Ok(MemoryWrite::Create {
                    kind: self.kind.unwrap(),
                    body: self.body.unwrap(),
                    event_date: self.event_date,
                })
            }
            "update" if self.kind.is_some() && self.body.is_some() => Ok(MemoryWrite::Update {
                base_revision: self.base_revision,
                kind: self.kind.unwrap(),
                body: self.body.unwrap(),
                event_date: self.event_date,
            }),
            // A delete carrying kind/body/eventDate is a reused template, not
            // a delete: drop nothing silently, reject the shape.
            "delete"
                if self.kind.is_none()
                    && self.body.is_none()
                    && self.event_date.is_none() =>
            {
                Ok(MemoryWrite::Delete {
                    base_revision: self.base_revision,
                })
            }
            _ => Err(bad()),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MemoryPushRequest {
    request_id: String,
    id: String,
    op: String,
    base_revision: i64,
    kind: Option<MemoryKind>,
    body: Option<String>,
    event_date: Option<String>,
}

/// Desktop outbox push; idempotent by request id.
pub(super) async fn push(
    State(s): State<AppState>,
    h: HeaderMap,
    b: Result<Json<MemoryPushRequest>, JsonRejection>,
) -> Result<Json<MemoryWriteReceipt>, ApiError> {
    let request = request_body(b)?;
    let write = MemoryWriteRequest {
        op: request.op,
        base_revision: request.base_revision,
        kind: request.kind,
        body: request.body,
        event_date: request.event_date,
    }
    .into_write()?;
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| {
            s.write_memory(i, &request.request_id, &request.id, &write)
        })
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MemoryPeerUpdate {
    revision: i64,
    request_id: String,
    kind: MemoryKind,
    body: String,
    event_date: Option<String>,
}

pub(super) async fn update(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    b: Result<Json<MemoryPeerUpdate>, JsonRejection>,
) -> Result<Json<MemoryWriteReceipt>, ApiError> {
    let request = request_body(b)?;
    let write = MemoryWrite::Update {
        base_revision: request.revision,
        kind: request.kind,
        body: request.body,
        event_date: request.event_date,
    };
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| {
            s.write_memory(i, &request.request_id, &id, &write)
        })
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MemoryPeerDelete {
    revision: i64,
    request_id: String,
}

pub(super) async fn delete(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    b: Result<Json<MemoryPeerDelete>, JsonRejection>,
) -> Result<Json<MemoryWriteReceipt>, ApiError> {
    let request = request_body(b)?;
    let write = MemoryWrite::Delete {
        base_revision: request.revision,
    };
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| {
            s.write_memory(i, &request.request_id, &id, &write)
        })
        .await?,
    ))
}

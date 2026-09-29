use super::*;
use companion_core::authorization::{ActionBinding, DocumentAction, ShareDocument};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Receipt {
    binding: ActionBinding,
    state: String,
    artifact_hash: String,
}
pub(super) async fn list(
    State(s): State<AppState>,
    h: HeaderMap,
) -> Result<Json<Vec<DocumentAction>>, ApiError> {
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(storage(&s, i, |s, i| s.list_documents(i)).await?))
}
pub(super) async fn share(
    State(s): State<AppState>,
    h: HeaderMap,
    b: Result<Json<ShareDocument>, JsonRejection>,
) -> Result<Json<DocumentAction>, ApiError> {
    let b = request_body(b)?;
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| s.share_document(i, &b)).await?,
    ))
}
pub(super) async fn get_one(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<DocumentAction>, ApiError> {
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| s.document_action(i, &id)).await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Empty {}
pub(super) async fn cancel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    b: Result<Json<Empty>, JsonRejection>,
) -> Result<Json<DocumentAction>, ApiError> {
    request_body(b)?;
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| s.cancel_document(i, &id)).await?,
    ))
}
/// Terminal records only; either participant. Returns the pre-delete snapshot.
pub(super) async fn delete(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    b: Result<Json<Empty>, JsonRejection>,
) -> Result<Json<DocumentAction>, ApiError> {
    request_body(b)?;
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| s.delete_document(i, &id)).await?,
    ))
}
pub(super) async fn confirm(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    b: Result<Json<ActionBinding>, JsonRejection>,
) -> Result<Json<DocumentAction>, ApiError> {
    advance(s, h, id, request_body(b)?, false).await
}
pub(super) async fn admit(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    b: Result<Json<ActionBinding>, JsonRejection>,
) -> Result<Json<DocumentAction>, ApiError> {
    advance(s, h, id, request_body(b)?, true).await
}
async fn advance(
    s: AppState,
    h: HeaderMap,
    id: String,
    b: ActionBinding,
    admit: bool,
) -> Result<Json<DocumentAction>, ApiError> {
    if id != b.action_id {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid_request"));
    }
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| {
            s.document_action(i, &id)?;
            if admit {
                s.admit_action(i, &b)?;
            } else {
                s.confirm_action(i, &b)?;
            }
            s.document_action(i, &id)
        })
        .await?,
    ))
}
pub(super) async fn receipt(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    b: Result<Json<Receipt>, JsonRejection>,
) -> Result<Json<DocumentAction>, ApiError> {
    let b = request_body(b)?;
    if id != b.binding.action_id {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid_request"));
    }
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| {
            s.receipt_document(i, &b.binding, &b.state, &b.artifact_hash)
        })
        .await?,
    ))
}

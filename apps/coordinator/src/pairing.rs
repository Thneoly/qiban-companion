//! Pairing transport only. Action admission is not exposed before an executor exists.
use super::*;
use companion_core::authorization::{Pairing, PairingOffer};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Offer {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Preview {
    code: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Accept {
    code: String,
    pairing_id: String,
    name: String,
}
pub(super) async fn list(
    State(s): State<AppState>,
    h: HeaderMap,
) -> Result<Json<Vec<Pairing>>, ApiError> {
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(storage(&s, i, |s, i| s.list_pairings(i)).await?))
}
pub(super) async fn offer(
    State(s): State<AppState>,
    h: HeaderMap,
    b: Result<Json<Offer>, JsonRejection>,
) -> Result<Json<PairingOffer>, ApiError> {
    let b = request_body(b)?;
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| s.offer_pairing(i, &b.name)).await?,
    ))
}
pub(super) async fn preview(
    State(s): State<AppState>,
    h: HeaderMap,
    b: Result<Json<Preview>, JsonRejection>,
) -> Result<Json<Pairing>, ApiError> {
    let b = request_body(b)?;
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| s.preview_pairing(i, &b.code)).await?,
    ))
}
pub(super) async fn accept(
    State(s): State<AppState>,
    h: HeaderMap,
    b: Result<Json<Accept>, JsonRejection>,
) -> Result<Json<Pairing>, ApiError> {
    let b = request_body(b)?;
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| {
            s.accept_pairing(i, &b.code, &b.pairing_id, &b.name)
        })
        .await?,
    ))
}
pub(super) async fn revoke(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    b: Result<Json<CancelRequest>, JsonRejection>,
) -> Result<Json<Pairing>, ApiError> {
    let b = request_body(b)?;
    let i = s.auth.verify(bearer(&h)?).await?;
    Ok(Json(
        storage(&s, i, move |s, i| s.revoke_pairing(i, &id, b.revision)).await?,
    ))
}

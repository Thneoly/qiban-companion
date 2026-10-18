//! Pairing transport only. Action admission is not exposed before an executor exists.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use companion_core::authorization::{Pairing, PairingOffer};
use sha2::Digest as _;
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

/// The embedded admission (P1-b PR-3): a controllerless phone trades a
/// pairing code for a full account session — the single transaction lives
/// in AccountStore::exchange_pairing_code (guarded budget, code
/// consumption, controller registration, session mint, idempotent by
/// nonce-derived hash). Mounted ONLY on the embedded /api composition by
/// the LAN shell; the external multi-account router never exposes it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Exchange {
    code: String,
    name: String,
    nonce: String,
}
pub(super) async fn exchange(
    State(s): State<AppState>,
    b: Result<Json<Exchange>, JsonRejection>,
) -> Result<Json<auth::SessionToken>, ApiError> {
    let b = request_body(b)?;
    let secret = s.auth.derive_session_secret(&b.nonce)?;
    let token_hash: [u8; 32] = sha2::Sha256::digest(secret.as_ref()).into();
    let store = s.store.clone();
    let (code, name) = (b.code, b.name);
    let (_, login) =
        tokio::task::spawn_blocking(move || store.exchange_pairing_code(&code, &name, &token_hash))
            .await
            .map_err(|_| AuthError::Unavailable)?
            .map_err(ApiError::from)?;
    Ok(Json(auth::SessionToken {
        access_token: format!("qbs_{}", URL_SAFE_NO_PAD.encode(secret.as_ref())),
        session_id: login.session_id,
        expires_at: login.expires_at,
        idle_timeout_seconds: 1800,
    }))
}

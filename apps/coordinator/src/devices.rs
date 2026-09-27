//! Device presence transport: a display-only lease refresh. Never gates the
//! confirm→admit authorization chain and never extends any expiry.
use super::*;
use companion_core::authorization::ACTION_SCOPES;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Heartbeat {
    capabilities: Vec<String>,
}
pub(super) async fn heartbeat(
    State(s): State<AppState>,
    h: HeaderMap,
    b: Result<Json<Heartbeat>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let b = request_body(b)?;
    // Raw list is capped first (simple wire contract), then every slug must
    // match a known ActionScope; the store keeps the canonical deduped set.
    if b.capabilities.len() > 8 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid_request"));
    }
    let mut scopes = Vec::new();
    for slug in &b.capabilities {
        let scope = ACTION_SCOPES
            .iter()
            .find(|scope| scope.slug() == slug.as_str())
            .ok_or(ApiError(StatusCode::BAD_REQUEST, "invalid_request"))?;
        if !scopes.contains(scope) {
            scopes.push(*scope);
        }
    }
    let i = s.auth.verify(bearer(&h)?).await?;
    let updated = storage(&s, i, move |s, i| s.heartbeat(i, &scopes)).await?;
    Ok(Json(serde_json::json!({ "updated": updated })))
}

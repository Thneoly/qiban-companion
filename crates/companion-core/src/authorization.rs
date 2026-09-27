//! Pairing is separate from permission to perform a concrete side effect.
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum AuthorizationError {
    #[error("配对信息已失效，或当前会话无权进行此操作")]
    Denied,
    #[error("配对或动作状态已变化，请重新核对")]
    Conflict,
    #[error("配对或动作参数无效")]
    Invalid,
    #[error("配对或动作记录已达上限")]
    Capacity,
    #[error("配对码核对失败次数过多，请五分钟后重试")]
    RateLimited,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionScope {
    DocumentExcerpt,
}
/// Every scope the desktop executor currently supports. The single source of
/// truth for heartbeat capability slugs: the desktop reports from it, the
/// coordinator validates against it, the phone maps labels over the same
/// slugs.
pub const ACTION_SCOPES: [ActionScope; 1] = [ActionScope::DocumentExcerpt];
impl ActionScope {
    pub fn slug(&self) -> &'static str {
        match self {
            ActionScope::DocumentExcerpt => "document_excerpt",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub id: String,
    pub desktop_id: String,
    pub desktop_name: String,
    pub controller_id: Option<String>,
    pub controller_name: Option<String>,
    pub scope: ActionScope,
    pub revision: u32,
    pub status: String,
    pub expires_at: u64,
    pub current_role: String,
    /// Presence lease, display-only: never gates confirm→admit. Defaults
    /// keep old payloads (and old coordinators) decodable.
    #[serde(default)]
    pub desktop_online: bool,
    #[serde(default)]
    pub desktop_last_heartbeat_at: Option<u64>,
    #[serde(default)]
    pub desktop_capabilities: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOffer {
    pub pairing: Pairing,
    pub code: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionBinding {
    pub action_id: String,
    pub resource_id: String,
    pub resource_version: u32,
    pub parameters_digest: String,
    pub pair_revision: u32,
    pub scope: ActionScope,
}
impl ActionBinding {
    pub fn validate(&self) -> Result<(), AuthorizationError> {
        if uuid::Uuid::parse_str(&self.action_id).is_err()
            || uuid::Uuid::parse_str(&self.resource_id).is_err()
            || self.parameters_digest.len() != 64
            || !self
                .parameters_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.resource_version == 0
            || self.pair_revision == 0
        {
            return Err(AuthorizationError::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionAuthorization {
    pub pairing_id: String,
    pub binding: ActionBinding,
    pub expires_at: u64,
    pub state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShareDocument {
    pub pairing_id: String,
    pub binding: ActionBinding,
    pub source_name: String,
    pub preview: String,
}
impl ShareDocument {
    pub fn validate(&self) -> Result<(), AuthorizationError> {
        self.binding.validate()?;
        crate::execution::prepare_document(
            &self.binding.action_id,
            &self.source_name,
            &self.preview,
        )
        .map_err(|_| AuthorizationError::Invalid)?;
        if self.preview.len() > 12 * 1024
            || self.binding.resource_version != 1
            || self.binding.parameters_digest != document_digest(&self.source_name, &self.preview)
        {
            return Err(AuthorizationError::Invalid);
        }
        Ok(())
    }
}
pub fn document_digest(name: &str, preview: &str) -> String {
    crate::execution::digest(format!("remote-excerpt-v1\0{name}\0{preview}").as_bytes())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAction {
    pub authorization: ActionAuthorization,
    pub source_name: String,
    pub preview: String,
    pub artifact_hash: String,
    pub current_role: String,
}

#[cfg(test)]
mod presence_tests {
    use super::*;
    #[test]
    fn legacy_pairing_payload_without_presence_fields_still_decodes() {
        // Mixed-version evidence: an older coordinator's payload (no
        // presence keys) must decode with the defaults, not fail.
        let legacy = serde_json::json!({
            "id":"00000000-0000-4000-8000-000000000001",
            "desktopId":"00000000-0000-4000-8000-000000000002",
            "desktopName":"旧电脑",
            "controllerId":null,
            "controllerName":null,
            "scope":"document_excerpt",
            "revision":1,
            "status":"pending",
            "expiresAt":1000,
            "currentRole":"desktop"
        });
        let p: Pairing = serde_json::from_value(legacy).unwrap();
        assert!(!p.desktop_online);
        assert_eq!(p.desktop_last_heartbeat_at, None);
        assert!(p.desktop_capabilities.is_empty());
    }
}

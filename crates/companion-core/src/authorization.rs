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
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionScope {
    DocumentExcerpt,
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

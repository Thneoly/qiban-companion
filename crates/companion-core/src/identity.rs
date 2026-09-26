//! Identity boundary for trusted authentication adapters.
//! No token parsing or network authentication takes place in this module.
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("身份或会话无效，请重新登录")]
    InvalidSession,
    #[error("会话已过期或已退出，请重新登录")]
    SessionEnded,
    #[error("请求标识无效")]
    InvalidRequest,
    #[error("请求内容或任务版本已变化")]
    Conflict,
    #[error("当前账号的待办已达 100 条")]
    Capacity,
    #[error("验证码无效或已失效")]
    InvalidCode,
    #[error("请求过于频繁，请稍后重试")]
    RateLimited,
}

/// Construct ONLY after a trusted authenticator validates the token and current
/// session state. External token adapters must also validate signature or
/// introspection, issuer and audience. Never deserialize client claims into this
/// type. `authenticated_at` is the original login time, NOT token refresh time.
/// `session_id` must remain stable across refreshes and change on a new login.
/// No access/refresh tokens or email addresses are retained here.
pub struct VerifiedIdentity {
    issuer: String,
    subject: String,
    session_id: String,
    authenticated_at: u64,
    expires_at: u64,
}

impl VerifiedIdentity {
    pub fn from_verified_provider(
        issuer: &str,
        subject: &str,
        session_id: &str,
        authenticated_at: u64,
        expires_at: u64,
    ) -> Result<Self, IdentityError> {
        for value in [issuer, subject, session_id] {
            if value.is_empty()
                || value.len() > 512
                || value.trim() != value
                || value.chars().any(char::is_control)
            {
                return Err(IdentityError::InvalidSession);
            }
        }
        if authenticated_at == 0 || expires_at <= authenticated_at || expires_at > i64::MAX as u64 {
            return Err(IdentityError::InvalidSession);
        }
        Ok(Self {
            issuer: issuer.into(),
            subject: subject.into(),
            session_id: session_id.into(),
            authenticated_at,
            expires_at,
        })
    }

    pub fn validate_at(&self, now: u64) -> Result<(), IdentityError> {
        if self.authenticated_at > now || now >= self.expires_at {
            return Err(IdentityError::SessionEnded);
        }
        Ok(())
    }
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
    pub fn subject(&self) -> &str {
        &self.subject
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn authenticated_at(&self) -> u64 {
        self.authenticated_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    pub account_id: String,
    pub companion_id: String,
}

/// A future transport must decode this strict request, not an owner-bearing DTO.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAccountTask {
    pub request_id: String,
    pub title: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_bounds_and_exact_expiration_fail_closed() {
        let identity =
            VerifiedIdentity::from_verified_provider("issuer", "subject", "session", 10, 20)
                .unwrap();
        assert!(identity.validate_at(9).is_err());
        assert!(identity.validate_at(10).is_ok());
        assert!(identity.validate_at(19).is_ok());
        assert!(identity.validate_at(20).is_err());
        for invalid in ["", "  user", "user\n"] {
            assert!(
                VerifiedIdentity::from_verified_provider("issuer", invalid, "session", 10, 20)
                    .is_err()
            );
        }
        assert!(
            VerifiedIdentity::from_verified_provider("issuer", "subject", "session", 20, 20)
                .is_err()
        );
    }
    #[test]
    fn request_cannot_supply_an_owner() {
        assert!(serde_json::from_str::<CreateAccountTask>(
            r#"{"requestId":"r","title":"t","accountId":"victim"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<CreateAccountTask>(
            r#"{"requestId":"r","title":"t","user_id":"victim"}"#
        )
        .is_err());
        assert!(
            serde_json::from_str::<CreateAccountTask>(r#"{"requestId":"r","title":"t"}"#).is_ok()
        );
    }
}

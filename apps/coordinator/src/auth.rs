//! Self-hosted email OTP and opaque sessions. No third-party identity provider.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use companion_core::identity::{IdentityError, VerifiedIdentity};
use companion_storage::{accounts::AccountStore, StorageError};
use hmac::{Hmac, Mac};
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, future::Future, pin::Pin, sync::Arc, time::Duration};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    Invalid,
    InvalidCode,
    InvalidInput,
    RateLimited,
    Unavailable,
    Configuration,
}
impl From<StorageError> for AuthError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Identity(IdentityError::InvalidCode) => Self::InvalidCode,
            StorageError::Identity(IdentityError::RateLimited) => Self::RateLimited,
            StorageError::Identity(IdentityError::InvalidSession | IdentityError::SessionEnded) => {
                Self::Invalid
            }
            _ => Self::Unavailable,
        }
    }
}

pub type MailResult<'a> = Pin<Box<dyn Future<Output = Result<(), ()>> + Send + 'a>>;
pub trait CodeMailer: Send + Sync {
    fn send<'a>(&'a self, email: &'a str, code: &'a str) -> MailResult<'a>;
}
pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}
impl SmtpMailer {
    pub fn new(
        host: &str,
        username: String,
        password: String,
        from: &str,
        starttls: bool,
    ) -> Result<Self, AuthError> {
        let builder = if starttls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::relay(host)
        }
        .map_err(|_| AuthError::Configuration)?;
        let transport = builder
            .credentials(Credentials::new(username, password))
            .timeout(Some(Duration::from_secs(8)))
            .build();
        Ok(Self {
            transport,
            from: from.parse().map_err(|_| AuthError::Configuration)?,
        })
    }
}
impl CodeMailer for SmtpMailer {
    fn send<'a>(&'a self, email: &'a str, code: &'a str) -> MailResult<'a> {
        Box::pin(async move {
            let message = Message::builder().from(self.from.clone()).to(email.parse().map_err(|_| ())?)
                .subject("栖伴登录验证码").body(format!("你的栖伴登录验证码为：{code}。10分钟内有效。请勿向他人提供验证码。如非本人操作，请忽略本邮件。")).map_err(|_| ())?;
            self.transport
                .send(message)
                .await
                .map(|_| ())
                .map_err(|_| ())
        })
    }
}

pub struct NativeAuth {
    store: Arc<AccountStore>,
    mailer: Arc<dyn CodeMailer>,
    pepper: Zeroizing<[u8; 32]>,
    allowed: HashSet<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeReceipt {
    pub challenge_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionToken {
    pub access_token: String,
    pub session_id: String,
    pub expires_at: u64,
    pub idle_timeout_seconds: u32,
}
impl NativeAuth {
    pub fn new(
        store: Arc<AccountStore>,
        mailer: Arc<dyn CodeMailer>,
        pepper: [u8; 32],
        allowed: &[String],
    ) -> Result<Self, AuthError> {
        if pepper == [0; 32] || allowed.is_empty() || allowed.len() > 100 {
            return Err(AuthError::Configuration);
        }
        let allowed: Result<HashSet<_>, _> =
            allowed.iter().map(|email| canonical_email(email)).collect();
        Ok(Self {
            store,
            mailer,
            pepper: Zeroizing::new(pepper),
            allowed: allowed.map_err(|_| AuthError::Configuration)?,
        })
    }
    async fn storage<T: Send + 'static>(
        &self,
        action: impl FnOnce(&AccountStore) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<T, AuthError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || action(&store))
            .await
            .map_err(|_| AuthError::Unavailable)?
            .map_err(Into::into)
    }
    fn code_mac(&self, id: &str, email: &str, code: &str) -> [u8; 32] {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.pepper.as_ref()).expect("HMAC accepts 32-byte key");
        for value in ["qiban-login-code-v1", id, email, code] {
            mac.update(&(value.len() as u64).to_be_bytes());
            mac.update(value.as_bytes());
        }
        mac.finalize().into_bytes().into()
    }
    pub async fn request_code(&self, email: &str) -> Result<CodeReceipt, AuthError> {
        let email = canonical_email(email)?;
        let challenge_id = uuid::Uuid::new_v4().to_string();
        // Closed invitation: identical response shape, no account-existence flag.
        if !self.allowed.contains(&email) {
            return Ok(CodeReceipt { challenge_id });
        }
        let code = Zeroizing::new(random_code()?);
        let mac = self.code_mac(&challenge_id, &email, &code);
        let code_id = challenge_id.clone();
        let address = email.clone();
        self.storage(move |store| store.reserve_login_code(&address, &code_id, &mac))
            .await?;
        let delivered = matches!(
            tokio::time::timeout(Duration::from_secs(10), self.mailer.send(&email, &code)).await,
            Ok(Ok(()))
        );
        let code_id = challenge_id.clone();
        self.storage(move |store| store.finish_code_delivery(&code_id, delivered))
            .await?;
        if !delivered {
            return Err(AuthError::Unavailable);
        }
        Ok(CodeReceipt { challenge_id })
    }
    pub async fn verify_code(&self, id: &str, code: &str) -> Result<SessionToken, AuthError> {
        if uuid::Uuid::parse_str(id).is_err()
            || code.len() != 8
            || !code.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(AuthError::InvalidCode);
        }
        let code_id = id.to_string();
        let email = self
            .storage(move |store| store.login_code_email(&code_id))
            .await?;
        if !self.allowed.contains(&email) {
            return Err(AuthError::InvalidCode);
        }
        let candidate = self.code_mac(id, &email, code);
        let mut random = Zeroizing::new([0u8; 32]);
        getrandom::fill(random.as_mut()).map_err(|_| AuthError::Unavailable)?;
        let token = format!("qbs_{}", URL_SAFE_NO_PAD.encode(random.as_ref()));
        let hash: [u8; 32] = Sha256::digest(random.as_ref()).into();
        let code_id = id.to_string();
        let login = self
            .storage(move |store| store.redeem_login_code(&code_id, &candidate, &hash))
            .await?;
        Ok(SessionToken {
            access_token: token,
            session_id: login.session_id,
            expires_at: login.expires_at,
            idle_timeout_seconds: 1800,
        })
    }
    pub async fn verify(&self, token: &str) -> Result<VerifiedIdentity, AuthError> {
        if token.len() != 47 || !token.starts_with("qbs_") {
            return Err(AuthError::Invalid);
        }
        let random = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(&token[4..])
                .map_err(|_| AuthError::Invalid)?,
        );
        if random.len() != 32 {
            return Err(AuthError::Invalid);
        }
        let hash: [u8; 32] = Sha256::digest(random.as_slice()).into();
        self.storage(move |store| store.authenticate_native_token(&hash))
            .await
    }
}

fn canonical_email(input: &str) -> Result<String, AuthError> {
    let email = input.trim().to_ascii_lowercase();
    if !email.is_ascii()
        || email.len() > 254
        || email.chars().any(char::is_whitespace)
        || email.parse::<lettre::Address>().is_err()
    {
        return Err(AuthError::InvalidInput);
    }
    Ok(email)
}
fn random_code() -> Result<String, AuthError> {
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes).map_err(|_| AuthError::Unavailable)?;
        let value = u32::from_le_bytes(bytes);
        if value < 4_200_000_000 {
            return Ok(format!("{:08}", value % 100_000_000));
        }
    }
}

//! Embedded LAN coordinator (P1-b): the desktop process hosts the
//! coordinator router itself, so trial phones connect over the LAN with no
//! external deployment. Design: docs/architecture/lan-phone-continuity.md.
//!
//! PR-1 scope: process-local composition (router without OTP routes),
//! port pinning with dev-mode detection, local-account minting, and the
//! desktop's boot-time self-healing re-mint. The browser-facing defense
//! layer (Host allowlist, /api adaptation, static assets) lands in PR-2 —
//! until then the listener below stays loopback-only.
use base64::Engine as _;
use companion_coordinator::auth::{CodeMailer, MailResult, NativeAuth};
use companion_coordinator::{router_embedded, AppState};
use companion_storage::accounts::AccountStore;
use sha2::{Digest, Sha256};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

/// A mailer that never delivers. The embedded composition does not mount
/// the OTP routes at all; this stub keeps NativeAuth's constructor honest
/// and the closed-invitation allowlist acts as the second backstop.
struct UnavailableMailer;
impl CodeMailer for UnavailableMailer {
    fn send<'a>(&'a self, _email: &'a str, _code: &'a str) -> MailResult<'a> {
        Box::pin(async { Err(()) })
    }
}

pub enum EmbeddedCoordinator {
    /// The embedded server owns this port. `store` is shared with the local
    /// mint path — same process, same connection, no second writer.
    Started {
        port: u16,
        store: Arc<AccountStore>,
        shutdown: Arc<AtomicBool>,
    },
    /// Something that answers like a coordinator already owns the preferred
    /// port — the dev workflow (`npm run coordinator` first, desktop after).
    /// The desktop client connects to it externally; nothing was started.
    External { port: u16 },
}

/// Managed Tauri state carrying the embedded server's shutdown flag so the
/// RunEvent::Exit handler can reach it without a static. Correctness never
/// depends on the graceful window (SQLite short transactions ride the
/// rollback journal, same posture as the three existing background loops).
pub struct ShutdownHandle(pub Arc<AtomicBool>);

/// Boot the embedded coordinator on `preferred`, or a nearby port when it is
/// taken by something that is NOT a coordinator. When a coordinator-looking
/// service already listens (dev workflow), return External and start nothing.
pub async fn start(db_path: &Path, preferred: u16) -> Result<EmbeddedCoordinator, String> {
    if coordinator_like(preferred).await {
        return Ok(EmbeddedCoordinator::External { port: preferred });
    }
    // Loopback only until the defense layer lands (PR-2); the LAN listener
    // switch is a one-line address change gated on that PR.
    for port in candidate_ports(preferred) {
        let listener =
            match tokio::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await
            {
                Ok(listener) => listener,
                Err(_) => continue,
            };
        let store = Arc::new(
            AccountStore::open(db_path).map_err(|error| format!("内嵌协调库打开失败：{error}"))?,
        );
        let mut pepper = Zeroizing::new([0u8; 32]);
        getrandom::fill(pepper.as_mut()).map_err(|_| "无法生成内嵌服务密钥材料")?;
        // The pepper is per-boot random and never persisted: the embedded
        // form has no OTP path, and token verification never touches it.
        let auth = NativeAuth::new(
            store.clone(),
            Arc::new(UnavailableMailer),
            *pepper,
            &["local@invalid".into()],
        )
        .map_err(|_| "内嵌协调服务配置无效")?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = shutdown.clone();
        let served = store.clone();
        tauri::async_runtime::spawn(async move {
            let _ = axum::serve(listener, router_embedded(AppState::new(auth, served)))
                .with_graceful_shutdown(async move {
                    while !flag.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                })
                .await;
        });
        return Ok(EmbeddedCoordinator::Started {
            port,
            store,
            shutdown,
        });
    }
    Err(format!(
        "端口 {preferred} 起的候选端口都被占用，内嵌服务未能启动"
    ))
}

fn candidate_ports(preferred: u16) -> Vec<u16> {
    (preferred..preferred.saturating_add(20))
        .filter(|port| (1024..=65535).contains(port))
        .collect()
}

/// Does the port answer like OUR coordinator? Only /healthz with the exact
/// known body counts — a random other service returning 200 does not.
async fn coordinator_like(port: u16) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    let Ok(response) = client
        .get(format!("http://127.0.0.1:{port}/healthz"))
        .send()
        .await
    else {
        return false;
    };
    response.status().is_success()
        && matches!(
            response.json::<serde_json::Value>().await,
            Ok(value) if value["status"] == "ok"
        )
}

/// Mint a fresh local-account session token (the `qbs_` + SHA256 rule the
/// verifier uses), retiring the vault's previous token in the same
/// transaction. Returns the token STRING for the vault; only its digest is
/// ever handed to the store.
pub fn mint_local_token(
    store: &AccountStore,
    previous_token: Option<&str>,
) -> Result<String, String> {
    let subject = store
        .local_account_subject()
        .map_err(|error| format!("无法读取本地账号：{error}"))?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut random = Zeroizing::new([0u8; 32]);
    getrandom::fill(random.as_mut()).map_err(|_| "无法生成会话密钥材料")?;
    let hash: [u8; 32] = Sha256::digest(random.as_ref()).into();
    let retire = previous_token.and_then(token_hash);
    store
        .mint_local_session(&subject, &hash, retire.as_ref())
        .map_err(|error| format!("无法建立本地会话：{error}"))?;
    Ok(format!(
        "qbs_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random.as_ref())
    ))
}

fn token_hash(token: &str) -> Option<[u8; 32]> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&token[4..])
        .ok()?;
    (raw.len() == 32).then(|| Sha256::digest(&raw).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minted_tokens_verify_through_the_normal_channel() {
        // A real file store with the real clock: mint → authenticate back to
        // back, well inside the 30-minute idle window.
        let path = std::env::temp_dir().join(format!("embedded-mint-{}.db", uuid::Uuid::new_v4()));
        let store = AccountStore::open(&path).unwrap();
        let first = mint_local_token(&store, None).unwrap();
        assert!(store
            .authenticate_native_token(&token_hash(&first).unwrap())
            .is_ok());
        // Re-mint retires the previous token atomically.
        let second = mint_local_token(&store, Some(&first)).unwrap();
        assert_ne!(first, second);
        assert!(store
            .authenticate_native_token(&token_hash(&first).unwrap())
            .is_err());
        assert!(store
            .authenticate_native_token(&token_hash(&second).unwrap())
            .is_ok());
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}

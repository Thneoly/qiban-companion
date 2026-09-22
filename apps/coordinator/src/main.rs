use base64::{engine::general_purpose::STANDARD, Engine};
use companion_coordinator::{
    auth::{NativeAuth, SmtpMailer},
    router, AppState,
};
use companion_storage::accounts::AccountStore;
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    sync::Arc,
};
use zeroize::Zeroizing;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
fn setting(name: &str) -> Result<String, &'static str> {
    std::env::var(name)
        .map_err(|_| "Missing coordinator configuration; see docs/development/coordinator-auth.md.")
}
async fn run() -> Result<(), &'static str> {
    let secret = Zeroizing::new(
        std::fs::read_to_string(setting("QIBAN_AUTH_SECRET_FILE")?)
            .map_err(|_| "Cannot read authentication secret file.")?,
    );
    let pepper = Zeroizing::new(
        STANDARD
            .decode(secret.trim())
            .map_err(|_| "Invalid authentication secret encoding.")?,
    );
    let pepper: [u8; 32] = pepper
        .as_slice()
        .try_into()
        .map_err(|_| "Authentication secret must contain 32 random bytes.")?;
    let emails: Vec<String> = setting("QIBAN_ALLOWED_EMAILS")?
        .split(',')
        .map(String::from)
        .collect();
    let tls = std::env::var("QIBAN_SMTP_TLS").unwrap_or_else(|_| "starttls".into());
    if !matches!(tls.as_str(), "starttls" | "tls") {
        return Err("QIBAN_SMTP_TLS must be starttls (587) or tls (465).");
    }
    let host = setting("QIBAN_SMTP_HOST")?;
    let username = setting("QIBAN_SMTP_USERNAME")?;
    // Mirror the launcher rules on the manual environment path so a
    // misconfigured Cloudflare relay fails at startup, not at first mail.
    if host
        .trim_end_matches('.')
        .eq_ignore_ascii_case("smtp.mx.cloudflare.net")
        && (tls != "tls" || username != "api_token")
    {
        return Err("Cloudflare SMTP requires QIBAN_SMTP_TLS=tls (465) and username api_token.");
    }
    let mailer = SmtpMailer::new(
        &host,
        username,
        setting("QIBAN_SMTP_PASSWORD")?,
        &setting("QIBAN_SMTP_FROM")?,
        tls == "starttls",
    )
    .map_err(|_| "Invalid SMTP configuration.")?;
    let path = PathBuf::from(setting("QIBAN_COORDINATOR_DB")?);
    let port = std::env::var("QIBAN_COORDINATOR_PORT")
        .unwrap_or_else(|_| "4318".into())
        .parse::<u16>()
        .map_err(|_| "Invalid QIBAN_COORDINATOR_PORT.")?;
    let store = Arc::new(
        AccountStore::open(&path).map_err(|_| "Cannot open dedicated coordinator database.")?,
    );
    let auth = NativeAuth::new(store.clone(), Arc::new(mailer), pepper, &emails)
        .map_err(|_| "Invalid authentication configuration.")?;
    let listener = tokio::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|_| "Cannot bind coordinator loopback port.")?;
    println!(
        "Qiban coordinator listening on {} (local integration only).",
        listener
            .local_addr()
            .map_err(|_| "Cannot read listener address.")?
    );
    axum::serve(listener, router(AppState::new(auth, store)))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|_| "Coordinator stopped unexpectedly.")
}

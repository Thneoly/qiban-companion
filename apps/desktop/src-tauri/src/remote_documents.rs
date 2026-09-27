//! Isolated remote ledger; ordinary local execution IPC cannot access these jobs.
use crate::{
    account::{AccountClient, AccountError, AccountState},
    execution::ExecutionState,
};
use companion_core::{
    authorization::{document_digest, ActionBinding, ActionScope, DocumentAction, ShareDocument},
    execution::ExecutionStatus,
};
use companion_storage::remote_jobs::{Prepared, RemoteJobStore};
use reqwest::Method;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{Manager, State};
type Result<T> = std::result::Result<T, AccountError>;
fn local_error<T>(_: T) -> AccountError {
    AccountError::new("document_storage")
}
pub struct RemoteDocuments {
    root: PathBuf,
    pending: std::sync::atomic::AtomicBool,
}
struct Workspace {
    executor: ExecutionState,
    jobs: RemoteJobStore,
}
impl Workspace {
    fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root).map_err(local_error)?;
        Ok(Self {
            jobs: RemoteJobStore::open(&root.join("remote-jobs.db")).map_err(local_error)?,
            executor: ExecutionState::open(root).map_err(local_error)?,
        })
    }
    fn load(&self, id: &str) -> Result<(Prepared, String)> {
        self.jobs.load(id).map_err(local_error)
    }
    fn phase(&self, id: &str, phase: &str) -> Result<()> {
        self.jobs.phase(id, phase).map_err(local_error)
    }
}
impl RemoteDocuments {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            pending: std::sync::atomic::AtomicBool::new(true),
        }
    }
    async fn workspace(&self, c: &AccountClient) -> Result<Workspace> {
        Workspace::open(&self.root.join(c.remote_owner().await?))
    }
    pub async fn prepare(
        &self,
        c: &AccountClient,
        pair_id: String,
        request_id: String,
        name: String,
        text: String,
    ) -> Result<Prepared> {
        let pair = c
            .pairings()
            .await?
            .into_iter()
            .find(|p| p.id == pair_id && p.status == "active" && p.current_role == "desktop")
            .ok_or(AccountError::new("pairing_denied"))?;
        let w = self.workspace(c).await?;
        let task = w
            .executor
            .store
            .prepare(&request_id, &name, &text)
            .map_err(local_error)?;
        let exists = w.jobs.contains(&task.id).map_err(local_error)?;
        if exists {
            let (old, _) = w.load(&task.id)?;
            if old.share.pairing_id != pair_id {
                return Err(AccountError::new("conflict"));
            }
            return Ok(old);
        }
        let binding = ActionBinding {
            action_id: task.action_id.clone(),
            resource_id: task.id.clone(),
            resource_version: 1,
            parameters_digest: document_digest(&task.source_name, &task.preview),
            pair_revision: pair.revision,
            scope: ActionScope::DocumentExcerpt,
        };
        let value = Prepared {
            share: ShareDocument {
                pairing_id: pair_id,
                binding,
                source_name: task.source_name.clone(),
                preview: task.preview.clone(),
            },
            task,
        };
        w.jobs.insert(&value).map_err(local_error)?;
        Ok(value)
    }
    pub async fn share(&self, c: &AccountClient, id: &str) -> Result<DocumentAction> {
        let w = self.workspace(c).await?;
        let (p, _) = w.load(id)?;
        w.phase(id, "sharing")?;
        self.pending
            .store(true, std::sync::atomic::Ordering::Relaxed);
        call(
            c,
            Method::POST,
            "/v1/documents",
            Some(serde_json::to_value(&p.share).map_err(local_error)?),
        )
        .await
    }
    pub async fn list(&self, c: &AccountClient) -> Result<Vec<DocumentAction>> {
        call(c, Method::GET, "/v1/documents", None).await
    }
    pub async fn sync(&self, c: &AccountClient) -> Result<Vec<DocumentAction>> {
        let w = self.workspace(c).await?;
        let docs = self.list(c).await?;
        for d in docs.iter().filter(|d| d.current_role == "desktop") {
            let id = &d.authorization.binding.resource_id;
            let Ok((p, phase)) = w.load(id) else {
                continue;
            };
            if phase == "local" || phase == "reported" {
                continue;
            }
            if p.share.binding != d.authorization.binding
                || p.share.pairing_id != d.authorization.pairing_id
                || p.task.preview != d.preview
                || p.task.source_name != d.source_name
                || p.task.artifact_hash != d.artifact_hash
            {
                return Err(AccountError::new("conflict"));
            }
            let route = format!("/v1/documents/{}", p.task.action_id);
            let state = d.authorization.state.as_str();
            if state == "confirmed" {
                if w.executor
                    .store
                    .detail(id)
                    .map_err(local_error)?
                    .task
                    .status
                    != ExecutionStatus::WaitingConfirmation
                {
                    continue;
                }
                // Persist BEFORE requesting single-use admission. Only a positive reply in
                // this call may cause execution; a later sync never replays an admission.
                w.phase(id, "claiming")?;
                let admitted: DocumentAction = call(
                    c,
                    Method::POST,
                    &format!("{route}/admit"),
                    Some(serde_json::to_value(&p.share.binding).map_err(local_error)?),
                )
                .await?;
                if admitted.authorization.state != "admitted" {
                    continue;
                }
                w.phase(id, "admitted")?;
                let current: DocumentAction = call(c, Method::GET, &route, None).await?;
                if current.authorization.state == "admitted"
                    && current.authorization.binding == p.share.binding
                {
                    let _ = w.executor.execute(id, p.task.revision);
                }
            }
            if ["admitted", "cancel_requested", "confirmed", "unknown"].contains(&state) {
                // Includes admission reply loss: pending local task is cancelled without writing.
                let mut local = w.executor.store.detail(id).map_err(local_error)?;
                if local.task.status == ExecutionStatus::WaitingConfirmation {
                    w.executor
                        .store
                        .cancel(id, local.task.revision)
                        .map_err(local_error)?;
                    local = w.executor.store.detail(id).map_err(local_error)?;
                }
                if matches!(
                    local.task.status,
                    ExecutionStatus::Running | ExecutionStatus::Unknown
                ) {
                    let _ = w.executor.reconcile(id);
                    local = w.executor.store.detail(id).map_err(local_error)?;
                }
                let outcome = match local.task.status {
                    ExecutionStatus::Completed if w.executor.result(id).is_ok() => "completed",
                    ExecutionStatus::Cancelled | ExecutionStatus::Failed => "failed",
                    _ => "unknown",
                };
                let _: DocumentAction = call(
                    c, Method::POST, &format!("{route}/receipt"),
                    Some(json!({"binding":p.share.binding,"state":outcome,"artifactHash":p.task.artifact_hash})),
                ).await?;
                if outcome != "unknown" {
                    w.phase(id, "reported")?;
                }
            } else if ["cancelled", "completed", "failed"].contains(&state) {
                let local = w.executor.store.detail(id).map_err(local_error)?;
                if local.task.status == ExecutionStatus::WaitingConfirmation {
                    w.executor
                        .store
                        .cancel(id, local.task.revision)
                        .map_err(local_error)?;
                }
                w.phase(id, "reported")?;
            }
        }
        let rows = self.list(c).await?;
        self.pending.store(
            rows.iter().any(|d| {
                d.current_role == "desktop"
                    && !["completed", "failed", "cancelled"]
                        .contains(&d.authorization.state.as_str())
            }),
            std::sync::atomic::Ordering::Relaxed,
        );
        Ok(rows)
    }
}
async fn call<T: serde::de::DeserializeOwned>(
    c: &AccountClient,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<T> {
    let secret = c.active_secret()?;
    let value = c
        .request(method, path, Some(&secret.token), body)
        .await
        .map_err(|e| {
            if e.code == "pairing_capacity" {
                AccountError::new("document_capacity")
            } else {
                e
            }
        })?;
    serde_json::from_value(value).map_err(|_| AccountError::new("unavailable"))
}
#[tauri::command]
pub async fn remote_document_prepare(
    pair_id: String,
    request_id: String,
    source_name: String,
    text: String,
    account: State<'_, AccountState>,
    remote: State<'_, RemoteDocuments>,
) -> Result<Prepared> {
    let c = account.lock().await;
    remote
        .prepare(&c, pair_id, request_id, source_name, text)
        .await
}
#[tauri::command]
pub async fn remote_document_share(
    id: String,
    account: State<'_, AccountState>,
    remote: State<'_, RemoteDocuments>,
) -> Result<DocumentAction> {
    let c = account.lock().await;
    remote.share(&c, &id).await
}
#[tauri::command]
pub async fn remote_document_sync(
    account: State<'_, AccountState>,
    remote: State<'_, RemoteDocuments>,
) -> Result<Vec<DocumentAction>> {
    let c = account.lock().await;
    remote.sync(&c).await
}
pub fn start(app: tauri::AppHandle) {
    // Two independent loops. The heartbeat MUST NOT share a tick with sync:
    // a slow sync (sequential document calls, each with a 12s timeout, plus
    // local executor work) would push the next beat past the 15s lease and
    // flap a healthy desktop to 电脑离线 on the phone. A separate loop also
    // bounds each loop's lock hold to its own request, so a hung coordinator
    // stalls UI commands for at most one call, not both.
    let heartbeat_app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            // Beat on every tick while signed in — independent of pending
            // documents, so an idle desktop stays online (and its session no
            // longer idle-expires). Best-effort: errors are ignored (401
            // clears the vault token in request() and beating stops; 404 =
            // an older coordinator).
            let account = heartbeat_app.state::<AccountState>();
            let c = account.lock().await;
            if c.active_secret().is_ok() {
                let _ = c.heartbeat(&capabilities()).await;
            }
            drop(c);
        }
    });
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let account = app.state::<AccountState>();
            let c = account.lock().await;
            if app
                .state::<RemoteDocuments>()
                .pending
                .load(std::sync::atomic::Ordering::Relaxed)
                && c.active_secret().is_ok()
            {
                let _ = app.state::<RemoteDocuments>().sync(&c).await;
            }
            drop(c);
        }
    });
}
/// The executor's advertised capability slugs, generated from the same
/// companion-core constant the coordinator validates against and the phone
/// maps labels over — never a hand-written list.
fn capabilities() -> Vec<String> {
    companion_core::authorization::ACTION_SCOPES
        .iter()
        .map(|scope| scope.slug().to_string())
        .collect()
}

//! The third background loop: cross-device memory sync. Pull-first, then
//! push — a reconnect applies the server's deletions before resuming our own
//! uploads, so an offline delete never resurrects through a stale base.
//!
//! It MUST NOT share a tick with the heartbeat or document loops for the same
//! reason they do not share one with each other (see remote_documents::start):
//! slow sequential HTTP here would push the presence beat past its 15s lease
//! and flap a healthy desktop offline. It also holds its locks the other way
//! around — the chat lock is taken only around synchronous store work and
//! never across an await, while the account lock (the remote_documents
//! precedent) may span the HTTP calls. The two locks are never held at the
//! same time, so no ordering cycle exists.
//!
//! Local application reuses the domain semantics wholesale: applying a remote
//! delete or wording change clears the chats that quoted the old content and
//! invalidates in-flight completions exactly like a local edit would.
use crate::account::AccountState;
use crate::chat::ChatState;
use companion_storage::memory_sync_outbox::{OutboxEntry, OutboxOp, RemoteApplyReport};
use tauri::{AppHandle, Manager};

const TICK: std::time::Duration = std::time::Duration::from_secs(5);
/// Hard bound on pages per tick (200 events each): far above any real
/// backlog the 512-event window can present, low enough to always yield.
const MAX_PAGES_PER_TICK: usize = 16;

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            tick(&app).await;
        }
    });
}

async fn tick(app: &AppHandle) {
    let owner = {
        let state = app.state::<AccountState>();
        let c = state.lock().await;
        if c.active_secret().is_err() {
            return;
        }
        match c.remote_owner().await {
            Ok(owner) => owner,
            Err(_) => return,
        }
    };
    settle_task_facts(app);
    pull(app, &owner).await;
    push(app, &owner).await;
}

/// Drain the execution store's fact ledger into the memory store. Runs every
/// tick (idempotent, cheap when empty) so a crash between the execution
/// commit and the deposit — the two-database two-phase window — closes on
/// the next tick without a dedicated startup path. Locks are taken one at a
/// time: read the ledger, deposit, then clear the row.
fn settle_task_facts(app: &AppHandle) {
    // ExecutionStore serializes on its own connection mutex — the long-op
    // `operation` lock is for file work, not for these short ledger queries.
    let pending = {
        let state = app.state::<crate::execution::ExecutionState>();
        match state.store.fact_pending() {
            Ok(pending) => pending,
            Err(_) => return,
        }
    };
    if pending.is_empty() {
        return;
    }
    let chat = app.state::<ChatState>();
    for (fingerprint, task_id, task) in pending {
        // The projection is the ONLY fact source and lives in the scope
        // contract table (single scope today; a second scope adds a row and
        // resolves by the task's scope when one is carried). None here means
        // a projection rule changed under old ledger rows — settle without
        // depositing.
        let projection = companion_core::execution::SCOPE_CONTRACTS[0].fact_projection;
        let Some((body, event_date)) = projection(&task) else {
            let state = app.state::<crate::execution::ExecutionState>();
            let _ = state.store.fact_settled(&task_id);
            continue;
        };
        let deposited = {
            let Ok(mut inner) = chat.0.lock() else {
                return;
            };
            let Some(store) = inner.store.as_mut() else {
                return;
            };
            store
                .task_fact_deposit(&fingerprint, &body, event_date.as_deref())
                .is_ok()
        };
        if deposited {
            let state = app.state::<crate::execution::ExecutionState>();
            let _ = state.store.fact_settled(&task_id);
            // The deposit bumped the epoch: emit like a local mutation so
            // panels refresh and in-flight completions die.
            notify_changed(app);
        }
    }
}

fn notify_changed(app: &AppHandle) {
    let state = app.state::<ChatState>();
    let Ok(inner) = state.0.lock() else {
        return;
    };
    let Some(store) = inner.store.as_ref() else {
        return;
    };
    let Ok(context_epoch) = store.context_epoch() else {
        return;
    };
    drop(inner);
    let _ = tauri::Emitter::emit(
        app,
        "memory-changed",
        crate::memory::MemoryChanged {
            context_epoch,
            chat_cleared: false,
            remote: false,
        },
    );
}

/// Pull until drained. The server keeps events for us until we report the new
/// cursor back (at-least-once), so a lost response only costs a re-pull. A
/// page cap and a cursor-progress check bound the loop against a coordinator
/// that keeps answering pending=true with a standing cursor — the server
/// controls paging, but it must never hold one tick hostage.
async fn pull(app: &AppHandle, owner: &str) {
    let mut last_cursor = None;
    for _ in 0..MAX_PAGES_PER_TICK {
        let Some((generation, cursor)) = read_sync_state(app, owner) else {
            return;
        };
        let response = {
            let state = app.state::<AccountState>();
            let c = state.lock().await;
            match c.memory_pull(generation, cursor).await {
                Ok(response) => response,
                Err(_) => return,
            }
        };
        let pending = response.pending;
        let report = match apply(app, owner, &response) {
            Some(report) => report,
            None => return,
        };
        if report.changed {
            notify(app, &report);
        }
        if !pending {
            return;
        }
        if Some(response.cursor) == last_cursor {
            // No forward progress between pages: stop instead of spinning.
            return;
        }
        last_cursor = Some(response.cursor);
    }
}

/// One transaction per pull: events (or the snapshot) plus the cursor move
/// land together or not at all. Returns None when local storage is offline.
fn apply(
    app: &AppHandle,
    owner: &str,
    response: &companion_core::memory::MemorySyncResponse,
) -> Option<RemoteApplyReport> {
    let state = app.state::<ChatState>();
    let mut inner = state.0.lock().ok()?;
    let store = inner.store.as_mut()?;
    let report = if let Some(items) = &response.snapshot {
        store
            .memory_apply_snapshot(owner, items, response.generation, response.cursor)
            .ok()?
    } else {
        store
            .memory_apply_remote(
                owner,
                &response.events,
                response.generation,
                response.cursor,
            )
            .ok()?
    };
    // Same invalidation contract as a local edit: remote changes cancel the
    // active stream, and a pruned history reloads its surviving prefix so a
    // restart cannot resurrect it (ghost split).
    inner.invalidate_memory(report.cleared_turns > 0);
    Some(report)
}

async fn push(app: &AppHandle, owner: &str) {
    let Some(entries) = read_outbox(app) else {
        return;
    };
    for entry in entries {
        let resolved = resolve_push(app, owner, &entry);
        let Some((op, base_revision)) = resolved else {
            // The owner's server has never met this entry; a delete for it is
            // a net zero — clear the row without a round trip.
            ack(app, owner, &entry.request_id, &entry.memory_id, 0);
            continue;
        };
        let result = {
            let state = app.state::<AccountState>();
            let c = state.lock().await;
            c.memory_push(&entry, op, base_revision).await
        };
        match result {
            Ok(receipt) => ack(
                app,
                owner,
                &entry.request_id,
                &entry.memory_id,
                receipt.revision,
            ),
            Err(error) => {
                if matches!(
                    error.code,
                    "memory_revision_conflict" | "memory_resurrection" | "memory_capacity"
                ) {
                    // Another device won. Drop the change for good — never
                    // rebase onto the newer base — and reset the pull
                    // position: the snapshot diff on the next tick is what
                    // converges the wording this pending row had skipped.
                    let _ = drop_row(app, owner, &entry.request_id);
                    continue;
                }
                // Network or server trouble: leave the row for the next tick.
                return;
            }
        }
    }
}

/// Update degrades to create for an owner whose server has never seen the
/// entry (account switch = merge). Returns None only when there is nothing to
/// push at all.
fn resolve_push(app: &AppHandle, owner: &str, entry: &OutboxEntry) -> Option<(&'static str, i64)> {
    match entry.op {
        OutboxOp::Create => Some(("create", 0)),
        OutboxOp::Update => {
            let base = push_base(app, owner, &entry.memory_id);
            Some(match base {
                Some(revision) => ("update", revision),
                None => ("create", 0),
            })
        }
        OutboxOp::Delete => push_base(app, owner, &entry.memory_id).map(|base| ("delete", base)),
    }
}

fn read_sync_state(app: &AppHandle, owner: &str) -> Option<(i64, i64)> {
    let state = app.state::<ChatState>();
    let inner = state.0.lock().ok()?;
    inner.store.as_ref()?.memory_sync_state(owner).ok()
}

fn push_base(app: &AppHandle, owner: &str, memory_id: &str) -> Option<i64> {
    let state = app.state::<ChatState>();
    let inner = state.0.lock().ok()?;
    inner
        .store
        .as_ref()?
        .memory_push_base(owner, memory_id)
        .ok()
        .flatten()
}

fn read_outbox(app: &AppHandle) -> Option<Vec<OutboxEntry>> {
    let state = app.state::<ChatState>();
    let inner = state.0.lock().ok()?;
    inner.store.as_ref()?.memory_outbox_peek().ok()
}

fn ack(app: &AppHandle, owner: &str, request_id: &str, memory_id: &str, server_revision: i64) {
    let state = app.state::<ChatState>();
    let Ok(mut inner) = state.0.lock() else {
        return;
    };
    if let Some(store) = inner.store.as_mut() {
        let _ = store.memory_outbox_ack(owner, request_id, memory_id, server_revision);
    }
}

fn drop_row(app: &AppHandle, owner: &str, request_id: &str) -> Option<bool> {
    let state = app.state::<ChatState>();
    let mut inner = state.0.lock().ok()?;
    inner
        .store
        .as_mut()?
        .memory_outbox_drop(owner, request_id)
        .ok()
}

fn notify(app: &AppHandle, report: &RemoteApplyReport) {
    let state = app.state::<ChatState>();
    let Ok(inner) = state.0.lock() else {
        return;
    };
    let Some(store) = inner.store.as_ref() else {
        return;
    };
    let Ok(context_epoch) = store.context_epoch() else {
        return;
    };
    drop(inner);
    let _ = tauri::Emitter::emit(
        app,
        "memory-changed",
        crate::memory::MemoryChanged {
            context_epoch,
            chat_cleared: report.cleared_turns > 0,
            remote: true,
        },
    );
}

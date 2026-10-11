//! App-local reminders (T18a): user-created, time-fired, in-app only.
//! No system notification API (we never claim an OS notification was sent),
//! no task fact (no external side effect to verify), no ActionScope (the
//! creator is exclusively the user via this UI — the model cannot call these
//! commands). Storage is the reminders table on chat-history (schema v6);
//! restart recovery is the same scan the firing loop runs.
use crate::chat::ChatState;
use companion_core::now_ms;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

const FIRE_TICK: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub id: String,
    pub due_at: i64,
    pub message: String,
    pub status: &'static str,
}

fn list(store: &companion_storage::history::HistoryStore) -> Result<Vec<Reminder>, String> {
    let rows = store
        .reminder_rows()
        .map_err(|_| "本机存储不可用".to_string())?;
    Ok(rows
        .into_iter()
        .map(|(id, due_at, message, status)| Reminder {
            id,
            due_at,
            message,
            status: match status.as_str() {
                "fired" => "fired",
                "cancelled" => "cancelled",
                _ => "pending",
            },
        })
        .collect())
}

#[tauri::command]
pub fn reminder_list(state: State<'_, ChatState>) -> Result<Vec<Reminder>, String> {
    let inner = state.0.lock().map_err(|_| "对话状态不可用".to_string())?;
    let store = inner.store.as_ref().ok_or("本机存储不可用".to_string())?;
    list(store)
}

#[tauri::command]
pub fn reminder_add(
    message: String,
    due_in_minutes: i64,
    state: State<'_, ChatState>,
) -> Result<Vec<Reminder>, String> {
    let message = message.trim().to_string();
    if message.is_empty()
        || message.chars().count() > 200
        || message.contains('\0')
        || !(1..=60 * 24).contains(&due_in_minutes)
    {
        return Err("提醒需要 1～200 字，且时间在 1 分钟到 24 小时之间".into());
    }
    let mut inner = state.0.lock().map_err(|_| "对话状态不可用".to_string())?;
    let store = inner.store.as_mut().ok_or("本机存储不可用".to_string())?;
    store
        .reminder_insert(&message, now_ms() + due_in_minutes as u64 * 60_000)
        .map_err(|_| "本机存储不可用".to_string())?;
    list(store)
}

#[tauri::command]
pub fn reminder_cancel(id: String, state: State<'_, ChatState>) -> Result<Vec<Reminder>, String> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err("提醒标识无效".into());
    }
    let mut inner = state.0.lock().map_err(|_| "对话状态不可用".to_string())?;
    let store = inner.store.as_mut().ok_or("本机存储不可用".to_string())?;
    store
        .reminder_cancel(&id)
        .map_err(|_| "本机存储不可用".to_string())?;
    list(store)
}

/// The firing loop: every 15s, due pending reminders flip to fired and the
/// pet window gets one in-app bubble event (plus the panel refresh). A
/// restart replays the same scan — recovered, not re-notified retroactively:
/// reminders that came due while the app was down fire once on next launch.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(FIRE_TICK).await;
            let due = {
                let state = app.state::<ChatState>();
                let Ok(mut inner) = state.0.lock() else {
                    continue;
                };
                let Some(store) = inner.store.as_mut() else {
                    continue;
                };
                store.reminder_fire_due(now_ms()).unwrap_or_default()
            };
            for (_, message) in due {
                // In-app only: the pet window shows a bubble; no OS
                // notification is claimed or sent.
                let _ = app.emit_to(tauri::EventTarget::labeled("pet"), "reminder-due", &message);
                let _ = app.emit("panel-refresh", ());
            }
        }
    });
}

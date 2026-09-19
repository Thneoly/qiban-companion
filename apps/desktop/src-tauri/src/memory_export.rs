use crate::{chat::ChatState, memory::MemoryFailure};
use serde::Serialize;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Manager, State, WebviewWindow};

#[derive(Default)]
pub struct ExportState(pub Mutex<()>);
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum ExportResult {
    Cancelled,
    Saved { count: usize },
}

fn write_snapshot(
    state: &ChatState,
    expected_epoch: i64,
    destination: &Path,
    private_dir: &Path,
) -> Result<ExportResult, MemoryFailure> {
    let parent = destination
        .parent()
        .ok_or_else(MemoryFailure::export)?
        .canonicalize()
        .map_err(|_| MemoryFailure::export())?;
    if parent.starts_with(
        private_dir
            .canonicalize()
            .map_err(|_| MemoryFailure::export())?,
    ) || !destination
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return Err(MemoryFailure::export());
    }
    let inner = state.0.lock().map_err(|_| MemoryFailure::unavailable())?;
    let store = inner
        .store
        .as_ref()
        .ok_or_else(MemoryFailure::unavailable)?;
    if store.context_epoch()? != expected_epoch {
        return Err(MemoryFailure::changed());
    }
    let items = store.memory_list()?;
    let count = items.len();
    let bytes = serde_json::to_vec_pretty(
        &serde_json::json!({"formatVersion":1,"exportedAt":companion_core::now_ms(),"items":items}),
    )
    .map_err(|_| MemoryFailure::export())?;
    // Same-directory replacement: a failure never leaves a partially written destination.
    let temporary = parent.join(format!(".qiban-export-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| MemoryFailure::export())?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|_| MemoryFailure::export())?;
    // Keep the context lock until replacement is finished, serializing deletion with export.
    drop(inner);
    Ok(ExportResult::Saved { count })
}

#[cfg(windows)]
fn choose_destination(parent: isize) -> Result<Option<PathBuf>, MemoryFailure> {
    use windows::{
        core::{w, HRESULT},
        Win32::{
            Foundation::{ERROR_CANCELLED, HWND},
            System::Com::*,
            UI::Shell::{Common::COMDLG_FILTERSPEC, *},
        },
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    let run = || -> windows::core::Result<Option<PathBuf>> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let _apartment = Apartment;
            let dialog: IFileSaveDialog =
                CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetTitle(w!("导出栖伴记忆"))?;
            dialog.SetFileName(w!("qiban-memories.json"))?;
            dialog.SetDefaultExtension(w!("json"))?;
            dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
                pszName: w!("JSON 记忆文件"),
                pszSpec: w!("*.json"),
            }])?;
            dialog.SetOptions(
                FOS_OVERWRITEPROMPT
                    | FOS_FORCEFILESYSTEM
                    | FOS_PATHMUSTEXIST
                    | FOS_STRICTFILETYPES
                    | FOS_NOCHANGEDIR,
            )?;
            if let Err(error) = dialog.Show(Some(HWND(parent as *mut _))) {
                if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                    return Ok(None);
                }
                return Err(error);
            }
            let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let value = name.to_string();
            CoTaskMemFree(Some(name.0.cast()));
            Ok(Some(PathBuf::from(value?)))
        }
    };
    run().map_err(|_| MemoryFailure::export())
}
#[cfg(not(windows))]
fn choose_destination(_parent: isize) -> Result<Option<PathBuf>, MemoryFailure> {
    Err(MemoryFailure::export())
}

#[tauri::command]
pub async fn memory_export(
    window: WebviewWindow,
    state: State<'_, ChatState>,
) -> Result<ExportResult, MemoryFailure> {
    let expected = state.memory_snapshot()?.context_epoch;
    let app = window.app_handle().clone();
    #[cfg(windows)]
    let parent = window.hwnd().map_err(|_| MemoryFailure::export())?.0 as isize;
    #[cfg(not(windows))]
    let parent = 0;
    tauri::async_runtime::spawn_blocking(move || {
        let exports = app.state::<ExportState>();
        let _guard = exports.0.try_lock().map_err(|_| MemoryFailure::export())?;
        // A dedicated thread guarantees its own STA; the dialog never holds the chat mutex.
        let destination = std::thread::spawn(move || choose_destination(parent))
            .join()
            .map_err(|_| MemoryFailure::export())??;
        let Some(destination) = destination else {
            return Ok(ExportResult::Cancelled);
        };
        let private_dir = app
            .path()
            .app_local_data_dir()
            .map_err(|_| MemoryFailure::export())?;
        write_snapshot(
            &app.state::<ChatState>(),
            expected,
            &destination,
            &private_dir,
        )
    })
    .await
    .map_err(|_| MemoryFailure::export())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use companion_core::memory::{MemoryDraft, MemoryKind};
    fn deletion_export(after_export: bool) {
        let root = std::env::temp_dir().join(format!("q6-export-{}.tmp", uuid::Uuid::new_v4()));
        let private = root.join("private");
        std::fs::create_dir_all(&private).unwrap();
        let state = ChatState::open(&private.join("chat-history.db"));
        let path = root.join("memory.json");
        let m = state
            .0
            .lock()
            .unwrap()
            .store
            .as_mut()
            .unwrap()
            .memory_create(
                &MemoryDraft {
                    kind: MemoryKind::Preference,
                    body: "exported synthetic secret".into(),
                    event_date: None,
                },
                0,
            )
            .unwrap()
            .value;
        if after_export {
            write_snapshot(&state, 1, &path, &private).unwrap();
        } else {
            std::fs::write(&path, "original destination").unwrap();
        }
        state
            .0
            .lock()
            .unwrap()
            .store
            .as_mut()
            .unwrap()
            .memory_delete(&m.id, 1, 1)
            .unwrap();
        if after_export {
            assert!(std::fs::read_to_string(&path).unwrap().contains(&m.body));
            assert!(state.memory_snapshot().unwrap().items.is_empty());
        } else {
            assert!(write_snapshot(&state, 1, &path, &private).is_err());
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "original destination"
            );
        }
        write_snapshot(&state, 2, &path, &private).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains(&m.body));
        drop(state);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(private.join("chat-history.db")).unwrap();
        std::fs::remove_dir(private).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn q6_del17_delete_while_picker_open_rejects_old_export() {
        deletion_export(false);
    }
    #[test]
    fn q6_del18_existing_export_is_not_recalled() {
        deletion_export(true);
    }
    #[test]
    fn export_is_active_only_atomic_and_rechecks_after_picker() {
        let root = std::env::temp_dir().join(format!("qiban-export-{}", uuid::Uuid::new_v4()));
        let private = root.join("private");
        std::fs::create_dir_all(&private).unwrap();
        let state = ChatState::open(&private.join("chat-history.db"));
        let draft = |body: &str| MemoryDraft {
            kind: MemoryKind::Experience,
            body: body.into(),
            event_date: None,
        };
        {
            let mut inner = state.0.lock().unwrap();
            let store = inner.store.as_mut().unwrap();
            let old = store
                .memory_create(&draft("deleted secret"), 0)
                .unwrap()
                .value;
            store.memory_delete(&old.id, 1, 1).unwrap();
            store.memory_create(&draft("保留🌱"), 2).unwrap();
        }
        let path = root.join("memories.json");
        std::fs::write(&path, "original").unwrap();
        assert!(write_snapshot(&state, 1, &path, &private).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert!(matches!(
            write_snapshot(&state, 3, &path, &private).unwrap(),
            ExportResult::Saved { count: 1 }
        ));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains("deleted secret"));
        assert!(!body.contains("contextEpoch"));
        assert!(body.contains("保留🌱"));
        assert!(write_snapshot(&state, 3, &private.join("unsafe.json"), &private).is_err());
        let blocked = root.join("directory.json");
        std::fs::create_dir(&blocked).unwrap();
        assert!(write_snapshot(&state, 3, &blocked, &private).is_err());
        assert!(!std::fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        drop(state);
        std::fs::remove_file(private.join("chat-history.db")).unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(blocked).unwrap();
        std::fs::remove_dir(private).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}

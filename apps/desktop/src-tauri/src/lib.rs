mod account;
mod account_vault;
mod chat;
mod commands;
mod credentials;
mod execution;
mod instance;
mod memory;
mod memory_context;
mod memory_export;
mod model_settings;
mod personal_memory;
mod personal_memory_context;
mod pet;
mod placement;
mod remote_documents;
mod tray;
mod voice;
mod voice_companion;

use companion_storage::TaskStore;
use std::sync::{atomic::Ordering, Arc};
use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    let state = Arc::new(pet::PetState::default());
    let worker_state = state.clone();
    tauri::Builder::default()
        // First plugin, before setup opens any database. Ignore external argv/cwd.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Err(error) = pet::restore(app, false) {
                eprintln!("restore existing instance: {error}");
            }
        }))
        .manage(state)
        .manage(voice::VoiceState::default())
        .manage(voice_companion::VoiceTurnState::default())
        .manage(memory_export::ExportState::default())
        .setup(move |app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let lease = instance::DataLease::acquire(&data_dir).inspect_err(|_| {
                instance::report_unavailable();
            })?;
            app.manage(lease);
            app.manage(chat::ChatState::open(&data_dir.join("chat-history.db")));
            app.manage(TaskStore::open(&data_dir.join("companion.db"))?);
            app.manage(execution::ExecutionState::open(&data_dir)?);
            app.manage(remote_documents::RemoteDocuments::new(
                data_dir.join("remote-documents"),
            ));
            app.manage(
                account::open_state(&data_dir.join("account-settings.db"))
                    .map_err(|e| e.message)?,
            );
            app.manage(std::sync::Mutex::new(model_settings::ModelStore::open(
                &data_dir.join("model-settings.db"),
            )?));
            let saved_position =
                match placement::PlacementStore::open(&data_dir.join("desktop-settings.db")) {
                    Ok(store) => {
                        let saved = store.saved();
                        app.manage(std::sync::Mutex::new(store));
                        saved
                    }
                    Err(error) => {
                        eprintln!("pet placement unavailable; using default position: {error}");
                        None
                    }
                };
            remote_documents::start(app.handle().clone());
            // Fail before showing a hidden-window-only UI if the recovery tray cannot be installed.
            tray::install(app.handle())?;
            let window = app.get_webview_window("pet").ok_or("missing pet window")?;
            window.set_ignore_cursor_events(true)?;
            if let Some(position) = saved_position {
                window.set_position(tauri::PhysicalPosition::new(position.x, position.y))?;
            }
            pet::recover_position(&window, saved_position.is_none())?;
            pet::start_hit_testing(app.handle().clone(), worker_state.clone());
            let persistence_app = app.handle().clone();
            std::thread::spawn(move || {
                while !worker_state.stopped.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    placement::flush(&persistence_app, false);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "pet" {
                if let WindowEvent::Moved(position) = event {
                    placement::moved(window.app_handle(), *position);
                }
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    eprintln!("hide window: {error}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            remote_documents::remote_document_prepare,
            remote_documents::remote_document_share,
            remote_documents::remote_document_sync,
            account::account_pairings,
            account::account_pairing_offer,
            account::account_pairing_revoke,
            account::account_snapshot,
            account::account_port_save,
            account::account_code_request,
            account::account_login,
            account::account_task_create,
            account::account_task_cancel,
            account::account_logout,
            memory_context::chat_context_preview,
            memory_context::memory_policy_set,
            memory::memory_list,
            memory::memory_mutate,
            memory::chat_context_epoch,
            memory_export::memory_export,
            personal_memory::personal_memory_overview,
            personal_memory::personal_memory_recall,
            personal_memory::personal_memory_detail,
            personal_memory_context::personal_memory_policy_set,
            placement::guide_status,
            placement::guide_complete,
            voice::voice_probe,
            voice::voice_cancel,
            voice::voice_key_set,
            voice_companion::voice_transcribe,
            voice_companion::voice_speak,
            voice_companion::voice_turn_cancel,
            model_settings::voice_settings_get,
            model_settings::voice_settings_save,
            model_settings::model_settings_get,
            model_settings::model_settings_save,
            model_settings::model_key_set,
            model_settings::model_key_delete,
            chat::chat_config,
            chat::chat_history,
            chat::chat_clear,
            chat::chat_generate,
            chat::chat_cancel,
            execution::execution_list,
            execution::execution_prepare,
            execution::execution_detail,
            execution::execution_cancel,
            execution::execution_confirm,
            execution::execution_reconcile,
            execution::execution_result,
            commands::get_runtime_info,
            commands::list_tasks,
            commands::create_task,
            commands::cancel_task,
            pet::pet_action,
            pet::set_pet_regions,
        ])
        .build(tauri::generate_context!())
        .expect("桌面伴侣启动失败；请检查 WebView2、托盘和本地数据目录")
        .run(|app, event| {
            if matches!(event, RunEvent::ExitRequested { .. }) {
                placement::flush(app, true);
            }
            if matches!(event, RunEvent::Exit) {
                placement::flush(app, true);
                app.state::<Arc<pet::PetState>>()
                    .stopped
                    .store(true, Ordering::Relaxed);
            }
        });
}

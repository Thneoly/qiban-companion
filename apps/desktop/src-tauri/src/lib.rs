mod commands;
mod pet;
mod tray;

use companion_storage::TaskStore;
use std::sync::{atomic::Ordering, Arc};
use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    let state = Arc::new(pet::PetState::default());
    let worker_state = state.clone();
    tauri::Builder::default()
        .manage(state)
        .setup(move |app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            app.manage(TaskStore::open(&data_dir.join("companion.db"))?);
            // Fail before showing a hidden-window-only UI if the recovery tray cannot be installed.
            tray::install(app.handle())?;
            let window = app.get_webview_window("pet").ok_or("missing pet window")?;
            window.set_ignore_cursor_events(true)?;
            pet::recover_position(&window, true)?;
            pet::start_hit_testing(app.handle().clone(), worker_state.clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    eprintln!("hide window: {error}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
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
            if matches!(event, RunEvent::Exit) {
                app.state::<Arc<pet::PetState>>()
                    .stopped
                    .store(true, Ordering::Relaxed);
            }
        });
}

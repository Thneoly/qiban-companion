mod commands;

use companion_storage::TaskStore;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let store = TaskStore::open(&data_dir.join("companion.db"))?;
            app.manage(store);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_runtime_info,
            commands::list_tasks,
            commands::create_task,
            commands::cancel_task,
        ])
        .run(tauri::generate_context!())
        .expect("桌面应用启动失败；请检查 WebView2 和本地数据目录权限");
}

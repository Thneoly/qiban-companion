use crate::pet;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示栖栖 / 恢复互动", true, None::<&str>)?;
    let quiet = MenuItem::with_id(app, "quiet", "安静陪伴（鼠标穿透）", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏栖栖", true, None::<&str>)?;
    let reset = MenuItem::with_id(app, "reset", "找回角色（移回主屏）", true, None::<&str>)?;
    let panel = MenuItem::with_id(app, "panel", "打开任务面板", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出栖伴", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quiet, &hide, &reset, &panel, &quit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| std::io::Error::other("缺少托盘图标"))?;
    TrayIconBuilder::with_id("companion")
        .tooltip("栖伴 · 左键恢复角色，右键打开菜单")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let result = match event.id.as_ref() {
                "show" => pet::restore(app, false),
                "quiet" => pet::quiet(app),
                "hide" => app
                    .get_webview_window("pet")
                    .ok_or("角色窗口不可用".into())
                    .and_then(|w| w.hide().map_err(|e| e.to_string())),
                "reset" => pet::restore(app, true),
                "panel" => pet::open_panel(app),
                "quit" => {
                    app.exit(0);
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                eprintln!("tray action: {error}");
            }
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                if let Err(error) = pet::restore(tray.app_handle(), false) {
                    eprintln!("tray restore: {error}");
                }
            }
        })
        .build(app)?;
    Ok(())
}

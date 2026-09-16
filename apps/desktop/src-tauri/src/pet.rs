use serde::Deserialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow};

#[derive(Clone, Copy, Deserialize)]
pub struct HitRegion {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
impl HitRegion {
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
            && self.x + self.width <= 1024.0
            && self.y + self.height <= 1024.0
    }
    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}
#[derive(Default)]
pub struct PetState {
    regions: Mutex<Vec<HitRegion>>,
    quiet: AtomicBool,
    force_hit_test: AtomicBool,
    pub stopped: AtomicBool,
}
fn pet(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("pet")
        .ok_or_else(|| "角色窗口不可用".into())
}
fn message(error: tauri::Error) -> String {
    error.to_string()
}

fn clamp_axis(position: i32, start: i32, available: u32, size: u32) -> i32 {
    let end = (i64::from(start) + i64::from(available.saturating_sub(size)))
        .min(i64::from(i32::MAX)) as i32;
    position.clamp(start, end)
}
pub fn recover_position(window: &WebviewWindow, reset: bool) -> tauri::Result<()> {
    let Some(primary) = window.primary_monitor()? else {
        return Ok(());
    };
    let size = window.outer_size()?;
    let current = window.outer_position()?;
    let monitors = window.available_monitors()?;
    let monitor = monitors
        .iter()
        .find(|m| {
            let area = m.work_area();
            let cx = i64::from(current.x) + i64::from(size.width / 2);
            let cy = i64::from(current.y) + i64::from(size.height / 2);
            cx >= i64::from(area.position.x)
                && cy >= i64::from(area.position.y)
                && cx < i64::from(area.position.x) + i64::from(area.size.width)
                && cy < i64::from(area.position.y) + i64::from(area.size.height)
        })
        .unwrap_or(&primary);
    let area = if reset {
        primary.work_area()
    } else {
        monitor.work_area()
    };
    let requested = if reset {
        PhysicalPosition::new(
            area.position.x + area.size.width.saturating_sub(size.width + 20) as i32,
            area.position.y + area.size.height.saturating_sub(size.height + 12) as i32,
        )
    } else {
        current
    };
    let position = PhysicalPosition::new(
        clamp_axis(requested.x, area.position.x, area.size.width, size.width),
        clamp_axis(requested.y, area.position.y, area.size.height, size.height),
    );
    if position != current {
        window.set_position(position)?;
    }
    Ok(())
}
pub fn restore(app: &AppHandle, reset: bool) -> Result<(), String> {
    let window = pet(app)?;
    app.state::<Arc<PetState>>()
        .quiet
        .store(false, Ordering::Relaxed);
    app.state::<Arc<PetState>>()
        .force_hit_test
        .store(true, Ordering::Relaxed);
    window.set_focusable(true).map_err(message)?;
    recover_position(&window, reset).map_err(message)?;
    window.emit("pet-restored", ()).map_err(message)?;
    // No set_focus: tray restoration must not repeatedly steal the working application's focus.
    window.show().map_err(message)
}
pub fn open_panel(app: &AppHandle) -> Result<(), String> {
    let panel = app.get_webview_window("main").ok_or("任务面板不可用")?;
    panel.unminimize().map_err(message)?;
    panel.show().map_err(message)?;
    panel.set_focus().map_err(message)?;
    panel.emit("panel-refresh", ()).map_err(message)
}
pub fn quiet(app: &AppHandle) -> Result<(), String> {
    let window = pet(app)?;
    window.emit("pet-quiet", ()).map_err(message)?;
    window.set_ignore_cursor_events(true).map_err(message)?;
    window.set_focusable(false).map_err(message)?;
    app.state::<Arc<PetState>>()
        .quiet
        .store(true, Ordering::Relaxed);
    app.state::<Arc<PetState>>()
        .force_hit_test
        .store(true, Ordering::Relaxed);
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetAction {
    Ready,
    OpenPanel,
    Hide,
    Quiet,
    Restore,
    Drag,
}

#[tauri::command]
pub fn pet_action(app: AppHandle, action: PetAction) -> Result<(), String> {
    match action {
        PetAction::Ready => pet(&app)?.show().map_err(message),
        PetAction::OpenPanel => open_panel(&app),
        PetAction::Hide => pet(&app)?.hide().map_err(message),
        PetAction::Quiet => quiet(&app),
        PetAction::Restore => restore(&app, false),
        PetAction::Drag => pet(&app)?.start_dragging().map_err(message),
    }
}
#[tauri::command]
pub fn set_pet_regions(
    window: WebviewWindow,
    state: State<'_, Arc<PetState>>,
    regions: Vec<HitRegion>,
) -> Result<(), String> {
    if window.label() != "pet" || regions.len() > 16 || regions.iter().any(|r| !r.valid()) {
        return Err("角色交互区域无效".into());
    }
    *state.regions.lock().map_err(|_| "角色状态不可用")? = regions;
    Ok(())
}
#[cfg(windows)]
fn mouse_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    // Read button state only, so a native drag keeps its existing hit-test mode until release.
    unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) < 0 }
}
#[cfg(not(windows))]
fn mouse_down() -> bool {
    false
}

pub fn start_hit_testing(app: AppHandle, state: Arc<PetState>) {
    std::thread::spawn(move || {
        let mut ticks = 0_u32;
        let mut ignoring = true;
        while !state.stopped.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(40));
            let Some(window) = app.get_webview_window("pet") else {
                break;
            };
            if !window.is_visible().unwrap_or(false) {
                continue;
            }
            ticks += 1;
            if ticks % 50 == 0 && !mouse_down() {
                if let Err(error) = recover_position(&window, false) {
                    eprintln!("pet position: {error}");
                }
            }
            if mouse_down() {
                continue;
            }
            let should_ignore = if state.quiet.load(Ordering::Relaxed) {
                true
            } else if let (Ok(cursor), Ok(origin), Ok(scale)) = (
                window.cursor_position(),
                window.outer_position(),
                window.scale_factor(),
            ) {
                let x = (cursor.x - f64::from(origin.x)) / scale;
                let y = (cursor.y - f64::from(origin.y)) / scale;
                !state
                    .regions
                    .lock()
                    .map(|rs| rs.iter().any(|r| r.contains(x, y)))
                    .unwrap_or(false)
            } else {
                true
            };
            // Quiet can change the native flag independently; reapply on a restore transition.
            if (should_ignore != ignoring || state.force_hit_test.swap(false, Ordering::Relaxed))
                && window.set_ignore_cursor_events(should_ignore).is_ok()
            {
                ignoring = should_ignore;
            }
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovers_offscreen_and_handles_negative_monitor_origins() {
        assert_eq!(clamp_axis(-2200, -1920, 1920, 320), -1920);
        assert_eq!(clamp_axis(2000, 0, 1920, 320), 1600);
        assert_eq!(clamp_axis(100, 0, 200, 320), 0);
        assert_eq!(clamp_axis(-900, -1920, 1920, 320), -900);
    }
    #[test]
    fn transparent_gaps_are_not_interactive() {
        let body = HitRegion {
            x: 80.0,
            y: 200.0,
            width: 180.0,
            height: 210.0,
        };
        assert!(body.valid());
        assert!(body.contains(140.0, 250.0));
        assert!(!body.contains(10.0, 10.0));
        assert!(!body.contains(260.0, 250.0));
        assert!(!HitRegion {
            width: f64::NAN,
            ..body
        }
        .valid());
    }
}

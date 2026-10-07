//! Application-specific window preferences; never part of the task domain or frontend IPC.
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{Manager, PhysicalPosition};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

pub struct PlacementStore {
    connection: Connection,
    saved: Option<Position>,
    pending: Option<(Position, Instant)>,
}
impl PlacementStore {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let mut connection = Connection::open(path)?;
        // Exit of the previous instance (placement flush, journal cleanup) can
        // still hold the file when the next one starts; 250ms lost that race
        // (real case 2026-10-07: guide_status read failure on quick restart).
        connection.busy_timeout(Duration::from_secs(5))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 2 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch("CREATE TABLE pet_position (id INTEGER PRIMARY KEY CHECK(id=1), x INTEGER NOT NULL, y INTEGER NOT NULL); PRAGMA user_version=1;")?;
            transaction.commit()?;
        }
        if version < 2 {
            let transaction = connection.transaction()?;
            transaction.execute_batch("CREATE TABLE onboarding (id INTEGER PRIMARY KEY CHECK(id=1), completed INTEGER NOT NULL CHECK(completed IN (0,1))); PRAGMA user_version=2;")?;
            transaction.commit()?;
        }
        let saved = connection
            .query_row("SELECT x,y FROM pet_position WHERE id=1", [], |row| {
                Ok(Position {
                    x: row.get(0)?,
                    y: row.get(1)?,
                })
            })
            .optional()?;
        Ok(Self {
            connection,
            saved,
            pending: None,
        })
    }
    pub fn saved(&self) -> Option<Position> {
        self.saved
    }
    pub fn guide_completed(&self) -> rusqlite::Result<bool> {
        Ok(self
            .connection
            .query_row("SELECT completed FROM onboarding WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or(false))
    }
    pub fn complete_guide(&self) -> rusqlite::Result<()> {
        self.connection.execute("INSERT INTO onboarding(id,completed) VALUES(1,1) ON CONFLICT(id) DO UPDATE SET completed=1", [])?;
        Ok(())
    }
    pub fn moved(&mut self, position: Position, now: Instant) {
        self.pending = if Some(position) == self.saved {
            None
        } else {
            Some((position, now))
        };
    }
    pub fn flush(&mut self, now: Instant, force: bool) -> rusqlite::Result<()> {
        let Some((position, changed)) = self.pending else {
            return Ok(());
        };
        if !force && now.saturating_duration_since(changed) < Duration::from_millis(400) {
            return Ok(());
        }
        // Keep pending on failure; retry on the next tick instead of claiming it was saved.
        self.connection.execute(
            "INSERT INTO pet_position (id,x,y) VALUES (1,?1,?2) ON CONFLICT(id) DO UPDATE SET x=excluded.x,y=excluded.y",
            params![position.x, position.y],
        )?;
        self.saved = Some(position);
        self.pending = None;
        Ok(())
    }
}
pub type PlacementState = Mutex<PlacementStore>;

#[tauri::command]
pub fn guide_status(app: tauri::AppHandle) -> Result<bool, String> {
    let state = app
        .try_state::<PlacementState>()
        .ok_or("使用指南状态不可用，请检查本机设置后重启")?;
    let store = state.lock().map_err(|_| "本机设置不可用")?;
    store
        .guide_completed()
        .map_err(|_| "读取使用指南状态失败".to_string())
}
#[tauri::command]
pub fn guide_complete(app: tauri::AppHandle) -> Result<(), String> {
    let state = app
        .try_state::<PlacementState>()
        .ok_or("使用指南状态不可用，尚未记住完成状态")?;
    let store = state.lock().map_err(|_| "本机设置不可用")?;
    store
        .complete_guide()
        .map_err(|_| "保存使用指南状态失败，可稍后再试".to_string())
}

pub fn moved(app: &tauri::AppHandle, position: PhysicalPosition<i32>) {
    if let Some(state) = app.try_state::<PlacementState>() {
        if let Ok(mut store) = state.lock() {
            store.moved(
                Position {
                    x: position.x,
                    y: position.y,
                },
                Instant::now(),
            );
        }
    }
}
pub fn flush(app: &tauri::AppHandle, force: bool) {
    if let Some(state) = app.try_state::<PlacementState>() {
        if let Ok(mut store) = state.lock() {
            if let Err(error) = store.flush(Instant::now(), force) {
                eprintln!("pet placement save: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guide_migrates_existing_position_and_completion_survives_restart() {
        let path = std::env::temp_dir().join(format!("guide-{}.db", uuid::Uuid::new_v4()));
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch("CREATE TABLE pet_position(id INTEGER PRIMARY KEY CHECK(id=1), x INTEGER NOT NULL, y INTEGER NOT NULL); INSERT INTO pet_position VALUES(1,-800,200); PRAGMA user_version=1;").unwrap();
        }
        {
            let store = PlacementStore::open(&path).unwrap();
            assert_eq!(store.saved(), Some(Position { x: -800, y: 200 }));
            assert!(!store.guide_completed().unwrap());
            store
                .connection
                .pragma_update(None, "query_only", true)
                .unwrap();
            assert!(store.complete_guide().is_err());
            assert!(!store.guide_completed().unwrap());
            store
                .connection
                .pragma_update(None, "query_only", false)
                .unwrap();
            store.complete_guide().unwrap();
            store.complete_guide().unwrap();
        }
        {
            let store = PlacementStore::open(&path).unwrap();
            assert!(store.guide_completed().unwrap());
            assert_eq!(store.saved(), Some(Position { x: -800, y: 200 }));
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn debounces_drag_restores_negative_coordinates_and_flushes_on_exit() {
        let path = std::env::temp_dir().join(format!("pet-placement-{}.db", uuid::Uuid::new_v4()));
        let now = Instant::now();
        {
            let mut store = PlacementStore::open(&path).unwrap();
            assert_eq!(store.saved(), None);
            store.moved(Position { x: 100, y: 200 }, now);
            store.moved(
                Position { x: -1200, y: 300 },
                now + Duration::from_millis(100),
            );
            store
                .flush(now + Duration::from_millis(450), false)
                .unwrap();
            assert_eq!(store.saved(), None);
            store
                .flush(now + Duration::from_millis(550), false)
                .unwrap();
        }
        {
            let mut store = PlacementStore::open(&path).unwrap();
            assert_eq!(store.saved(), Some(Position { x: -1200, y: 300 }));
            store.moved(Position { x: 50, y: 60 }, now);
            store.flush(now, true).unwrap();
        }
        assert_eq!(
            PlacementStore::open(&path).unwrap().saved(),
            Some(Position { x: 50, y: 60 })
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn refuses_future_preferences_without_overwriting() {
        let path = std::env::temp_dir().join(format!("pet-placement-{}.db", uuid::Uuid::new_v4()));
        {
            let connection = Connection::open(&path).unwrap();
            connection.pragma_update(None, "user_version", 3).unwrap();
        }
        assert!(PlacementStore::open(&path).is_err());
        {
            let connection = Connection::open(&path).unwrap();
            let version: u32 = connection
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(version, 3);
        }
        std::fs::remove_file(path).unwrap();
    }
}

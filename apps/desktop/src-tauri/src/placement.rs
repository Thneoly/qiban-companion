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
        connection.busy_timeout(Duration::from_millis(250))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch("CREATE TABLE pet_position (id INTEGER PRIMARY KEY CHECK(id=1), x INTEGER NOT NULL, y INTEGER NOT NULL); PRAGMA user_version=1;")?;
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
            connection.pragma_update(None, "user_version", 2).unwrap();
        }
        assert!(PlacementStore::open(&path).is_err());
        {
            let connection = Connection::open(&path).unwrap();
            let version: u32 = connection
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(version, 2);
        }
        std::fs::remove_file(path).unwrap();
    }
}

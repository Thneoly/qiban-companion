//! One local archive, globally bounded; model context is scoped by endpoint and model.
use crate::StorageError;
use companion_core::conversation::ChatTurn;
use rusqlite::{params, Connection};
use std::{path::Path, time::Duration};

pub struct HistoryStore(Connection);
impl HistoryStore {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Self::from_connection(Connection::open(path)?)
    }
    fn from_connection(mut connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(Duration::from_millis(250))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 1 {
            return Err(StorageError::NewerSchema);
        }
        if version == 0 {
            let tx = connection.transaction()?;
            tx.execute_batch("CREATE TABLE chat_turns (id INTEGER PRIMARY KEY, base TEXT NOT NULL, model TEXT NOT NULL, user TEXT NOT NULL, assistant TEXT NOT NULL); PRAGMA user_version=1;")?;
            tx.commit()?;
        }
        connection.pragma_update(None, "secure_delete", "ON")?;
        // Fail closed on incompatible/corrupt data, without deleting or overwriting it.
        let (count, size): (i64, i64) = connection.query_row(
            "SELECT count(*), coalesce(sum(length(user)+length(assistant)),0) FROM chat_turns",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if count > 6 || size > 12000 {
            return Err(StorageError::Unavailable);
        }
        Ok(Self(connection))
    }
    pub fn load(&self, base: &str, model: &str) -> Result<Vec<ChatTurn>, StorageError> {
        Self::read(&self.0, base, model)
    }
    fn read(
        connection: &Connection,
        base: &str,
        model: &str,
    ) -> Result<Vec<ChatTurn>, StorageError> {
        let mut statement = connection.prepare(
            "SELECT user, assistant FROM chat_turns WHERE base=?1 AND model=?2 ORDER BY id",
        )?;
        let turns = statement
            .query_map(params![base, model], |r| {
                Ok(ChatTurn {
                    user: r.get(0)?,
                    assistant: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(turns)
    }
    pub fn append(
        &mut self,
        base: &str,
        model: &str,
        turn: &ChatTurn,
    ) -> Result<Vec<ChatTurn>, StorageError> {
        // Oversized complete pairs are not archived; preserve previous history.
        if turn.user.is_empty()
            || turn.assistant.is_empty()
            || turn.user.chars().count() + turn.assistant.chars().count() > 12000
            || turn.user.contains('\0')
            || turn.assistant.contains('\0')
        {
            return Err(StorageError::Unavailable);
        }
        let tx = self.0.transaction()?;
        tx.execute(
            "INSERT INTO chat_turns(base,model,user,assistant) VALUES(?1,?2,?3,?4)",
            params![base, model, turn.user, turn.assistant],
        )?;
        loop {
            let (count, size): (i64, i64) = tx.query_row(
                "SELECT count(*), coalesce(sum(length(user)+length(assistant)),0) FROM chat_turns",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if count <= 6 && size <= 12000 {
                break;
            }
            tx.execute(
                "DELETE FROM chat_turns WHERE id=(SELECT min(id) FROM chat_turns)",
                [],
            )?;
        }
        let turns = Self::read(&tx, base, model)?;
        tx.commit()?;
        Ok(turns)
    }
    pub fn clear(&mut self) -> Result<(), StorageError> {
        self.0.execute("DELETE FROM chat_turns", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn turn(n: usize) -> ChatTurn {
        ChatTurn {
            user: format!("问题{n}"),
            assistant: "答复🌱".into(),
        }
    }
    #[test]
    fn reopens_isolates_prunes_and_deletes_all_scopes() {
        let path = std::env::temp_dir().join(format!("history-{}.db", uuid::Uuid::new_v4()));
        {
            let mut store = HistoryStore::open(&path).unwrap();
            for n in 0..8 {
                store.append("https://a", "a", &turn(n)).unwrap();
            }
            assert_eq!(store.load("https://a", "a").unwrap()[0].user, "问题2");
            store.append("https://b", "a", &turn(8)).unwrap();
            store.append("https://a", "b", &turn(9)).unwrap();
        }
        {
            let mut store = HistoryStore::open(&path).unwrap();
            assert_eq!(store.load("https://a", "a").unwrap().len(), 4);
            assert_eq!(store.load("https://b", "a").unwrap(), vec![turn(8)]);
            assert_eq!(store.load("https://a", "b").unwrap(), vec![turn(9)]);
            store
                .append(
                    "https://a",
                    "b",
                    &ChatTurn {
                        user: "新".into(),
                        assistant: "字".repeat(11999),
                    },
                )
                .unwrap();
            assert!(store.load("https://a", "a").unwrap().is_empty());
            assert!(store
                .append(
                    "https://a",
                    "b",
                    &ChatTurn {
                        user: "超".into(),
                        assistant: "字".repeat(12000)
                    }
                )
                .is_err());
            assert_eq!(store.load("https://a", "b").unwrap().len(), 1);
            store.clear().unwrap();
            store.clear().unwrap();
        }
        assert!(HistoryStore::open(&path)
            .unwrap()
            .load("https://a", "b")
            .unwrap()
            .is_empty());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn refuses_future_and_corrupt_schema_without_overwrite() {
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        assert!(matches!(
            HistoryStore::from_connection(connection),
            Err(StorageError::NewerSchema)
        ));
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        assert!(HistoryStore::from_connection(connection).is_err());
    }
    #[test]
    fn failed_writes_keep_previous_history() {
        let mut store =
            HistoryStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store.append("a", "a", &turn(1)).unwrap();
        store.0.pragma_update(None, "query_only", true).unwrap();
        assert!(store.clear().is_err());
        assert!(store.append("a", "a", &turn(2)).is_err());
        assert_eq!(store.load("a", "a").unwrap(), vec![turn(1)]);
    }
}

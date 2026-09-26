//! Account-scoped coordination storage, deliberately separate from desktop
//! databases. The coordinator authenticates HTTP requests before entering here.
use crate::StorageError;
use companion_core::{
    identity::{AccountProfile, CreateAccountTask, IdentityError, VerifiedIdentity},
    now_ms, Task,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::{path::Path, sync::Mutex, time::Duration};
mod authorization;
mod native_auth;
pub use native_auth::{NativeLogin, NATIVE_ISSUER};

pub struct AccountStore {
    connection: Mutex<Connection>,
    clock: fn() -> u64,
}

impl AccountStore {
    /// The caller is a trusted composition root, never a client-selected path.
    /// Use a NEW dedicated database; existing desktop data has no account owner.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(mut c: Connection) -> Result<Self, StorageError> {
        c.busy_timeout(Duration::from_secs(5))?;
        c.pragma_update(None, "foreign_keys", true)?;
        let version: u32 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 3 {
            return Err(StorageError::NewerSchema);
        }
        // Fail closed on another store's schema rather than adopting local data.
        let app_id: u32 = c.pragma_query_value(None, "application_id", |r| r.get(0))?;
        const APPLICATION_ID: u32 = 0x51424143; // QBAC
        if version == 0 {
            let table_count: u32 = c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0))?;
            if app_id != 0 || table_count != 0 {
                return Err(StorageError::Unavailable);
            }
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(
                "CREATE TABLE accounts (
                   id TEXT PRIMARY KEY NOT NULL,
                   issuer TEXT NOT NULL, subject TEXT NOT NULL,
                   companion_id TEXT NOT NULL UNIQUE,
                   revoked_before INTEGER NOT NULL DEFAULT 0,
                   UNIQUE(issuer,subject));
                 CREATE TABLE account_sessions (
                   account_id TEXT NOT NULL REFERENCES accounts(id),
                   session_id TEXT NOT NULL, authenticated_at INTEGER NOT NULL,
                   revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
                   PRIMARY KEY(account_id,session_id));
                 CREATE TABLE account_tasks (
                   account_id TEXT NOT NULL REFERENCES accounts(id),
                   id TEXT NOT NULL, request_id TEXT NOT NULL, title TEXT NOT NULL,
                   created_at INTEGER NOT NULL, body TEXT NOT NULL,
                   PRIMARY KEY(account_id,id), UNIQUE(account_id,request_id));
                 CREATE INDEX account_tasks_order ON account_tasks(account_id,created_at DESC,id);
                 PRAGMA application_id=1363296579;
                 PRAGMA user_version=1;",
            )?;
            tx.commit()?;
        } else if app_id != APPLICATION_ID {
            return Err(StorageError::Unavailable);
        }
        if version < 2 {
            native_auth::migrate(&mut c)?;
        }
        if version < 3 {
            authorization::migrate(&mut c)?;
        }
        Ok(Self {
            connection: Mutex::new(c),
            clock: now_ms,
        })
    }

    // Every public operation rechecks expiration and durable revocation inside
    // the same write-serialized transaction as the owner-scoped access. There is
    // no reusable account handle and no caller-supplied account ID.
    fn access<T>(
        &self,
        identity: &VerifiedIdentity,
        operation: impl FnOnce(&Transaction<'_>, &AccountProfile, u64) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let mut c = self
            .connection
            .lock()
            .map_err(|_| StorageError::Unavailable)?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = (self.clock)(); // After any mutex/database wait.
        identity.validate_at(now)?;
        let account: Option<(String, String, i64)> = tx.query_row(
            "SELECT id,companion_id,revoked_before FROM accounts WHERE issuer=?1 AND subject=?2",
            params![identity.issuer(), identity.subject()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        ).optional()?;
        let (account_id, companion_id, revoked_before) = match account {
            Some(value) => value,
            None => {
                let account_id = uuid::Uuid::new_v4().to_string();
                let companion_id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO accounts(id,issuer,subject,companion_id) VALUES(?1,?2,?3,?4)",
                    params![
                        account_id,
                        identity.issuer(),
                        identity.subject(),
                        companion_id
                    ],
                )?;
                (account_id, companion_id, 0)
            }
        };
        if revoked_before < 0 || identity.authenticated_at() as i64 <= revoked_before {
            return Err(IdentityError::SessionEnded.into());
        }
        let session: Option<(i64, bool)> = tx.query_row(
            "SELECT authenticated_at,revoked FROM account_sessions WHERE account_id=?1 AND session_id=?2",
            params![account_id, identity.session_id()], |r| Ok((r.get(0)?, r.get(1)?))
        ).optional()?;
        match session {
            Some((authenticated_at, revoked)) => {
                if revoked || authenticated_at != identity.authenticated_at() as i64 {
                    return Err(IdentityError::SessionEnded.into());
                }
            }
            None => {
                tx.execute("INSERT INTO account_sessions(account_id,session_id,authenticated_at) VALUES(?1,?2,?3)",
                    params![account_id, identity.session_id(), identity.authenticated_at() as i64])?;
            }
        }
        let result = operation(
            &tx,
            &AccountProfile {
                account_id,
                companion_id,
            },
            now,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn profile(&self, identity: &VerifiedIdentity) -> Result<AccountProfile, StorageError> {
        self.access(identity, |_, profile, _| Ok(profile.clone()))
    }

    pub fn create_task(
        &self,
        identity: &VerifiedIdentity,
        request: &CreateAccountTask,
    ) -> Result<Task, StorageError> {
        self.access(identity, |tx, profile, now| {
            let request_id = uuid::Uuid::parse_str(&request.request_id)
                .map_err(|_| IdentityError::InvalidRequest)?.to_string();
            let mut task = Task::new(&request.title)?;
            let prior: Option<(String, String)> = tx.query_row(
                "SELECT title,body FROM account_tasks WHERE account_id=?1 AND request_id=?2",
                params![profile.account_id, request_id], |r| Ok((r.get(0)?, r.get(1)?))
            ).optional()?;
            if let Some((title, body)) = prior {
                if title != task.title { return Err(IdentityError::Conflict.into()); }
                return Ok(serde_json::from_str(&body)?);
            }
            let count: u32 = tx.query_row("SELECT COUNT(*) FROM account_tasks WHERE account_id=?1",
                [&profile.account_id], |r| r.get(0))?;
            if count >= 100 { return Err(IdentityError::Capacity.into()); }
            task.created_at = now;
            task.updated_at = now;
            tx.execute("INSERT INTO account_tasks(account_id,id,request_id,title,created_at,body) VALUES(?1,?2,?3,?4,?5,?6)",
                params![profile.account_id, task.id, request_id, task.title, now as i64, serde_json::to_string(&task)?])?;
            Ok(task)
        })
    }

    pub fn list_tasks(&self, identity: &VerifiedIdentity) -> Result<Vec<Task>, StorageError> {
        self.access(identity, |tx, profile, _| {
            let mut query = tx.prepare(
                "SELECT body FROM account_tasks WHERE account_id=?1 ORDER BY created_at DESC,id",
            )?;
            let rows = query.query_map([&profile.account_id], |r| r.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
        })
    }

    pub fn get_task(&self, identity: &VerifiedIdentity, id: &str) -> Result<Task, StorageError> {
        self.access(identity, |tx, profile, _| {
            read_task(tx, &profile.account_id, id)
        })
    }

    pub fn cancel_task(
        &self,
        identity: &VerifiedIdentity,
        id: &str,
        revision: u32,
    ) -> Result<Task, StorageError> {
        self.access(identity, |tx, profile, _| {
            let mut task = read_task(tx, &profile.account_id, id)?;
            if task.revision != revision {
                return Err(IdentityError::Conflict.into());
            }
            task.cancel_queued()?;
            tx.execute(
                "UPDATE account_tasks SET body=?1 WHERE account_id=?2 AND id=?3",
                params![serde_json::to_string(&task)?, profile.account_id, id],
            )?;
            Ok(task)
        })
    }

    /// Local coordinator revocation; provider-wide sign-out is a separate adapter
    /// operation. A refreshed token must keep the original login time/session ID.
    pub fn sign_out(
        &self,
        identity: &VerifiedIdentity,
        all_sessions: bool,
    ) -> Result<(), StorageError> {
        self.access(identity, |tx, profile, now| {
            if all_sessions {
                tx.execute(
                    "UPDATE accounts SET revoked_before=MAX(revoked_before,?1) WHERE id=?2",
                    params![now as i64, profile.account_id],
                )?;
                tx.execute(
                    "UPDATE account_sessions SET revoked=1 WHERE account_id=?1",
                    [&profile.account_id],
                )?;
                tx.execute("UPDATE native_codes SET state='invalid',mac=NULL WHERE state IN ('pending','ready')
                    AND email IN (SELECT u.email FROM native_users u JOIN accounts a ON a.subject=u.subject
                    WHERE a.id=?1 AND a.issuer=?2)", params![profile.account_id,NATIVE_ISSUER])?;
            } else {
                tx.execute(
                    "UPDATE account_sessions SET revoked=1 WHERE account_id=?1 AND session_id=?2",
                    params![profile.account_id, identity.session_id()],
                )?;
            }
            Ok(())
        })
    }
}

fn read_task(tx: &Transaction<'_>, account_id: &str, id: &str) -> Result<Task, StorageError> {
    let body: String = tx
        .query_row(
            "SELECT body FROM account_tasks WHERE account_id=?1 AND id=?2",
            params![account_id, id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(StorageError::NotFound)?;
    Ok(serde_json::from_str(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use companion_core::TaskStatus;

    // Synthetic identities represent the output of a trusted adapter. These
    // tests do not constitute provider authentication or HTTP authorization tests.
    fn identity(issuer: &str, subject: &str, session: &str, login: u64) -> VerifiedIdentity {
        VerifiedIdentity::from_verified_provider(issuer, subject, session, login, 10_000).unwrap()
    }
    fn fixture() -> AccountStore {
        let mut store =
            AccountStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store.clock = || 1000;
        store
    }
    fn request(title: &str) -> CreateAccountTask {
        CreateAccountTask {
            request_id: uuid::Uuid::new_v4().to_string(),
            title: title.into(),
        }
    }
    fn ended<T>(result: Result<T, StorageError>) -> bool {
        matches!(
            result,
            Err(StorageError::Identity(IdentityError::SessionEnded))
        )
    }

    #[test]
    fn two_accounts_cannot_read_cancel_or_list_each_others_tasks() {
        let store = fixture();
        let alice = identity("provider", "alice", "session-a", 100);
        let bob = identity("provider", "bob", "session-b", 100);
        let same_subject_other_issuer = identity("another-provider", "alice", "session-a", 100);
        let profile = store.profile(&alice).unwrap();
        assert_ne!(profile, store.profile(&bob).unwrap());
        assert_ne!(profile, store.profile(&same_subject_other_issuer).unwrap());
        let task = store
            .create_task(&alice, &request("private draft"))
            .unwrap();
        for other in [&bob, &same_subject_other_issuer] {
            assert!(store.list_tasks(other).unwrap().is_empty());
            assert!(matches!(
                store.get_task(other, &task.id),
                Err(StorageError::NotFound)
            ));
            assert!(matches!(
                store.get_task(other, "missing"),
                Err(StorageError::NotFound)
            ));
            assert!(matches!(
                store.cancel_task(other, &task.id, 0),
                Err(StorageError::NotFound)
            ));
        }
        assert_eq!(
            store.get_task(&alice, &task.id).unwrap().status,
            TaskStatus::Queued
        );
        let cancelled = store.cancel_task(&alice, &task.id, 0).unwrap();
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
        assert!(matches!(
            store.cancel_task(&alice, &task.id, 0),
            Err(StorageError::Identity(IdentityError::Conflict))
        ));
        assert_eq!(store.cancel_task(&alice, &task.id, 1).unwrap().revision, 1);
    }

    #[test]
    fn request_dedup_is_payload_bound_and_namespaced_per_account() {
        let store = fixture();
        let alice = identity("provider", "alice", "session", 100);
        let bob = identity("provider", "bob", "session", 100);
        let mut command = request("  first draft  ");
        let first = store.create_task(&alice, &command).unwrap();
        command.title = "first draft".into();
        assert_eq!(store.create_task(&alice, &command).unwrap().id, first.id);
        assert_ne!(store.create_task(&bob, &command).unwrap().id, first.id);
        command.title = "other draft".into();
        assert!(matches!(
            store.create_task(&alice, &command),
            Err(StorageError::Identity(IdentityError::Conflict))
        ));
        assert_eq!(store.list_tasks(&alice).unwrap().len(), 1);
        assert_eq!(store.list_tasks(&bob).unwrap().len(), 1);
    }

    #[test]
    fn single_sign_out_survives_refresh_and_does_not_revoke_another_session() {
        let store = fixture();
        let a = identity("provider", "alice", "session-a", 100);
        let b = identity("provider", "alice", "session-b", 200);
        let profile = store.profile(&a).unwrap();
        assert_eq!(profile, store.profile(&b).unwrap());
        store.sign_out(&a, false).unwrap();
        let refreshed =
            VerifiedIdentity::from_verified_provider("provider", "alice", "session-a", 100, 20_000)
                .unwrap();
        assert!(ended(store.profile(&refreshed)));
        assert!(ended(store.create_task(&a, &request("denied"))));
        assert!(ended(store.list_tasks(&a)));
        assert_eq!(profile, store.profile(&b).unwrap());
        // A provider changing login time on the same session is rejected too.
        assert!(ended(store.profile(&identity(
            "provider",
            "alice",
            "session-a",
            900
        ))));
        assert!(store.list_tasks(&b).unwrap().is_empty());
    }

    #[test]
    fn all_sign_out_rejects_unseen_old_sessions_and_preserves_other_accounts() {
        let mut store = fixture();
        let a = identity("provider", "alice", "session-a", 100);
        let profile = store.profile(&a).unwrap();
        let task = store.create_task(&a, &request("keep after login")).unwrap();
        store.sign_out(&a, true).unwrap();
        // Never registered locally before logout; still cannot enter afterwards.
        assert!(ended(
            store.profile(&identity("provider", "alice", "unseen", 200))
        ));
        assert!(ended(
            store.profile(&identity("provider", "alice", "boundary", 1000))
        ));
        assert!(store
            .profile(&identity("provider", "bob", "session", 100))
            .is_ok());
        store.clock = || 1002;
        let fresh = identity("provider", "alice", "new-login", 1001);
        assert_eq!(store.profile(&fresh).unwrap(), profile);
        assert_eq!(
            store.get_task(&fresh, &task.id).unwrap().title,
            "keep after login"
        );
        assert!(ended(store.get_task(&a, &task.id)));
    }

    #[test]
    fn expired_identity_is_rechecked_for_every_access_without_writes() {
        let mut store = fixture();
        let a =
            VerifiedIdentity::from_verified_provider("provider", "alice", "session-a", 100, 1001)
                .unwrap();
        let task = store.create_task(&a, &request("draft")).unwrap();
        store.clock = || 1001;
        assert!(ended(store.profile(&a)));
        assert!(ended(store.list_tasks(&a)));
        assert!(ended(store.get_task(&a, &task.id)));
        assert!(ended(store.cancel_task(&a, &task.id, 0)));
        assert!(ended(store.create_task(&a, &request("denied"))));
        assert!(ended(store.sign_out(&a, true)));
        let refresh = identity("provider", "alice", "session-a", 100);
        assert_eq!(store.list_tasks(&refresh).unwrap().len(), 1);
        assert_eq!(
            store.get_task(&refresh, &task.id).unwrap().status,
            TaskStatus::Queued
        );
        let changed_login = identity("provider", "alice", "session-a", 101);
        assert!(ended(store.profile(&changed_login)));
    }

    #[test]
    fn separate_connections_observe_revocation_and_reopen_preserves_identity() {
        let path = std::env::temp_dir().join(format!("qiban-accounts-{}.db", uuid::Uuid::new_v4()));
        let a = identity("provider", "alice", "session", 100);
        let profile;
        {
            let mut writer = AccountStore::open(&path).unwrap();
            writer.clock = || 1000;
            let mut reader = AccountStore::open(&path).unwrap();
            reader.clock = || 1000;
            profile = writer.profile(&a).unwrap();
            let task = writer.create_task(&a, &request("durable")).unwrap();
            assert_eq!(reader.get_task(&a, &task.id).unwrap().title, "durable");
            writer.sign_out(&a, true).unwrap();
            assert!(ended(reader.cancel_task(&a, &task.id, 0)));
        }
        {
            let mut reopened = AccountStore::open(&path).unwrap();
            reopened.clock = || 2000;
            assert!(ended(reopened.profile(&a)));
            let fresh = identity("provider", "alice", "new-session", 1001);
            assert_eq!(reopened.profile(&fresh).unwrap(), profile);
            assert_eq!(
                reopened.list_tasks(&fresh).unwrap()[0].status,
                TaskStatus::Queued
            );
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn refuses_newer_or_unrelated_database_without_migrating_local_data() {
        let c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "user_version", 4).unwrap();
        assert!(matches!(
            AccountStore::from_connection(c),
            Err(StorageError::NewerSchema)
        ));
        let path =
            std::env::temp_dir().join(format!("qiban-not-accounts-{}.db", uuid::Uuid::new_v4()));
        {
            let local = crate::TaskStore::open(&path).unwrap();
            local.create("local only").unwrap();
        }
        assert!(matches!(
            AccountStore::open(&path),
            Err(StorageError::Unavailable)
        ));
        {
            let local = crate::TaskStore::open(&path).unwrap();
            assert_eq!(local.list().unwrap()[0].title, "local only");
        }
        std::fs::remove_file(path).unwrap();
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE other(value TEXT);").unwrap();
        assert!(matches!(
            AccountStore::from_connection(c),
            Err(StorageError::Unavailable)
        ));
    }

    #[test]
    fn failed_write_rolls_back_new_account_and_capacity_is_per_owner() {
        let store = fixture();
        let a = identity("provider", "alice", "session-a", 100);
        let b = identity("provider", "bob", "session-b", 100);
        {
            let c = store.connection.lock().unwrap();
            c.execute_batch("CREATE TRIGGER fail_task BEFORE INSERT ON account_tasks BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        }
        assert!(store.create_task(&a, &request("cannot persist")).is_err());
        {
            let c = store.connection.lock().unwrap();
            let count: u32 = c
                .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
            c.execute_batch("DROP TRIGGER fail_task;").unwrap();
        }
        let first = request("first");
        let first_id = store.create_task(&a, &first).unwrap().id;
        for _ in 1..100 {
            store.create_task(&a, &request("draft")).unwrap();
        }
        assert!(matches!(
            store.create_task(&a, &request("full")),
            Err(StorageError::Identity(IdentityError::Capacity))
        ));
        assert_eq!(store.create_task(&a, &first).unwrap().id, first_id);
        assert!(store
            .create_task(&b, &request("room for another account"))
            .is_ok());
    }
}

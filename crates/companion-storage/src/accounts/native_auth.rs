//! Native email-code/session persistence. Only keyed code digests and random
//! session token digests enter the database; plaintext secrets stay at the edge.
use super::*;
use subtle::ConstantTimeEq;

pub const NATIVE_ISSUER: &str = "qiban:self-hosted:v1";
const CODE_TTL: u64 = 10 * 60 * 1000;
const SESSION_TTL: u64 = 24 * 60 * 60 * 1000;
const IDLE_TTL: u64 = 30 * 60 * 1000;
pub struct NativeLogin {
    pub session_id: String,
    pub expires_at: u64,
}

pub(super) fn migrate(c: &mut Connection) -> Result<(), StorageError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE native_users(email TEXT PRIMARY KEY NOT NULL,subject TEXT NOT NULL UNIQUE);
        CREATE TABLE native_codes(id TEXT PRIMARY KEY NOT NULL,email TEXT NOT NULL,mac BLOB,
          expires_at INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,state TEXT NOT NULL);
        CREATE INDEX native_codes_email ON native_codes(email);
        CREATE TABLE native_rate_limits(key TEXT PRIMARY KEY NOT NULL,window_start INTEGER NOT NULL,
          count INTEGER NOT NULL,last_request INTEGER NOT NULL);
        CREATE TABLE native_tokens(hash BLOB PRIMARY KEY NOT NULL,account_id TEXT NOT NULL,
          session_id TEXT NOT NULL,expires_at INTEGER NOT NULL,last_seen INTEGER NOT NULL,
          FOREIGN KEY(account_id,session_id) REFERENCES account_sessions(account_id,session_id));
        PRAGMA user_version=2;",
    )?;
    tx.commit()?;
    Ok(())
}

fn limited(
    tx: &Transaction<'_>,
    key: &str,
    now: i64,
    maximum: u32,
    cooldown: i64,
) -> Result<(), StorageError> {
    let old: Option<(i64, u32, i64)> = tx
        .query_row(
            "SELECT window_start,count,last_request FROM native_rate_limits WHERE key=?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (start, count) = if let Some((start, count, last)) = old {
        if now < last || now - last < cooldown {
            return Err(IdentityError::RateLimited.into());
        }
        if now.saturating_sub(start) < 3_600_000 {
            if count >= maximum {
                return Err(IdentityError::RateLimited.into());
            }
            (start, count + 1)
        } else {
            (now, 1)
        }
    } else {
        (now, 1)
    };
    tx.execute("INSERT INTO native_rate_limits(key,window_start,count,last_request) VALUES(?1,?2,?3,?4)
        ON CONFLICT(key) DO UPDATE SET window_start=excluded.window_start,count=excluded.count,last_request=excluded.last_request",
        params![key,start,count,now])?;
    Ok(())
}

fn recover_login(
    tx: &Transaction<'_>,
    hash: &[u8],
    now: i64,
) -> Result<Option<NativeLogin>, StorageError> {
    let row: Option<(String, i64, i64, bool, i64)> = tx
        .query_row(
            "SELECT s.session_id,t.expires_at,s.authenticated_at,s.revoked,a.revoked_before
            FROM native_tokens t JOIN accounts a ON a.id=t.account_id
            JOIN account_sessions s ON s.account_id=t.account_id AND s.session_id=t.session_id
            WHERE t.hash=?1 AND a.issuer=?2",
            params![hash, NATIVE_ISSUER],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let (session_id, expires_at, authenticated_at, revoked, cutoff) = match row {
        Some(row) => row,
        None => return Ok(None),
    };
    if revoked || authenticated_at <= cutoff || now < authenticated_at || now >= expires_at {
        return Err(IdentityError::SessionEnded.into());
    }
    Ok(Some(NativeLogin {
        session_id,
        expires_at: expires_at as u64,
    }))
}

impl AccountStore {
    pub fn reserve_login_code(
        &self,
        email: &str,
        id: &str,
        mac: &[u8],
    ) -> Result<(), StorageError> {
        if email.is_empty()
            || email.len() > 254
            || mac.len() != 32
            || uuid::Uuid::parse_str(id).is_err()
        {
            return Err(IdentityError::InvalidRequest.into());
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| StorageError::Unavailable)?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = i64::try_from((self.clock)()).map_err(|_| StorageError::InvalidTimestamp)?;
        limited(&tx, "global", now, 30, 0)?;
        limited(&tx, &format!("email:{email}"), now, 5, 60_000)?;
        tx.execute(
            "DELETE FROM native_codes WHERE expires_at<?1",
            [now.saturating_sub(86_400_000)],
        )?;
        tx.execute("UPDATE native_codes SET state='invalid',mac=NULL WHERE email=?1 AND state IN ('pending','ready')",[email])?;
        tx.execute(
            "INSERT INTO native_codes(id,email,mac,expires_at,state) VALUES(?1,?2,?3,?4,'pending')",
            params![
                id,
                email,
                mac,
                now.checked_add(CODE_TTL as i64)
                    .ok_or(StorageError::InvalidTimestamp)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn finish_code_delivery(&self, id: &str, delivered: bool) -> Result<(), StorageError> {
        let c = self
            .connection
            .lock()
            .map_err(|_| StorageError::Unavailable)?;
        let now = i64::try_from((self.clock)()).map_err(|_| StorageError::InvalidTimestamp)?;
        let changed = if delivered {
            c.execute("UPDATE native_codes SET state='ready' WHERE id=?1 AND state='pending' AND expires_at>?2",params![id,now])?
        } else {
            c.execute(
                "UPDATE native_codes SET state='invalid',mac=NULL WHERE id=?1 AND state='pending'",
                [id],
            )?
        };
        if changed != 1 {
            return Err(IdentityError::InvalidCode.into());
        }
        Ok(())
    }

    pub fn login_code_email(&self, id: &str) -> Result<String, StorageError> {
        let c = self
            .connection
            .lock()
            .map_err(|_| StorageError::Unavailable)?;
        c.query_row("SELECT email FROM native_codes WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or(IdentityError::InvalidCode.into())
    }

    pub fn redeem_login_code(
        &self,
        id: &str,
        candidate_mac: &[u8],
        token_hash: &[u8],
    ) -> Result<NativeLogin, StorageError> {
        if candidate_mac.len() != 32 || token_hash.len() != 32 {
            return Err(IdentityError::InvalidCode.into());
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| StorageError::Unavailable)?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = i64::try_from((self.clock)()).map_err(|_| StorageError::InvalidTimestamp)?;
        // Replay recovery: when the original verify response was lost, retrying
        // with the same login nonce must regain the existing session instead of
        // failing on the already-consumed code. Only the nonce holder can hit
        // this row: the derived token hash is a 256-bit secret.
        if let Some(login) = recover_login(&tx, token_hash, now)? {
            return Ok(login);
        }
        type CodeRow = (String, Option<Vec<u8>>, i64, u32, String);
        let row: Option<CodeRow> = tx
            .query_row(
                "SELECT email,mac,expires_at,attempts,state FROM native_codes WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (email, mac, expires, attempts, state) = row.ok_or(IdentityError::InvalidCode)?;
        if state != "ready" || now >= expires || attempts >= 5 {
            return Err(IdentityError::InvalidCode.into());
        }
        let mac = mac.ok_or(IdentityError::InvalidCode)?;
        if !bool::from(mac.as_slice().ct_eq(candidate_mac)) {
            tx.execute("UPDATE native_codes SET attempts=attempts+1,mac=CASE WHEN attempts>=4 THEN NULL ELSE mac END,
                state=CASE WHEN attempts>=4 THEN 'invalid' ELSE state END WHERE id=?1",[id])?;
            tx.commit()?; // Wrong guesses must consume attempts, never roll back.
            return Err(IdentityError::InvalidCode.into());
        }
        let subject: Option<String> = tx
            .query_row(
                "SELECT subject FROM native_users WHERE email=?1",
                [&email],
                |r| r.get(0),
            )
            .optional()?;
        let subject = match subject {
            Some(subject) => subject,
            None => {
                let subject = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO native_users(email,subject) VALUES(?1,?2)",
                    params![email, subject],
                )?;
                subject
            }
        };
        let account: Option<(String, i64)> = tx
            .query_row(
                "SELECT id,revoked_before FROM accounts WHERE issuer=?1 AND subject=?2",
                params![NATIVE_ISSUER, subject],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let account_id = match account {
            Some((id, cutoff)) => {
                if now <= cutoff {
                    return Err(IdentityError::SessionEnded.into());
                }
                id
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO accounts(id,issuer,subject,companion_id) VALUES(?1,?2,?3,?4)",
                    params![id, NATIVE_ISSUER, subject, uuid::Uuid::new_v4().to_string()],
                )?;
                id
            }
        };
        let session_id = uuid::Uuid::new_v4().to_string();
        let expires_at = now
            .checked_add(SESSION_TTL as i64)
            .ok_or(StorageError::InvalidTimestamp)?;
        tx.execute(
            "INSERT INTO account_sessions(account_id,session_id,authenticated_at) VALUES(?1,?2,?3)",
            params![account_id, session_id, now],
        )?;
        tx.execute(
            "DELETE FROM native_tokens WHERE expires_at<?1",
            [now.saturating_sub(86_400_000)],
        )?;
        tx.execute("INSERT INTO native_tokens(hash,account_id,session_id,expires_at,last_seen) VALUES(?1,?2,?3,?4,?5)",
            params![token_hash,account_id,session_id,expires_at,now])?;
        tx.execute(
            "UPDATE native_codes SET state='used',mac=NULL WHERE id=?1",
            [id],
        )?;
        tx.commit()?;
        Ok(NativeLogin {
            session_id,
            expires_at: expires_at as u64,
        })
    }

    pub fn authenticate_native_token(&self, hash: &[u8]) -> Result<VerifiedIdentity, StorageError> {
        if hash.len() != 32 {
            return Err(IdentityError::InvalidSession.into());
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| StorageError::Unavailable)?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = i64::try_from((self.clock)()).map_err(|_| StorageError::InvalidTimestamp)?;
        type TokenRow = (String, String, i64, i64, i64, bool, i64);
        let row: Option<TokenRow> = tx.query_row("SELECT a.subject,s.session_id,s.authenticated_at,t.expires_at,t.last_seen,s.revoked,a.revoked_before
            FROM native_tokens t JOIN accounts a ON a.id=t.account_id
            JOIN account_sessions s ON s.account_id=t.account_id AND s.session_id=t.session_id
            WHERE t.hash=?1 AND a.issuer=?2",params![hash,NATIVE_ISSUER],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
        let (subject, session, login, expiry, last_seen, revoked, cutoff) =
            row.ok_or(IdentityError::InvalidSession)?;
        if revoked
            || login <= cutoff
            || now < login
            || now < last_seen
            || now >= expiry
            || now.saturating_sub(last_seen) >= IDLE_TTL as i64
        {
            return Err(IdentityError::SessionEnded.into());
        }
        let effective_expiry = expiry.min(now.saturating_add(IDLE_TTL as i64));
        let identity = VerifiedIdentity::from_verified_provider(
            NATIVE_ISSUER,
            &subject,
            &session,
            login as u64,
            effective_expiry as u64,
        )?;
        tx.execute(
            "UPDATE native_tokens SET last_seen=?1 WHERE hash=?2",
            params![now, hash],
        )?;
        tx.commit()?;
        Ok(identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> AccountStore {
        let mut store =
            AccountStore::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store.clock = || 100_000;
        store
    }
    #[test]
    fn all_logout_invalidates_pending_email_codes_and_preserves_other_users() {
        let mut store = fixture();
        let id = ready(&store, "a@example.com");
        store.redeem_login_code(&id, &[1; 32], &[2; 32]).unwrap();
        let a = store.authenticate_native_token(&[2; 32]).unwrap();
        let id = ready(&store, "b@example.com");
        store.redeem_login_code(&id, &[1; 32], &[3; 32]).unwrap();
        store.clock = || 160_000;
        let pending = ready(&store, "a@example.com");
        store.sign_out(&a, true).unwrap();
        assert!(store
            .redeem_login_code(&pending, &[1; 32], &[4; 32])
            .is_err());
        assert!(store.authenticate_native_token(&[3; 32]).is_ok());
    }
    #[test]
    fn global_mail_limit_is_durable_and_does_not_overcount_rejected_requests() {
        let store = fixture();
        for n in 0..30 {
            ready(&store, &format!("user{n}@example.com"));
        }
        assert!(matches!(
            store.reserve_login_code(
                "next@example.com",
                &uuid::Uuid::new_v4().to_string(),
                &[1; 32]
            ),
            Err(StorageError::Identity(IdentityError::RateLimited))
        ));
        let c = store.connection.lock().unwrap();
        let count: u32 = c
            .query_row(
                "SELECT count FROM native_rate_limits WHERE key='global'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 30);
    }
    fn ready(store: &AccountStore, email: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        store.reserve_login_code(email, &id, &[1; 32]).unwrap();
        store.finish_code_delivery(&id, true).unwrap();
        id
    }
    #[test]
    fn pending_failed_consumed_and_wrong_attempt_codes_never_authenticate() {
        let store = fixture();
        let id = uuid::Uuid::new_v4().to_string();
        store
            .reserve_login_code("a@example.com", &id, &[1; 32])
            .unwrap();
        assert!(store.redeem_login_code(&id, &[1; 32], &[2; 32]).is_err());
        store.finish_code_delivery(&id, false).unwrap();
        assert!(store.redeem_login_code(&id, &[1; 32], &[2; 32]).is_err());
        let id = ready(&store, "b@example.com");
        for _ in 0..5 {
            assert!(store.redeem_login_code(&id, &[9; 32], &[2; 32]).is_err());
        }
        assert!(store.redeem_login_code(&id, &[1; 32], &[2; 32]).is_err());
        let id = ready(&store, "c@example.com");
        store.redeem_login_code(&id, &[1; 32], &[3; 32]).unwrap();
        assert!(store.redeem_login_code(&id, &[1; 32], &[4; 32]).is_err());
        let c = store.connection.lock().unwrap();
        let mac: Option<Vec<u8>> = c
            .query_row("SELECT mac FROM native_codes WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(mac.is_none());
    }
    #[test]
    fn code_deadlines_and_persistent_email_limits_supersede_old_codes() {
        let mut store = fixture();
        let old = ready(&store, "a@example.com");
        assert!(matches!(
            store.reserve_login_code("a@example.com", &uuid::Uuid::new_v4().to_string(), &[1; 32]),
            Err(StorageError::Identity(IdentityError::RateLimited))
        ));
        store.clock = || 160_000;
        let fresh = ready(&store, "a@example.com");
        assert!(store.redeem_login_code(&old, &[1; 32], &[2; 32]).is_err());
        store.clock = || 760_000; // Exact ten minute boundary.
        assert!(store.redeem_login_code(&fresh, &[1; 32], &[2; 32]).is_err());
        ready(&store, "a@example.com");
        store.clock = || 820_000;
        ready(&store, "a@example.com");
        store.clock = || 880_000;
        ready(&store, "a@example.com");
        store.clock = || 940_000;
        assert!(matches!(
            store.reserve_login_code("a@example.com", &uuid::Uuid::new_v4().to_string(), &[1; 32]),
            Err(StorageError::Identity(IdentityError::RateLimited))
        ));
        assert!(store
            .reserve_login_code("b@example.com", &uuid::Uuid::new_v4().to_string(), &[1; 32])
            .is_ok());
    }
    #[test]
    fn replay_with_same_token_hash_recovers_and_revocation_ends_recovery() {
        let mut store = fixture();
        let id = ready(&store, "a@example.com");
        let first = store.redeem_login_code(&id, &[1; 32], &[7; 32]).unwrap();
        // Retrying the same login nonce regains the existing session even
        // though the code row is already consumed.
        let second = store.redeem_login_code(&id, &[1; 32], &[7; 32]).unwrap();
        assert_eq!(first.session_id, second.session_id);
        assert_eq!(first.expires_at, second.expires_at);
        // A different nonce cannot reuse the consumed code.
        assert!(store.redeem_login_code(&id, &[1; 32], &[8; 32]).is_err());
        // Recovery stops once the underlying session is revoked.
        let identity = store.authenticate_native_token(&[7; 32]).unwrap();
        store.clock = || 160_000;
        store.sign_out(&identity, true).unwrap();
        assert!(store.redeem_login_code(&id, &[1; 32], &[7; 32]).is_err());
    }
    #[test]
    fn hourly_email_window_resets_after_a_full_hour() {
        let mut store = fixture();
        store.clock = || 100_000;
        ready(&store, "a@example.com");
        store.clock = || 161_000;
        ready(&store, "a@example.com");
        store.clock = || 222_000;
        ready(&store, "a@example.com");
        store.clock = || 283_000;
        ready(&store, "a@example.com");
        store.clock = || 344_000;
        ready(&store, "a@example.com");
        store.clock = || 405_000; // Cooldown passed, still inside the hour window.
        assert!(matches!(
            store.reserve_login_code("a@example.com", &uuid::Uuid::new_v4().to_string(), &[1; 32]),
            Err(StorageError::Identity(IdentityError::RateLimited))
        ));
        store.clock = || 3_700_001; // One full hour after window_start 100_000.
        assert!(store
            .reserve_login_code("a@example.com", &uuid::Uuid::new_v4().to_string(), &[1; 32])
            .is_ok());
    }
    #[test]
    fn same_email_keeps_partner_single_logout_and_all_logout_are_immediate() {
        let mut store = fixture();
        let first = ready(&store, "a@example.com");
        store.redeem_login_code(&first, &[1; 32], &[2; 32]).unwrap();
        let a = store.authenticate_native_token(&[2; 32]).unwrap();
        let profile = store.profile(&a).unwrap();
        store.clock = || 160_000;
        let second = ready(&store, "a@example.com");
        store
            .redeem_login_code(&second, &[1; 32], &[3; 32])
            .unwrap();
        let b = store.authenticate_native_token(&[3; 32]).unwrap();
        assert_eq!(profile, store.profile(&b).unwrap());
        store.sign_out(&a, false).unwrap();
        assert!(store.authenticate_native_token(&[2; 32]).is_err());
        assert!(store.profile(&a).is_err()); // Already constructed stale identity.
        assert!(store.authenticate_native_token(&[3; 32]).is_ok());
        store.sign_out(&b, true).unwrap();
        assert!(store.authenticate_native_token(&[3; 32]).is_err());
        store.clock = || 220_000;
        let fresh = ready(&store, "a@example.com");
        store.redeem_login_code(&fresh, &[1; 32], &[4; 32]).unwrap();
        assert_eq!(
            profile,
            store
                .profile(&store.authenticate_native_token(&[4; 32]).unwrap())
                .unwrap()
        );
    }
    #[test]
    fn absolute_idle_timeouts_and_clock_rollback_fail_closed() {
        let mut store = fixture();
        let id = ready(&store, "a@example.com");
        let grant = store.redeem_login_code(&id, &[1; 32], &[2; 32]).unwrap();
        store.clock = || 99_999;
        assert!(store.authenticate_native_token(&[2; 32]).is_err());
        store.clock = || 1_900_000;
        assert!(store.authenticate_native_token(&[2; 32]).is_err());
        store.clock = || 86_500_000;
        {
            let c = store.connection.lock().unwrap();
            c.execute(
                "UPDATE native_tokens SET last_seen=?1",
                [grant.expires_at as i64 - 1],
            )
            .unwrap();
        }
        assert!(store.authenticate_native_token(&[2; 32]).is_err());
    }
    #[test]
    fn failed_session_commit_does_not_consume_code_or_create_user() {
        let store = fixture();
        let id = ready(&store, "a@example.com");
        {
            let c = store.connection.lock().unwrap();
            c.execute_batch("CREATE TRIGGER fail_session BEFORE INSERT ON native_tokens BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        }
        assert!(store.redeem_login_code(&id, &[1; 32], &[2; 32]).is_err());
        {
            let c = store.connection.lock().unwrap();
            let count: u32 = c
                .query_row("SELECT COUNT(*) FROM native_users", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
            c.execute_batch("DROP TRIGGER fail_session;").unwrap();
        }
        assert!(store.redeem_login_code(&id, &[1; 32], &[2; 32]).is_ok());
    }
    #[test]
    fn v1_migration_preserves_accounts_and_revocation_reopens() {
        let path =
            std::env::temp_dir().join(format!("qiban-native-auth-{}.db", uuid::Uuid::new_v4()));
        let identity = VerifiedIdentity::from_verified_provider(
            "old-provider",
            "subject",
            "session",
            100,
            10_000_000,
        )
        .unwrap();
        let old_profile;
        {
            let mut store = AccountStore::open(&path).unwrap();
            store.clock = || 100_000;
            old_profile = store.profile(&identity).unwrap();
            let c = store.connection.lock().unwrap();
            c.execute_batch("DROP TABLE shared_documents;DROP TABLE paired_devices;DROP TABLE device_pairings;DROP TABLE action_authorizations;DROP TABLE authorized_resources;DROP TABLE native_tokens;DROP TABLE native_codes;DROP TABLE native_users;DROP TABLE native_rate_limits;PRAGMA user_version=1;").unwrap();
        }
        {
            let mut store = AccountStore::open(&path).unwrap();
            store.clock = || 100_000;
            assert_eq!(old_profile, store.profile(&identity).unwrap());
            let id = ready(&store, "a@example.com");
            store.redeem_login_code(&id, &[1; 32], &[2; 32]).unwrap();
            let a = store.authenticate_native_token(&[2; 32]).unwrap();
            store.sign_out(&a, true).unwrap();
        }
        {
            let mut store = AccountStore::open(&path).unwrap();
            store.clock = || 100_001;
            assert!(store.authenticate_native_token(&[2; 32]).is_err());
            assert!(matches!(
                store.reserve_login_code(
                    "a@example.com",
                    &uuid::Uuid::new_v4().to_string(),
                    &[1; 32]
                ),
                Err(StorageError::Identity(IdentityError::RateLimited))
            ));
        }
        std::fs::remove_file(path).unwrap();
    }
}

use super::*;
const WINDOW_MS: i64 = 300_000;
const MAX_FAILURES: u32 = 5;

pub(in crate::accounts) fn migrate_short_codes(c: &mut Connection) -> Result<(), StorageError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE pairing_code_limits (
        account_id TEXT PRIMARY KEY NOT NULL REFERENCES accounts(id),
        window_start INTEGER NOT NULL CHECK(window_start>=0), failures INTEGER NOT NULL CHECK(failures BETWEEN 1 AND 5));
        UPDATE device_pairings SET status='revoked',code_hash=NULL,revision=revision+1 WHERE status='pending';
        PRAGMA user_version=5;")?;
    tx.commit()?;
    Ok(())
}
fn digits(random: u32) -> Option<String> {
    // Rejection sampling avoids modulo bias; first four UUID v4 bytes are random bits.
    (random < 4_294_000_000).then(|| format!("{:06}", random % 1_000_000))
}
pub(super) fn new_code(tx: &Transaction<'_>, account: &str) -> Result<String, StorageError> {
    for _ in 0..32 {
        let random = uuid::Uuid::new_v4();
        let Some(code) = digits(u32::from_le_bytes(
            random.as_bytes()[..4].try_into().unwrap(),
        )) else {
            continue;
        };
        // Includes the previous code BEFORE regeneration clears it, and other desktops.
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM device_pairings WHERE account_id=?1 AND code_hash=?2)",
            params![account, hash(&code)?],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(code);
        }
    }
    Err(Error::Capacity.into())
}
/// The inner error is committed by AccountStore::access: wrong-code attempts must
/// survive rejection, process restart, new login and code regeneration.
pub(super) fn guarded_preview(
    tx: &Transaction<'_>,
    account: &str,
    identity: &VerifiedIdentity,
    code: &str,
    expected_id: Option<&str>,
    now: u64,
) -> Result<Result<Pairing, StorageError>, StorageError> {
    let limit: Option<(i64, u32)> = tx
        .query_row(
            "SELECT window_start,failures FROM pairing_code_limits WHERE account_id=?1",
            [account],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let clock = i64::try_from(now).map_err(|_| StorageError::InvalidTimestamp)?;
    if limit.is_some_and(|(start, failures)| {
        start < 0 || clock < start || (clock - start < WINDOW_MS && failures >= MAX_FAILURES)
    }) {
        return Ok(Err(Error::RateLimited.into()));
    }
    let result = preview(tx, account, identity, code, now).and_then(|p| {
        if expected_id.is_some_and(|id| id != p.id) {
            Err(Error::Conflict.into())
        } else {
            Ok(p)
        }
    });
    match result {
        Ok(p) => Ok(Ok(p)),
        Err(e @ StorageError::Authorization(Error::Denied | Error::Conflict | Error::Invalid)) => {
            let (start, failures) = match limit {
                Some((start, failures)) if clock - start < WINDOW_MS => (start, failures + 1),
                _ => (clock, 1),
            };
            tx.execute("INSERT INTO pairing_code_limits VALUES(?1,?2,?3) ON CONFLICT(account_id) DO UPDATE SET window_start=excluded.window_start,failures=excluded.failures", params![account, start, failures])?;
            Ok(Err(if failures >= MAX_FAILURES {
                Error::RateLimited.into()
            } else {
                e
            }))
        }
        Err(e) => Err(e),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uniform_digits_preserve_leading_zero_and_reject_biased_tail() {
        assert_eq!(digits(123), Some("000123".into()));
        assert_eq!(digits(4_293_999_999), Some("999999".into()));
        assert_eq!(digits(4_294_000_000), None);
        assert_eq!(digits(u32::MAX), None);
    }
}

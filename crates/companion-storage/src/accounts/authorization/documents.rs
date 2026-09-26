use super::*;
use companion_core::authorization::{DocumentAction, ShareDocument};
use companion_core::execution::digest;
pub(in crate::accounts) fn migrate_documents(c: &mut Connection) -> Result<(), StorageError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE shared_documents(account_id TEXT NOT NULL,action_id TEXT NOT NULL,source_name TEXT NOT NULL,preview TEXT NOT NULL,PRIMARY KEY(account_id,action_id)); PRAGMA user_version=4;")?;
    tx.commit()?;
    Ok(())
}
fn document(
    tx: &Transaction<'_>,
    account: &str,
    id: &str,
    identity: &VerifiedIdentity,
    now: u64,
) -> Result<DocumentAction, StorageError> {
    let mut authorization = read_action(tx, account, id)?;
    let p = pair(tx, account, &authorization.pairing_id, identity, now)?;
    if p.current_role == "observer" {
        return Err(Error::Denied.into());
    }
    if p.status != "active" || now >= authorization.expires_at {
        authorization.state = invalidated_state(&authorization.state).into();
        save_action(tx, account, &authorization)?;
    }
    let (source_name, preview): (String, String) = tx
        .query_row(
            "SELECT source_name,preview FROM shared_documents WHERE account_id=?1 AND action_id=?2",
            params![account, id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::Denied)?;
    Ok(DocumentAction {
        artifact_hash: digest(preview.as_bytes()),
        authorization,
        source_name,
        preview,
        current_role: p.current_role,
    })
}
impl AccountStore {
    pub fn share_document(
        &self,
        i: &VerifiedIdentity,
        request: &ShareDocument,
    ) -> Result<DocumentAction, StorageError> {
        request.validate()?;
        self.access(i, |tx, a, now| {
            let p = pair(tx, &a.account_id, &request.pairing_id, i, now)?;
            require_active(&p, "desktop")?;
            let b = &request.binding;
            if b.pair_revision != p.revision || b.scope != p.scope {
                return Err(Error::Denied.into());
            }
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM shared_documents WHERE account_id=?1 AND action_id=?2)",
                params![a.account_id, b.action_id], |r| r.get(0),
            )?;
            if exists {
                let old = document(tx, &a.account_id, &b.action_id, i, now)?;
                if old.authorization.binding != *b
                    || old.authorization.pairing_id != request.pairing_id
                    || old.source_name != request.source_name
                    || old.preview != request.preview {
                    return Err(Error::Conflict.into());
                }
                return Ok(old);
            }
            let count: u32 = tx.query_row(
                "SELECT COUNT(*) FROM shared_documents WHERE account_id=?1", [&a.account_id], |r| r.get(0),
            )?;
            let actions: u32 = tx.query_row(
                "SELECT COUNT(*) FROM action_authorizations WHERE account_id=?1", [&a.account_id], |r| r.get(0),
            )?;
            if count >= 20 || actions >= 500 { return Err(Error::Capacity.into()); }
            let authorization = ActionAuthorization {
                pairing_id: p.id, binding: b.clone(),
                expires_at: (now + 300_000).min(i.expires_at()),
                state: "awaiting_confirmation".into(),
            };
            tx.execute("INSERT INTO authorized_resources VALUES(?1,?2,?3,1,1)",
                params![a.account_id, p.desktop_id, b.resource_id])?;
            tx.execute("INSERT INTO action_authorizations VALUES(?1,?2,?3,?4,?5)",
                params![a.account_id, b.action_id, authorization.pairing_id, b.resource_id, serde_json::to_string(&authorization)?])?;
            tx.execute("INSERT INTO shared_documents VALUES(?1,?2,?3,?4)",
                params![a.account_id, b.action_id, request.source_name, request.preview])?;
            document(tx, &a.account_id, &b.action_id, i, now)
        })
    }

    pub fn list_documents(
        &self,
        i: &VerifiedIdentity,
    ) -> Result<Vec<DocumentAction>, StorageError> {
        self.access(i, |tx, a, now| {
            let mut q=tx.prepare("SELECT d.action_id FROM shared_documents d JOIN action_authorizations a ON a.account_id=d.account_id AND a.action_id=d.action_id JOIN device_pairings p ON p.account_id=a.account_id AND p.id=a.pairing_id JOIN paired_devices v ON v.account_id=p.account_id AND (v.id=p.desktop_id OR v.id=p.controller_id) WHERE d.account_id=?1 AND v.session_id=?2 ORDER BY d.rowid DESC")?;
            let ids = q.query_map(params![a.account_id, i.session_id()], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            ids.iter().map(|id| document(tx, &a.account_id, id, i, now)).collect()
        })
    }
    pub fn document_action(
        &self,
        i: &VerifiedIdentity,
        id: &str,
    ) -> Result<DocumentAction, StorageError> {
        self.access(i, |tx, a, now| document(tx, &a.account_id, id, i, now))
    }
    pub fn cancel_document(
        &self,
        i: &VerifiedIdentity,
        id: &str,
    ) -> Result<DocumentAction, StorageError> {
        self.access(i, |tx, a, now| {
            let mut d = document(tx, &a.account_id, id, i, now)?;
            d.authorization.state = invalidated_state(&d.authorization.state).into();
            save_action(tx, &a.account_id, &d.authorization)?;
            Ok(d)
        })
    }
    pub fn receipt_document(
        &self,
        i: &VerifiedIdentity,
        b: &ActionBinding,
        state: &str,
        artifact_hash: &str,
    ) -> Result<DocumentAction, StorageError> {
        if !["completed", "failed", "unknown"].contains(&state) {
            return Err(Error::Invalid.into());
        }
        self.access(i, |tx, a, now| {
            let mut d = document(tx, &a.account_id, &b.action_id, i, now)?;
            if d.current_role != "desktop"
                || d.authorization.binding != *b
                || artifact_hash != d.artifact_hash
            {
                return Err(Error::Denied.into());
            }
            if d.authorization.state == state {
                return Ok(d);
            }
            if !["admitted", "cancel_requested", "unknown"]
                .contains(&d.authorization.state.as_str())
            {
                return Err(Error::Conflict.into());
            }
            d.authorization.state = state.into();
            save_action(tx, &a.account_id, &d.authorization)?;
            Ok(d)
        })
    }
}

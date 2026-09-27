use super::*;
use companion_core::authorization::{
    ActionAuthorization, ActionBinding, ActionScope, AuthorizationError as Error, Pairing,
    PairingOffer,
};
use sha2::{Digest, Sha256};
mod pairing_codes;
pub(super) use pairing_codes::migrate_short_codes;
use pairing_codes::{guarded_preview, new_code};

pub(super) fn migrate(c: &mut Connection) -> Result<(), StorageError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE paired_devices (
        account_id TEXT NOT NULL REFERENCES accounts(id), id TEXT NOT NULL,
        session_id TEXT NOT NULL, name TEXT NOT NULL, expires_at INTEGER NOT NULL,
        PRIMARY KEY(account_id,id), UNIQUE(account_id,session_id));
      CREATE TABLE device_pairings (
        account_id TEXT NOT NULL REFERENCES accounts(id), id TEXT NOT NULL,
        desktop_id TEXT NOT NULL, controller_id TEXT,
        code_hash TEXT, offer_expires INTEGER NOT NULL, revision INTEGER NOT NULL,
        status TEXT NOT NULL,
        PRIMARY KEY(account_id,id));
      CREATE INDEX pairing_code ON device_pairings(account_id,code_hash);
      CREATE TABLE action_authorizations (
        account_id TEXT NOT NULL, action_id TEXT NOT NULL, pairing_id TEXT NOT NULL,
        resource_id TEXT NOT NULL, body TEXT NOT NULL,
        PRIMARY KEY(account_id,action_id));
      CREATE TABLE authorized_resources (
        account_id TEXT NOT NULL, desktop_id TEXT NOT NULL, resource_id TEXT NOT NULL,
        version INTEGER NOT NULL, enabled INTEGER NOT NULL,
        PRIMARY KEY(account_id,desktop_id,resource_id));
      PRAGMA user_version=3;",
    )?;
    tx.commit()?;
    Ok(())
}
fn name(value: &str) -> Result<&str, StorageError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 40 || value.chars().any(char::is_control) {
        return Err(Error::Invalid.into());
    }
    Ok(value)
}
fn hash(code: &str) -> Result<String, StorageError> {
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Denied.into());
    }
    Ok(format!("{:x}", Sha256::digest(code.as_bytes())))
}
fn device(
    tx: &Transaction<'_>,
    account: &str,
    identity: &VerifiedIdentity,
    label: &str,
) -> Result<String, StorageError> {
    let old: Option<String> = tx
        .query_row(
            "SELECT id FROM paired_devices WHERE account_id=?1 AND session_id=?2",
            params![account, identity.session_id()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = old {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO paired_devices VALUES(?1,?2,?3,?4,?5)",
        params![
            account,
            id,
            identity.session_id(),
            name(label)?,
            identity.expires_at() as i64
        ],
    )?;
    Ok(id)
}
fn live(tx: &Transaction<'_>, account: &str, id: &str, now: u64) -> Result<bool, StorageError> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM paired_devices d JOIN account_sessions s ON s.account_id=d.account_id AND s.session_id=d.session_id WHERE d.account_id=?1 AND d.id=?2 AND d.expires_at>?3 AND s.revoked=0 AND (EXISTS(SELECT 1 FROM accounts a WHERE a.id=d.account_id AND a.issuer<>'qiban:self-hosted:v1') OR EXISTS(SELECT 1 FROM native_tokens t WHERE t.account_id=d.account_id AND t.session_id=d.session_id AND t.expires_at>?3 AND t.last_seen<=?3 AND t.last_seen>?3-1800000)))",params![account,id,now as i64],|r|r.get(0))?)
}
fn pair(
    tx: &Transaction<'_>,
    account: &str,
    id: &str,
    identity: &VerifiedIdentity,
    now: u64,
) -> Result<Pairing, StorageError> {
    let row: Option<(Pairing,String,Option<String>)> = tx.query_row("SELECT p.id,p.desktop_id,d.name,p.controller_id,c.name,p.revision,p.status,p.offer_expires,d.session_id,c.session_id FROM device_pairings p JOIN paired_devices d ON d.account_id=p.account_id AND d.id=p.desktop_id LEFT JOIN paired_devices c ON c.account_id=p.account_id AND c.id=p.controller_id WHERE p.account_id=?1 AND p.id=?2",params![account,id],|r| Ok((Pairing {
        id:r.get(0)?,desktop_id:r.get(1)?,desktop_name:r.get(2)?,controller_id:r.get(3)?,controller_name:r.get(4)?,scope:ActionScope::DocumentExcerpt,revision:r.get(5)?,status:r.get(6)?,expires_at:r.get::<_,i64>(7)? as u64,current_role:String::new()
    },r.get(8)?,r.get(9)?))).optional()?;
    let (mut p, desktop, controller) = row.ok_or(Error::Denied)?;
    p.current_role = if desktop == identity.session_id() {
        "desktop"
    } else if controller.as_deref() == Some(identity.session_id()) {
        "controller"
    } else {
        "observer"
    }
    .into();
    if p.status != "revoked"
        && ((!live(tx, account, &p.desktop_id, now)?)
            || (p.status == "pending" && now >= p.expires_at)
            || (p.status == "active"
                && !live(
                    tx,
                    account,
                    p.controller_id.as_deref().ok_or(Error::Denied)?,
                    now,
                )?))
    {
        p.status = "expired".into();
    }
    Ok(p)
}
fn require_active(p: &Pairing, role: &str) -> Result<(), StorageError> {
    if p.status != "active" || p.current_role != role {
        return Err(Error::Denied.into());
    }
    Ok(())
}
impl AccountStore {
    pub fn offer_pairing(
        &self,
        identity: &VerifiedIdentity,
        label: &str,
    ) -> Result<PairingOffer, StorageError> {
        name(label)?;
        self.access(identity, |tx,a,now| {
            let count:u32=tx.query_row("SELECT COUNT(*) FROM device_pairings WHERE account_id=?1",[&a.account_id],|r|r.get(0))?;
            if count >= 100 { return Err(Error::Capacity.into()); }
            let desktop = device(tx,&a.account_id,identity,label)?;
            let code = new_code(tx, &a.account_id)?;
            // Regeneration invalidates all earlier codes from this session.
            tx.execute("UPDATE device_pairings SET status='revoked',code_hash=NULL,revision=revision+1 WHERE account_id=?1 AND desktop_id=?2 AND status='pending'",params![a.account_id,desktop])?;
            let id=uuid::Uuid::new_v4().to_string();
            tx.execute("INSERT INTO device_pairings VALUES(?1,?2,?3,NULL,?4,?5,1,'pending')",params![a.account_id,id,desktop,hash(&code)?,(now+300_000).min(identity.expires_at()) as i64])?;
            Ok(PairingOffer { pairing:pair(tx,&a.account_id,&id,identity,now)?,code })
        })
    }
    pub fn preview_pairing(
        &self,
        identity: &VerifiedIdentity,
        code: &str,
    ) -> Result<Pairing, StorageError> {
        self.access(identity, |tx, a, now| {
            guarded_preview(tx, &a.account_id, identity, code, None, now)
        })?
    }
    pub fn accept_pairing(
        &self,
        identity: &VerifiedIdentity,
        code: &str,
        expected_id: &str,
        label: &str,
    ) -> Result<Pairing, StorageError> {
        name(label)?;
        self.access(identity, |tx, a, now| {
            let p = match guarded_preview(tx, &a.account_id, identity, code, Some(expected_id), now)? {
                Ok(p) => p,
                Err(e) => return Ok(Err(e)),
            };
            let controller = device(tx, &a.account_id, identity, label)?;
            tx.execute("UPDATE device_pairings SET controller_id=?1,status='active',revision=revision+1,code_hash=NULL WHERE account_id=?2 AND id=?3", params![controller, a.account_id, p.id])?;
            Ok(Ok(pair(tx, &a.account_id, &p.id, identity, now)?))
        })?
    }

    pub fn list_pairings(&self, identity: &VerifiedIdentity) -> Result<Vec<Pairing>, StorageError> {
        self.access(identity, |tx, a, now| {
            let mut q = tx.prepare(
                "SELECT id FROM device_pairings WHERE account_id=?1 ORDER BY rowid DESC",
            )?;
            let ids = q
                .query_map([&a.account_id], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            ids.iter()
                .map(|id| pair(tx, &a.account_id, id, identity, now))
                .collect()
        })
    }
    pub fn revoke_pairing(
        &self,
        identity: &VerifiedIdentity,
        id: &str,
        revision: u32,
    ) -> Result<Pairing, StorageError> {
        self.access(identity,|tx,a,now| {
            let p=pair(tx,&a.account_id,id,identity,now)?;
            if p.revision!=revision { return Err(Error::Conflict.into()); }
            if p.status!="revoked" {
                tx.execute("UPDATE device_pairings SET status='revoked',code_hash=NULL,revision=revision+1 WHERE account_id=?1 AND id=?2",params![a.account_id,id])?;
                invalidate(tx,&a.account_id,"pairing_id",id)?;
            }
            pair(tx,&a.account_id,id,identity,now)
        })
    }
    /// Trusted executor adapter only. A concrete local resource replaces an arbitrary path.
    pub fn set_authorized_resource(
        &self,
        identity: &VerifiedIdentity,
        pair_id: &str,
        resource_id: &str,
        version: u32,
        enabled: bool,
    ) -> Result<(), StorageError> {
        if uuid::Uuid::parse_str(resource_id).is_err() || version == 0 {
            return Err(Error::Invalid.into());
        }
        self.access(identity,|tx,a,now| {
            let p=pair(tx,&a.account_id,pair_id,identity,now)?; require_active(&p,"desktop")?;
            let old:Option<u32>=tx.query_row("SELECT version FROM authorized_resources WHERE account_id=?1 AND desktop_id=?2 AND resource_id=?3",params![a.account_id,p.desktop_id,resource_id],|r|r.get(0)).optional()?;
            if old.is_some_and(|old|version<=old) {return Err(Error::Conflict.into());}
            tx.execute("INSERT INTO authorized_resources VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account_id,desktop_id,resource_id) DO UPDATE SET version=excluded.version,enabled=excluded.enabled",params![a.account_id,p.desktop_id,resource_id,version,enabled])?;
            invalidate(tx,&a.account_id,"resource_id",resource_id)?;
            Ok(())
        })
    }
    pub fn prepare_action(
        &self,
        identity: &VerifiedIdentity,
        pair_id: &str,
        binding: &ActionBinding,
    ) -> Result<ActionAuthorization, StorageError> {
        binding.validate()?;
        self.access(identity,|tx,a,now| {
            let p=pair(tx,&a.account_id,pair_id,identity,now)?; require_active(&p,"desktop")?;
            check_resource(tx,&a.account_id,&p,binding)?;
            let count:u32=tx.query_row("SELECT COUNT(*) FROM action_authorizations WHERE account_id=?1",[&a.account_id],|r|r.get(0))?;
            if count>=500 {return Err(Error::Capacity.into());}
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM action_authorizations WHERE account_id=?1 AND action_id=?2)",params![a.account_id,binding.action_id],|r|r.get(0))?;
            if exists {return Err(Error::Conflict.into());}
            let action=ActionAuthorization{pairing_id:pair_id.into(),binding:binding.clone(),expires_at:(now+300_000).min(identity.expires_at()),state:"awaiting_confirmation".into()};
            tx.execute("INSERT INTO action_authorizations VALUES(?1,?2,?3,?4,?5)",params![a.account_id,binding.action_id,pair_id,binding.resource_id,serde_json::to_string(&action)?])?;
            Ok(action)
        })
    }
    pub fn confirm_action(
        &self,
        identity: &VerifiedIdentity,
        binding: &ActionBinding,
    ) -> Result<ActionAuthorization, StorageError> {
        self.advance_action(
            identity,
            binding,
            "controller",
            "awaiting_confirmation",
            "confirmed",
        )
    }
    /// One-time admission, not an execution receipt. A retry after admission is rejected.
    pub fn admit_action(
        &self,
        identity: &VerifiedIdentity,
        binding: &ActionBinding,
    ) -> Result<ActionAuthorization, StorageError> {
        self.advance_action(identity, binding, "desktop", "confirmed", "admitted")
    }
    fn advance_action(
        &self,
        identity: &VerifiedIdentity,
        binding: &ActionBinding,
        role: &str,
        from: &str,
        to: &str,
    ) -> Result<ActionAuthorization, StorageError> {
        binding.validate()?;
        self.access(identity, |tx, a, now| {
            let mut action = read_action(tx, &a.account_id, &binding.action_id)?;
            if action.binding != *binding || action.state != from || now >= action.expires_at {
                return Err(Error::Conflict.into());
            }
            let p = pair(tx, &a.account_id, &action.pairing_id, identity, now)?;
            require_active(&p, role)?;
            check_resource(tx, &a.account_id, &p, binding)?;
            action.state = to.into();
            save_action(tx, &a.account_id, &action)?;
            Ok(action)
        })
    }
    pub fn get_action_authorization(
        &self,
        identity: &VerifiedIdentity,
        id: &str,
    ) -> Result<ActionAuthorization, StorageError> {
        self.access(identity, |tx, a, now| {
            let mut action = read_action(tx, &a.account_id, id)?;
            let p = pair(tx, &a.account_id, &action.pairing_id, identity, now)?;
            if p.current_role == "observer" {
                return Err(Error::Denied.into());
            }
            if p.status != "active" || now >= action.expires_at {
                action.state = invalidated_state(&action.state).into();
                save_action(tx, &a.account_id, &action)?;
            }
            Ok(action)
        })
    }
}
fn preview(
    tx: &Transaction<'_>,
    account: &str,
    identity: &VerifiedIdentity,
    code: &str,
    now: u64,
) -> Result<Pairing, StorageError> {
    let id: Option<String> = tx
        .query_row(
            "SELECT id FROM device_pairings WHERE account_id=?1 AND code_hash=?2",
            params![account, hash(code)?],
            |r| r.get(0),
        )
        .optional()?;
    let p = pair(tx, account, &id.ok_or(Error::Denied)?, identity, now)?;
    if p.status != "pending" || p.current_role == "desktop" {
        return Err(Error::Denied.into());
    }
    Ok(p)
}
fn check_resource(
    tx: &Transaction<'_>,
    account: &str,
    p: &Pairing,
    b: &ActionBinding,
) -> Result<(), StorageError> {
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM authorized_resources WHERE account_id=?1 AND desktop_id=?2 AND resource_id=?3 AND version=?4 AND enabled=1)",params![account,p.desktop_id,b.resource_id,b.resource_version],|r|r.get(0))?;
    if !valid || b.pair_revision != p.revision || b.scope != p.scope {
        return Err(Error::Denied.into());
    }
    Ok(())
}
fn read_action(
    tx: &Transaction<'_>,
    account: &str,
    id: &str,
) -> Result<ActionAuthorization, StorageError> {
    let body: Option<String> = tx
        .query_row(
            "SELECT body FROM action_authorizations WHERE account_id=?1 AND action_id=?2",
            params![account, id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&body.ok_or(Error::Denied)?)?)
}
fn save_action(
    tx: &Transaction<'_>,
    account: &str,
    a: &ActionAuthorization,
) -> Result<(), StorageError> {
    tx.execute(
        "UPDATE action_authorizations SET body=?1 WHERE account_id=?2 AND action_id=?3",
        params![serde_json::to_string(a)?, account, a.binding.action_id],
    )?;
    Ok(())
}
fn invalidated_state(state: &str) -> &str {
    match state {
        "admitted" | "cancel_requested" => "cancel_requested",
        "completed" | "failed" | "unknown" => state,
        _ => "cancelled",
    }
}
fn invalidate(
    tx: &Transaction<'_>,
    account: &str,
    column: &str,
    id: &str,
) -> Result<(), StorageError> {
    // column is selected by internal callers only, never deserialized from a request.
    let mut q = tx.prepare(&format!(
        "SELECT body FROM action_authorizations WHERE account_id=?1 AND {column}=?2"
    ))?;
    let rows = q
        .query_map(params![account, id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for body in rows {
        let mut a: ActionAuthorization = serde_json::from_str(&body)?;
        a.state = invalidated_state(&a.state).into();
        save_action(tx, account, &a)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests;

mod documents;
pub(super) use documents::migrate_documents;

//! Device-local credentials. Account logic depends on this interface, not Win32.
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionSecret {
    pub token: String,
    // Persist before attempting revocation. Never use a pending token for data.
    pub pending_logout: Option<bool>,
}
impl Drop for SessionSecret {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}
pub trait SessionVault: Send + Sync {
    fn read(&self, scope: &str) -> Result<Option<SessionSecret>, ()>;
    fn write(&self, scope: &str, session: &SessionSecret) -> Result<(), ()>;
    fn delete(&self, scope: &str) -> Result<(), ()>;
}
pub struct SystemVault;

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{Foundation::ERROR_NOT_FOUND, Security::Credentials::*},
    };
    use zeroize::Zeroizing;
    fn target(scope: &str) -> Vec<u16> {
        format!("dev.qiban.companion/account/{scope}")
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }
    impl SessionVault for SystemVault {
        fn read(&self, scope: &str) -> Result<Option<SessionSecret>, ()> {
            let name = target(scope);
            let mut credential = std::ptr::null_mut();
            unsafe {
                if let Err(e) = CredReadW(
                    PCWSTR(name.as_ptr()),
                    CRED_TYPE_GENERIC,
                    None,
                    &mut credential,
                ) {
                    return if e.code() == windows::core::HRESULT::from_win32(ERROR_NOT_FOUND.0) {
                        Ok(None)
                    } else {
                        Err(())
                    };
                }
                let size = (*credential).CredentialBlobSize as usize;
                let value = if size == 0 || size > 2048 {
                    Err(())
                } else {
                    let bytes = Zeroizing::new(
                        std::slice::from_raw_parts((*credential).CredentialBlob, size).to_vec(),
                    );
                    serde_json::from_slice(&bytes).map(Some).map_err(|_| ())
                };
                CredFree(credential.cast());
                value
            }
        }
        fn write(&self, scope: &str, session: &SessionSecret) -> Result<(), ()> {
            let mut name = target(scope);
            let secret = Zeroizing::new(serde_json::to_vec(session).map_err(|_| ())?);
            if secret.len() > 2048 {
                return Err(());
            }
            let credential = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(name.as_mut_ptr()),
                CredentialBlobSize: secret.len() as u32,
                CredentialBlob: secret.as_ptr() as *mut u8,
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                ..Default::default()
            };
            unsafe { CredWriteW(&credential, 0) }.map_err(|_| ())
        }
        fn delete(&self, scope: &str) -> Result<(), ()> {
            let name = target(scope);
            match unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) } {
                Ok(()) => Ok(()),
                Err(e) if e.code() == windows::core::HRESULT::from_win32(ERROR_NOT_FOUND.0) => {
                    Ok(())
                }
                Err(_) => Err(()),
            }
        }
    }
}
#[cfg(not(windows))]
impl SessionVault for SystemVault {
    fn read(&self, _: &str) -> Result<Option<SessionSecret>, ()> {
        Err(())
    }
    fn write(&self, _: &str, _: &SessionSecret) -> Result<(), ()> {
        Err(())
    }
    fn delete(&self, _: &str) -> Result<(), ()> {
        Err(())
    }
}

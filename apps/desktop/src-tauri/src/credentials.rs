#[cfg(windows)]
mod platform {
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{
            Foundation::{ERROR_CANCELLED, ERROR_NOT_FOUND, HWND},
            Security::Credentials::*,
        },
    };
    use zeroize::Zeroizing;
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn target(base: &str) -> Vec<u16> {
        wide(&format!("dev.qiban.companion/model/{base}"))
    }
    pub fn read(base: &str) -> Result<Option<Zeroizing<String>>, String> {
        let target = target(base);
        let mut credential = std::ptr::null_mut();
        unsafe {
            if let Err(e) = CredReadW(
                PCWSTR(target.as_ptr()),
                CRED_TYPE_GENERIC,
                None,
                &mut credential,
            ) {
                if e.code() == windows::core::HRESULT::from_win32(ERROR_NOT_FOUND.0) {
                    return Ok(None);
                }
                return Err("无法读取系统凭据存储".into());
            }
            let size = (*credential).CredentialBlobSize as usize;
            let result = if size == 0 {
                Ok(None)
            } else {
                std::str::from_utf8(std::slice::from_raw_parts(
                    (*credential).CredentialBlob,
                    size,
                ))
                .map(|s| Some(Zeroizing::new(s.to_owned())))
                .map_err(|_| "系统密钥编码无效".into())
            };
            CredFree(credential.cast());
            result
        }
    }
    fn write(base: &str, key: &str) -> Result<(), String> {
        let mut target = target(base);
        let mut user = wide("API Key");
        let secret = Zeroizing::new(key.as_bytes().to_vec());
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            CredentialBlobSize: secret.len() as u32,
            CredentialBlob: secret.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: PWSTR(user.as_mut_ptr()),
            ..Default::default()
        };
        unsafe { CredWriteW(&credential, 0) }.map_err(|_| "无法保存到Windows凭据管理器".into())
    }
    pub fn delete(base: &str) -> Result<(), String> {
        let target = target(base);
        match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(()),
            Err(e) if e.code() == windows::core::HRESULT::from_win32(ERROR_NOT_FOUND.0) => Ok(()),
            Err(_) => Err("删除系统密钥失败".into()),
        }
    }
    pub fn prompt(base: &str, parent: usize) -> Result<(), String> {
        let target = target(base);
        let caption = wide("栖伴 · 设置模型 API Key");
        let message = wide(&format!(
            "将 API Key 填入密码栏。密钥仅用于：{base}\n用户名保持 API Key 即可。"
        ));
        let mut username = vec![0u16; 128];
        let initial = wide("API Key");
        username[..initial.len()].copy_from_slice(&initial);
        let mut password = Zeroizing::new(vec![0u16; 513]);
        let info = CREDUI_INFOW {
            cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
            hwndParent: HWND(parent as *mut _),
            pszMessageText: PCWSTR(message.as_ptr()),
            pszCaptionText: PCWSTR(caption.as_ptr()),
            ..Default::default()
        };
        let result = unsafe {
            CredUIPromptForCredentialsW(
                Some(&info),
                PCWSTR(target.as_ptr()),
                None,
                0,
                &mut username,
                &mut password,
                None,
                CREDUI_FLAGS_GENERIC_CREDENTIALS
                    | CREDUI_FLAGS_ALWAYS_SHOW_UI
                    | CREDUI_FLAGS_DO_NOT_PERSIST,
            )
        };
        if result == ERROR_CANCELLED {
            return Err("已取消设置密钥".into());
        }
        if result.0 != 0 {
            return Err("无法打开系统密钥输入窗口".into());
        }
        let length = password
            .iter()
            .position(|v| *v == 0)
            .unwrap_or(password.len());
        let key =
            Zeroizing::new(String::from_utf16(&password[..length]).map_err(|_| "密钥编码无效")?);
        if key.trim().is_empty() {
            return Err("API Key不能为空".into());
        }
        write(base, key.trim())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        #[ignore = "Explicit local Windows credential-store integration test"]
        fn credential_is_scoped_and_can_be_removed() {
            let base = format!("https://qiban-test.invalid/{}", uuid::Uuid::new_v4());
            write(&base, "test-only-not-a-real-key").unwrap();
            let result = read(&base).unwrap();
            let other = read(&(base.clone() + "/other")).unwrap();
            delete(&base).unwrap();
            assert_eq!(
                result.as_deref().map(|s| s.as_str()),
                Some("test-only-not-a-real-key")
            );
            assert!(other.is_none());
            assert!(read(&base).unwrap().is_none());
        }
    }
}
#[cfg(windows)]
pub use platform::*;
#[cfg(not(windows))]
pub fn read(_: &str) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    Ok(None)
}
#[cfg(not(windows))]
pub fn delete(_: &str) -> Result<(), String> {
    Err("当前仅支持Windows凭据存储".into())
}

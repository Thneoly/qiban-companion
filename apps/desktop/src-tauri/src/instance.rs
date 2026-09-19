//! The plugin restores the existing pet. This OS-held file lease additionally closes
//! startup races (or a plugin IPC failure) before any application database is opened.
use std::{fs::File, path::Path};

pub struct DataLease {
    _file: File,
}
impl DataLease {
    #[cfg(windows)]
    pub fn acquire(directory: &Path) -> std::io::Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        // Never unlink: deleting a live lock file could admit another writer. Windows
        // releases the exclusive handle on normal exit AND process termination.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(directory.join("instance.lock"))?;
        Ok(Self { _file: file })
    }
    #[cfg(not(windows))]
    pub fn acquire(_directory: &Path) -> std::io::Result<Self> {
        Err(std::io::Error::other(
            "此构建尚未实现当前平台的数据目录排他锁",
        ))
    }
}

pub fn report_unavailable() {
    #[cfg(windows)]
    unsafe {
        use windows::{
            core::w,
            Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK},
        };
        MessageBoxW(None, w!("无法独占本地资料目录。栖伴可能已在运行，请从系统托盘恢复角色；若未运行，请检查本地目录权限。此进程未打开数据库。"), w!("栖伴启动提示"), MB_OK | MB_ICONINFORMATION);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::process::Command;
    #[test]
    fn lease_child() {
        let Some(path) = std::env::var_os("QIBAN_LEASE_TEST_PATH") else {
            return;
        };
        let result = DataLease::acquire(Path::new(&path));
        assert_eq!(
            result.is_ok(),
            std::env::var_os("QIBAN_LEASE_EXPECT_FREE").is_some()
        );
    }
    #[test]
    fn q6_del20_blocks_other_process_and_releases_with_profile_isolation() {
        let directory = std::env::temp_dir().join(format!("qiban-lease-{}", uuid::Uuid::new_v4()));
        let other = directory.join("acceptance");
        std::fs::create_dir_all(&other).unwrap();
        let lease = DataLease::acquire(&directory).unwrap();
        let run = |path: &Path, free: bool| {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "instance::tests::lease_child", "--nocapture"])
                .env("QIBAN_LEASE_TEST_PATH", path)
                .env_remove("QIBAN_LEASE_EXPECT_FREE");
            if free {
                command.env("QIBAN_LEASE_EXPECT_FREE", "1");
            }
            assert!(command.output().unwrap().status.success());
        };
        run(&directory, false);
        run(&other, true);
        drop(lease);
        run(&directory, true);
        std::fs::remove_file(directory.join("instance.lock")).unwrap();
        std::fs::remove_file(other.join("instance.lock")).unwrap();
        std::fs::remove_dir(other).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}

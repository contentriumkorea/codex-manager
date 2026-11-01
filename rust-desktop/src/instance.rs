use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub struct LegacyLock {
    file: Option<File>,
    path: PathBuf,
    identity: Option<String>,
}
fn running(pid: u32) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(handle) => {
                let mut exit = 259;
                let result = GetExitCodeProcess(handle, &mut exit);
                let _ = CloseHandle(handle);
                result.is_err() || exit == 259
            }
            Err(error) => error.code().0 as u32 != 0x80070057,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        true
    }
}
impl LegacyLock {
    pub fn acquire(state: &Path) -> Result<Self, String> {
        let path = state.join("application.lock");
        codex_manager_core::absolute(&path)?;
        if path.exists() {
            let mut probe_options = OpenOptions::new();
            probe_options.read(true).write(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                probe_options.share_mode(0);
            }
            let mut probe = probe_options.open(&path).map_err(|_| {
                "기존 Codex Manager가 실행 중이거나 잠금 파일을 확인할 수 없습니다."
            })?;
            let identity = codex_manager_core::file_identity(&probe)?;
            if probe.metadata().map_err(|e| e.to_string())?.len() > 16384 {
                return Err(
                    "기존 Manager 잠금 파일이 예상 형식보다 큽니다. 설정 폴더를 확인하세요.".into(),
                );
            }
            let mut bytes = Vec::new();
            probe.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            let pid = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|s| s.lines().next())
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|id| *id > 0);
            if pid.is_some_and(running) {
                return Err("기존 Codex Manager를 완전히 종료한 뒤 새 버전을 실행하세요.".into());
            }
            if pid.is_none() {
                // Preserve an interrupted, inactive lock before replacing it.
                let backup = state.join(format!(
                    "application.lock.interrupted-{}",
                    uuid::Uuid::new_v4()
                ));
                let mut copy = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(backup)
                    .map_err(|e| e.to_string())?;
                copy.write_all(&bytes).map_err(|e| e.to_string())?;
                copy.sync_all().map_err(|e| e.to_string())?;
            }
            drop(probe);
            if codex_manager_core::identity(&path)? != identity
                || fs::read(&path).map_err(|e| e.to_string())? != bytes
            {
                return Err("Manager 실행 상태가 변경됐습니다. 다시 실행하세요.".into());
            }
            fs::remove_file(&path).map_err(|_| "기존 Manager 실행 잠금을 해제할 수 없습니다.")?;
        }
        let mut options = OpenOptions::new();
        options.create_new(true).write(true).read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options
            .open(&path)
            .map_err(|_| "같은 설정을 사용하는 Codex Manager가 이미 실행 중입니다.")?;
        let mut owned = Self {
            file: Some(file),
            path,
            identity: None,
        };
        // Establish ownership before any fallible initialization writes.
        owned.identity = Some(codex_manager_core::file_identity(
            owned.file.as_ref().unwrap(),
        )?);
        let file = owned.file.as_mut().unwrap();
        writeln!(
            file,
            "{}\nCodexManager\n{}",
            std::process::id(),
            std::env::var("COMPUTERNAME").unwrap_or_default()
        )
        .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        Ok(owned)
    }
}
impl Drop for LegacyLock {
    fn drop(&mut self) {
        drop(self.file.take());
        if self.identity.as_ref().is_some_and(|expected| {
            codex_manager_core::identity(&self.path).is_ok_and(|id| &id == expected)
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn legacy_lock_blocks_coexistence_and_removes_only_its_own_file() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let lock = LegacyLock::acquire(&root).unwrap();
        assert!(LegacyLock::acquire(&root).is_err());
        drop(lock);
        assert!(!root.join("application.lock").exists());
        fs::write(
            root.join("application.lock"),
            "2147483647\nCodexManager\nfixture\n",
        )
        .unwrap();
        let lock = LegacyLock::acquire(&root).unwrap();
        drop(lock);
        assert!(!root.join("application.lock").exists());
    }
    #[test]
    fn interrupted_empty_and_partial_locks_are_preserved_then_recovered() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        for bytes in [b"".as_slice(), b"0", b"partial\n", b"\xff"] {
            fs::write(root.join("application.lock"), bytes).unwrap();
            let lock = LegacyLock::acquire(&root).unwrap();
            drop(lock);
            assert!(!root.join("application.lock").exists());
        }
        assert_eq!(fs::read_dir(root).unwrap().count(), 4);
    }
}

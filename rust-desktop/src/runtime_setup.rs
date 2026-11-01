use std::path::Path;
pub fn prepare(runtime: &Path, state: &Path) -> Result<(), String> {
    codex_manager_core::absolute(runtime)?;
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Version {
            size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform: u32,
            description: [u16; 128],
        }
        #[link(name = "ntdll")]
        extern "system" {
            fn RtlGetVersion(version: *mut Version) -> i32;
        }
        let mut version = Version {
            size: std::mem::size_of::<Version>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            description: [0; 128],
        };
        if unsafe { RtlGetVersion(&mut version) } < 0 {
            return Err("Windows 실행 환경을 확인할 수 없습니다.".into());
        }
        if version.major == 10 && version.build < 22000 {
            use sha2::{Digest, Sha256};
            let hash = format!("{:x}", Sha256::digest(runtime.to_string_lossy().as_bytes()));
            let marker = state.join(format!("webview2-permissions-{}.json", &hash[..16]));
            let executable = runtime.join("msedgewebview2.exe");
            let stamp = std::fs::metadata(&executable)
                .map_err(|e| e.to_string())?
                .modified()
                .map_err(|e| e.to_string())?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs();
            let expected = serde_json::json!({"runtime":runtime,"modified":stamp});
            if std::fs::read(&marker)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .as_ref()
                != Some(&expected)
            {
                let binary = std::path::PathBuf::from(
                    std::env::var_os("SystemRoot").ok_or("Windows 폴더 누락")?,
                )
                .join("System32/icacls.exe");
                codex_manager_core::command_output(
                    std::process::Command::new(binary).arg(runtime).args([
                        "/grant",
                        "*S-1-15-2-1:(OI)(CI)(RX)",
                        "*S-1-15-2-2:(OI)(CI)(RX)",
                        "/T",
                        "/C",
                        "/Q",
                    ]),
                )?;
                let temporary = state.join(format!(
                    ".runtime-permissions-{}.json",
                    uuid::Uuid::new_v4()
                ));
                codex_manager_core::write_json_new(&temporary, &expected)?;
                std::fs::rename(temporary, marker).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

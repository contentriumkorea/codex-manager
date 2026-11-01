use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub fn command_output(command: &mut Command) -> Result<String, String> {
    command.env_remove("PSModulePath");
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env(
            "PSModulePath",
            PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/Modules"),
        );
    }
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or("프로세스 출력을 열 수 없습니다.")?;
    let (sender, output) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut s = String::new();
        let result = stdout
            .take(1024 * 1024)
            .read_to_string(&mut s)
            .map(|_| s)
            .map_err(|e| e.to_string());
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let out = output
                    .recv_timeout(remaining)
                    .map_err(|_| "프로세스 출력 시간이 초과됐습니다.")??;
                return if status.success() {
                    Ok(out.trim().to_owned())
                } else {
                    Err("명령 실행에 실패했습니다.".into())
                };
            }
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("명령 응답 시간이 초과됐습니다.".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub fn desktop_closed() -> Result<(), String> {
    desktop_closed_except(&[])
}
pub fn desktop_closed_except(owned: &[u32]) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
            System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
        };
        struct Snapshot(HANDLE);
        impl Drop for Snapshot {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if raw == INVALID_HANDLE_VALUE {
            return Err("Codex 실행 상태를 확인할 수 없습니다.".into());
        }
        let snapshot = Snapshot(raw);
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) };
        while found != 0 {
            let end = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if name.eq_ignore_ascii_case("codex.exe") && !owned.contains(&entry.th32ProcessID) {
                return Err("Codex 앱과 CLI 작업을 완전히 종료한 뒤 변경하세요. 조회와 백업은 실행 중에도 가능합니다.".into());
            }
            found = unsafe { Process32NextW(snapshot.0, &mut entry) };
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(18) {
            return Err("Codex 프로세스 목록을 확인할 수 없습니다.".into());
        }
    }
    Ok(())
}
pub fn find_codex() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let location=command_output(Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command","(Get-AppxPackage -Name OpenAI.Codex | Sort-Object Version -Descending | Select-Object -First 1).InstallLocation"]));
        if let Ok(location) = location {
            if !location.is_empty() {
                let binary = PathBuf::from(location).join("app/resources/codex.exe");
                if binary.is_file() {
                    return Ok(binary);
                }
            }
        }
        if let Some(user) = std::env::var_os("LOCALAPPDATA") {
            let binary = PathBuf::from(user).join("Programs/OpenAI/Codex/bin/codex.exe");
            if binary.is_file() {
                return Ok(binary);
            }
        }
    }
    Err("현재 Codex 앱 실행 파일을 찾을 수 없습니다. Codex 앱을 설치하거나 설정에서 실행 파일을 지정하세요.".into())
}
pub fn check_version(binary: &Path) -> Result<String, String> {
    if !binary.is_file() {
        return Err("Codex 실행 파일을 찾을 수 없습니다. Codex 앱을 설치하거나 실행 파일 위치를 지정하세요.".into());
    }
    let version = command_output(Command::new(binary).arg("--version"))?;
    let text = version
        .split_whitespace()
        .nth(1)
        .ok_or("Codex 버전을 확인할 수 없습니다.")?;
    let mut parts = text.split(['.', '-']);
    let major = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or("Codex 버전 오류")?;
    let minor = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or("Codex 버전 오류")?;
    if (major, minor) < (0, 160) {
        return Err(format!(
            "Codex 앱을 업데이트하세요. 변경 기능에는 0.160 이상이 필요합니다. 현재: {text}"
        ));
    }
    Ok(version)
}

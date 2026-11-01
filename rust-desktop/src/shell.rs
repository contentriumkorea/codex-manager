use std::path::Path;
#[cfg(windows)]
use windows::{
    core::{Interface, GUID, PCWSTR},
    Win32::{
        Foundation::{HWND, PROPERTYKEY},
        System::{
            Com::StructuredStorage::{PropVariantChangeType, PROPVARIANT, PVCHF_DEFAULT},
            Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER},
            Variant::VT_LPWSTR,
        },
        UI::Shell::{
            IShellLinkW,
            PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
            ShellLink,
        },
    },
};
fn quote(text: &str) -> String {
    let mut output = String::from("\"");
    let mut slashes = 0;
    for ch in text.chars() {
        if ch == '\\' {
            slashes += 1;
        } else {
            output.push_str(&"\\".repeat(if ch == '\"' { slashes * 2 + 1 } else { slashes }));
            output.push(ch);
            slashes = 0;
        }
    }
    output.push_str(&"\\".repeat(slashes * 2));
    output.push('"');
    output
}
#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
#[cfg(windows)]
fn key(pid: u32) -> PROPERTYKEY {
    PROPERTYKEY {
        fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid,
    }
}
#[cfg(windows)]
pub fn clear(hwnd: HWND) -> Result<(), String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).map_err(|e| e.to_string())?;
        for pid in [2, 3, 4, 5] {
            store
                .SetValue(&key(pid), &PROPVARIANT::default())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
#[cfg(windows)]
pub fn register(hwnd: HWND, exe: &Path, state: &Path) -> Result<(), String> {
    let icon = exe
        .parent()
        .ok_or("프로그램 경로 누락")?
        .join("_internal/project_manager/ui/app-icon.ico");
    let fallback = exe
        .parent()
        .ok_or("프로그램 경로 누락")?
        .join("app-icon.ico");
    let icon = if icon.is_file() { icon } else { fallback };
    if !icon.is_file() {
        return Err("작업표시줄 아이콘 파일 누락".into());
    }
    let args = format!("-s {}", quote(&state.to_string_lossy()));
    let mut command = format!("{} {args}", quote(&exe.to_string_lossy()));
    if command.encode_utf16().count() > 255 {
        let link = state.join("taskbar-relaunch.lnk");
        let temporary = state.join(format!(".taskbar-{}.lnk", uuid::Uuid::new_v4()));
        unsafe {
            let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            shell
                .SetPath(PCWSTR(wide(&exe.to_string_lossy()).as_ptr()))
                .map_err(|e| e.to_string())?;
            shell
                .SetArguments(PCWSTR(wide(&args).as_ptr()))
                .map_err(|e| e.to_string())?;
            shell
                .SetWorkingDirectory(PCWSTR(
                    wide(&exe.parent().ok_or("프로그램 경로 누락")?.to_string_lossy()).as_ptr(),
                ))
                .map_err(|e| e.to_string())?;
            shell
                .SetIconLocation(PCWSTR(wide(&icon.to_string_lossy()).as_ptr()), 0)
                .map_err(|e| e.to_string())?;
            let persist: IPersistFile = shell.cast().map_err(|e| e.to_string())?;
            persist
                .Save(PCWSTR(wide(&temporary.to_string_lossy()).as_ptr()), true)
                .map_err(|e| e.to_string())?;
        }
        std::fs::rename(temporary, &link).map_err(|e| e.to_string())?;
        command = quote(&link.to_string_lossy());
    }
    let result = (|| unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).map_err(|e| e.to_string())?;
        for (pid, text) in [
            (2, command),
            (3, format!("{},0", icon.display())),
            (4, "Codex Manager".into()),
            (5, "Contentrium.CodexManager".into()),
        ] {
            let source = PROPVARIANT::from(text.as_str());
            let mut value = PROPVARIANT::default();
            PropVariantChangeType(&mut value, &source, PVCHF_DEFAULT, VT_LPWSTR)
                .map_err(|e| e.to_string())?;
            store
                .SetValue(&key(pid), &value)
                .map_err(|e| e.to_string())?;
            let actual = store.GetValue(&key(pid)).map_err(|e| e.to_string())?;
            if actual != value {
                return Err("작업표시줄 정보 재조회 불일치".into());
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = clear(hwnd);
    }
    result
}

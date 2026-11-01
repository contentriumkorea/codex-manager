use crate::service::VERSION;
use codex_manager_core::{absolute, digest, paths_overlap, valid_bundle_path, write_json_new, Job};
use reqwest::blocking::{Client, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
const REPO: &str = "https://github.com/contentriumkorea/codex-manager";
const ASSET: &str = "Codex-Manager-Windows-x64.zip";
const LIMIT: u64 = 2_147_483_647;
const HELPER: &str = include_str!("../../src/project_manager/install_update.ps1");
pub struct Updater {
    release: Mutex<Option<Value>>,
    prepared: Mutex<Option<Value>>,
    gate: Mutex<()>,
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn version(text: &str) -> Result<[u64; 3], String> {
    let parts = text
        .strip_prefix('v')
        .unwrap_or(text)
        .split('.')
        .map(|p| {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                Err("업데이트 버전 오류".into())
            } else {
                p.parse::<u64>().map_err(error)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    parts.try_into().map_err(|_| "업데이트 버전 오류".into())
}
fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(format!("Codex-Manager/{VERSION}"))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(12))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(6))
        .build()
        .map_err(error)
}
fn request(client: &Client, url: &str) -> Result<Response, String> {
    client
        .get(url)
        .send()
        .map_err(|e| format!("업데이트 서버 연결 실패: {e}"))?
        .error_for_status()
        .map_err(error)
}
fn json_response(response: Response) -> Result<Value, String> {
    let mut bytes = Vec::new();
    response
        .take(512 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > 512 * 1024 {
        return Err("업데이트 안내가 너무 큽니다.".into());
    }
    serde_json::from_slice(&bytes).map_err(error)
}
fn free_space(path: &Path) -> Result<u64, String> {
    #[cfg(windows)]
    {
        use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};
        let wide = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut free = 0;
        unsafe {
            GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None)
                .map_err(error)?;
        }
        Ok(free)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(u64::MAX)
    }
}
fn download(path: &Path, url: &str, size: u64, expected: &str, job: &Job) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(error)?;
    runtime.block_on(async{
        let client=reqwest::Client::builder().user_agent(format!("Codex-Manager/{VERSION}")).connect_timeout(Duration::from_secs(8)).read_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(3600)).https_only(true).redirect(reqwest::redirect::Policy::limited(6)).build().map_err(error)?;
        job.check()?;
        let mut response=client.get(url).send().await.map_err(error)?.error_for_status().map_err(error)?;
        let mut output=fs::OpenOptions::new().create_new(true).write(true).open(path).map_err(error)?;
        let mut hash=Sha256::new();let mut done=0u64;
        loop{
            job.check()?;
            let chunk=std::pin::pin!(response.chunk());let mut chunk=chunk;
            let bytes=loop{tokio::select!{result=&mut chunk=>break result.map_err(error)?,_=tokio::time::sleep(Duration::from_millis(100))=>{job.check()?;}}};
            let Some(bytes)=bytes else{break;};
            done=done.checked_add(bytes.len() as u64).ok_or("다운로드 크기 초과")?;
            if done>size{return Err("다운로드 파일이 안내 크기를 초과합니다.".into());}
            output.write_all(&bytes).map_err(error)?;hash.update(&bytes);job.advance(done,ASSET,bytes.len() as u64);
        }
        output.sync_all().map_err(error)?;
        if done!=size||format!("{:x}",hash.finalize())!=expected.to_lowercase(){return Err("다운로드 무결성 검증에 실패했습니다.".into());}Ok(())
    })
}
fn install() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(error)?;
    if exe.file_name().and_then(|s| s.to_str()) != Some("CodexManager.exe") {
        return Err("업데이트는 설치형 또는 포터블 배포에서 사용할 수 있습니다.".into());
    }
    let dir = exe.parent().ok_or("설치 폴더 누락")?.to_owned();
    absolute(&dir)?;
    Ok(dir)
}
fn validate_plan(plan: &Value, state: &Path) -> Result<(), String> {
    let current = install()?;
    let staged = PathBuf::from(plan["staged"].as_str().ok_or("준비 경로 누락")?);
    let backup = PathBuf::from(plan["backup"].as_str().ok_or("복구 경로 누락")?);
    absolute(&staged)?;
    absolute(&backup)?;
    absolute(state)?;
    if plan["install_dir"] != json!(current)
        || plan["state_dir"] != json!(state)
        || paths_overlap(&current, state)
        || staged.file_name().and_then(|s| s.to_str()) != Some("CodexManager")
        || staged.parent().and_then(Path::parent) != current.parent()
        || !staged
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|s| s.to_string_lossy().starts_with(".codex-manager-update-"))
        || backup.parent() != current.parent()
        || !backup
            .file_name()
            .is_some_and(|s| s.to_string_lossy().starts_with(".codex-manager-previous-"))
    {
        return Err("프로그램 교체 경로가 올바르지 않습니다.".into());
    }
    Ok(())
}
fn verify_prepared(plan: &Value, state: &Path, job: &Job) -> Result<(), String> {
    validate_plan(plan, state)?;
    let staged = PathBuf::from(plan["staged"].as_str().ok_or("준비 경로 누락")?);
    let path = staged.join("install-manifest.json");
    if fs::metadata(&path).map_err(error)?.len() > 512 * 1024
        || digest(&path, job)?
            != plan["manifest_sha256"]
                .as_str()
                .ok_or("파일 목록 해시 누락")?
    {
        return Err("준비된 업데이트 목록이 변경됐습니다.".into());
    }
    let m: Value = serde_json::from_slice(&fs::read(&path).map_err(error)?).map_err(error)?;
    if m["version"] != plan["version"] {
        return Err("업데이트 버전 불일치".into());
    }
    let expected = m["files"].as_object().ok_or("파일 목록 오류")?;
    if !expected.contains_key("CodexManager.exe") {
        return Err("실행 파일 누락".into());
    }
    let mut seen = HashSet::new();
    let mut stack = vec![staged.clone()];
    while let Some(dir) = stack.pop() {
        job.check()?;
        absolute(&dir)?;
        for entry in fs::read_dir(dir).map_err(error)? {
            job.check()?;
            let entry = entry.map_err(error)?;
            absolute(&entry.path())?;
            if entry.file_type().map_err(error)?.is_dir() {
                stack.push(entry.path());
            } else {
                let name = entry
                    .path()
                    .strip_prefix(&staged)
                    .map_err(error)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if name != "install-manifest.json" {
                    let hash = expected
                        .get(&name)
                        .and_then(Value::as_str)
                        .ok_or("예상하지 않은 업데이트 파일")?;
                    if digest(&entry.path(), job)? != hash {
                        return Err("준비된 업데이트 파일이 변경됐습니다.".into());
                    }
                    seen.insert(name);
                }
            }
        }
    }
    if seen.len() != expected.len() {
        return Err("준비된 업데이트 파일 누락".into());
    }
    Ok(())
}
fn extract_archive(
    archive: &Path,
    root: &Path,
    release_version: &str,
    job: &Job,
) -> Result<PathBuf, String> {
    let mut buffer = vec![0; 1024 * 1024];
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).map_err(error)?).map_err(error)?;
    let mut entries = HashMap::new();
    let mut seen = HashSet::new();
    let mut expanded = 0u64;
    for i in 0..zip.len() {
        job.check()?;
        let entry = zip.by_index(i).map_err(error)?;
        let name = entry.name().trim_end_matches('/');
        if name == "CodexManager" && entry.is_dir() {
            continue;
        }
        let relative = name
            .strip_prefix("CodexManager/")
            .ok_or("프로그램 바깥 파일이 있습니다.")?;
        if !valid_bundle_path(relative)
            || !seen.insert(relative.to_lowercase())
            || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err("안전하지 않은 업데이트 파일 경로".into());
        }
        if !entry.is_dir() {
            expanded = expanded
                .checked_add(entry.size())
                .ok_or("업데이트 크기 초과")?;
            entries.insert(relative.to_owned(), (i, entry.size()));
        }
    }
    if expanded > LIMIT || free_space(root)? < expanded + 10 * 1024 * 1024 {
        return Err("업데이트 설치 공간이 부족합니다.".into());
    }
    let (index, len) = entries
        .get("install-manifest.json")
        .ok_or("업데이트 파일 목록 누락")?;
    if *len > 512 * 1024 {
        return Err("업데이트 파일 목록 크기 초과".into());
    }
    let mut bytes = Vec::new();
    zip.by_index(*index)
        .map_err(error)?
        .take(512 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    let manifest: Value = serde_json::from_slice(&bytes).map_err(error)?;
    let expected = manifest["files"]
        .as_object()
        .ok_or("업데이트 파일 목록 오류")?;
    if manifest["version"] != release_version
        || !expected.contains_key("CodexManager.exe")
        || entries.len() != expected.len() + 1
        || entries
            .keys()
            .any(|n| n != "install-manifest.json" && !expected.contains_key(n))
    {
        return Err("업데이트 파일 구성이 일치하지 않습니다.".into());
    }
    let staged = root.join("CodexManager");
    fs::create_dir(&staged).map_err(error)?;
    job.phase("업데이트 파일 검증·준비", expanded);
    let mut done = 0;
    for (name, (index, _)) in &entries {
        job.check()?;
        let path = staged.join(name);
        fs::create_dir_all(path.parent().ok_or("파일 부모 폴더 누락")?).map_err(error)?;
        let mut source = zip.by_index(*index).map_err(error)?;
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(error)?;
        let mut hash = Sha256::new();
        loop {
            job.check()?;
            let n = source.read(&mut buffer).map_err(error)?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n]).map_err(error)?;
            hash.update(&buffer[..n]);
            done += n as u64;
            job.advance(done, name, n as u64);
        }
        output.sync_all().map_err(error)?;
        if name != "install-manifest.json"
            && expected[name].as_str() != Some(&format!("{:x}", hash.finalize()))
        {
            return Err("업데이트 내부 파일 검증 실패".into());
        }
    }
    Ok(staged)
}
impl Updater {
    pub fn new() -> Self {
        Self {
            release: Mutex::new(None),
            prepared: Mutex::new(None),
            gate: Mutex::new(()),
        }
    }
    pub fn check(&self, job: &Job) -> Result<Value, String> {
        job.phase_items("최신 버전 확인", 1);
        job.check()?;
        let client = client()?;
        let response = client
            .get("https://api.github.com/repos/contentriumkorea/codex-manager/releases/latest")
            .timeout(Duration::from_secs(12))
            .send()
            .map_err(error)?;
        let status = response.status().as_u16();
        let release = if [403, 429, 500, 502, 503, 504].contains(&status) {
            let m = json_response(request(
                &client,
                &format!("{REPO}/releases/latest/download/update.json"),
            )?)?;
            let v = m["version"].as_str().ok_or("버전 누락")?;
            version(v)?;
            if m["asset"] != ASSET {
                return Err("공식 업데이트 파일 누락".into());
            }
            json!({"version":v,"archive":format!("{REPO}/releases/download/v{v}/{ASSET}"),"metadata":format!("{REPO}/releases/download/v{v}/update.json"),"notes":"최신 정식 버전","url":format!("{REPO}/releases/tag/v{v}")})
        } else if status == 404 {
            json!({"version":VERSION,"notes":"공개된 새 버전이 없습니다."})
        } else {
            let data = json_response(response.error_for_status().map_err(error)?)?;
            if data["draft"] == true || data["prerelease"] == true {
                return Err("정식 배포가 아닙니다.".into());
            }
            let tag = data["tag_name"].as_str().ok_or("배포 태그 누락")?;
            let v = tag.strip_prefix('v').ok_or("정식 버전 태그 오류")?;
            version(v)?;
            let prefix = format!("{REPO}/releases/download/{tag}/");
            let assets = data["assets"].as_array().ok_or("배포 파일 목록 누락")?;
            for name in [ASSET, "update.json"] {
                if !assets.iter().any(|a| {
                    a["name"] == name && a["browser_download_url"] == format!("{prefix}{name}")
                }) {
                    return Err("공식 배포 파일 주소가 일치하지 않습니다.".into());
                }
            }
            json!({"version":v,"archive":format!("{prefix}{ASSET}"),"metadata":format!("{prefix}update.json"),"notes":data["body"],"url":format!("{REPO}/releases/tag/{tag}")})
        };
        let mut release = release;
        release["available"] =
            json!(version(release["version"].as_str().ok_or("버전 누락")?)? > version(VERSION)?);
        *self.release.lock().map_err(error)? = Some(release.clone());
        Ok(release)
    }
    pub fn prepare(&self, state: &Path, job: &Job) -> Result<Value, String> {
        let _gate = self.gate.lock().map_err(error)?;
        job.check()?;
        let release = self
            .release
            .lock()
            .map_err(error)?
            .clone()
            .ok_or("업데이트를 먼저 확인하세요.")?;
        if release["available"] != true {
            return Err("현재 최신 버전입니다.".into());
        }
        let updates = state.join("updates");
        absolute(&updates)?;
        fs::create_dir_all(&updates).map_err(error)?;
        let current = install()?;
        let parent = current.parent().ok_or("설치 상위 폴더 누락")?;
        let key = format!(
            "{:x}",
            Sha256::digest(current.to_string_lossy().to_lowercase().as_bytes())
        );
        let cache = updates.join(format!("rust-prepared-{}.json", &key[..16]));
        if let Ok(bytes) = fs::read(&cache) {
            if let Ok(mut plan) = serde_json::from_slice::<Value>(&bytes) {
                if plan["version"] == release["version"]
                    && verify_prepared(&plan, state, job).is_ok()
                {
                    plan["parent_pid"] = json!(std::process::id());
                    *self.prepared.lock().map_err(error)? = Some(plan);
                    return Ok(json!({"prepared":true,"version":release["version"]}));
                }
            }
        }
        let client = client()?;
        let metadata = json_response(request(
            &client,
            release["metadata"].as_str().ok_or("안내 주소 누락")?,
        )?)?;
        let size = metadata["size"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= LIMIT)
            .ok_or("업데이트 크기 오류")?;
        let expected = metadata["sha256"]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("업데이트 해시 오류")?;
        if metadata["version"] != release["version"] || metadata["asset"] != ASSET {
            return Err("업데이트 안내가 배포와 다릅니다.".into());
        }
        if free_space(parent)? < size + 10 * 1024 * 1024 {
            return Err("업데이트 다운로드 공간이 부족합니다.".into());
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let root = parent.join(format!(".codex-manager-update-{id}"));
        fs::create_dir(&root).map_err(error)?;
        let archive = root.join("download.zip");
        let result = (|| {
            job.phase("업데이트 다운로드", size);
            download(
                &archive,
                release["archive"].as_str().ok_or("파일 주소 누락")?,
                size,
                expected,
                job,
            )?;
            let staged = extract_archive(
                &archive,
                &root,
                release["version"].as_str().ok_or("버전 누락")?,
                job,
            )?;
            let plan = json!({"id":id,"version":release["version"],"install_dir":current,"staged":staged,"backup":parent.join(format!(".codex-manager-previous-{id}")),"state_dir":state,"parent_pid":std::process::id(),"health":updates.join(format!("{id}.health.json")),"ready":updates.join(format!("{id}.ready.json")),"result":updates.join("last-result.json"),"manifest_sha256":digest(&staged.join("install-manifest.json"),job)?});
            validate_plan(&plan, state)?;
            let temporary = updates.join(format!(".prepared-{id}.json"));
            write_json_new(&temporary, &plan)?;
            fs::rename(temporary, &cache).map_err(error)?;
            *self.prepared.lock().map_err(error)? = Some(plan);
            Ok(json!({"prepared":true,"version":release["version"]}))
        })();
        let _ = fs::remove_file(archive);
        if result.is_err() {
            absolute(&root)?;
            if root.parent() == Some(parent)
                && root
                    .file_name()
                    .is_some_and(|s| s.to_string_lossy().starts_with(".codex-manager-update-"))
            {
                let _ = fs::remove_dir_all(&root);
            }
        }
        result
    }
    pub fn apply(&self, state: &Path) -> Result<(), String> {
        let _gate = self.gate.lock().map_err(error)?;
        let mut plan = self
            .prepared
            .lock()
            .map_err(error)?
            .clone()
            .ok_or("업데이트 다운로드를 먼저 완료하세요.")?;
        verify_prepared(&plan, state, &Job::new())?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let updates = state.join("updates");
        let install = install()?;
        plan["id"] = json!(id);
        plan["parent_pid"] = json!(std::process::id());
        plan["backup"] = json!(install
            .parent()
            .ok_or("설치 부모 누락")?
            .join(format!(".codex-manager-previous-{id}")));
        plan["health"] = json!(updates.join(format!("{id}.health.json")));
        plan["ready"] = json!(updates.join(format!("{id}.ready.json")));
        let path = updates.join(format!("{id}.plan.json"));
        write_json_new(&path, &plan)?;
        let script = updates.join(format!("{id}.ps1"));
        fs::write(&script, HELPER).map_err(error)?;
        fs::write(updates.join(format!("recover-{id}.cmd")),format!("@echo off\r\npowershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File \"%~dp0{id}.ps1\" -Plan \"%~dp0{id}.plan.json\"\r\n")).map_err(error)?;
        let powershell = PathBuf::from(std::env::var_os("SystemRoot").ok_or("Windows 폴더 누락")?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = Command::new(powershell);
        command.env_remove("PSModulePath");
        command.env(
            "PSModulePath",
            PathBuf::from(std::env::var_os("SystemRoot").ok_or("Windows 폴더 누락")?)
                .join("System32/WindowsPowerShell/v1.0/Modules"),
        );
        command
            .args([
                "-NoProfile",
                "-WindowStyle",
                "Hidden",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .arg("-Plan")
            .arg(&path)
            .current_dir(&updates)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(error)?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let ready = Path::new(plan["ready"].as_str().ok_or("준비 확인 경로 누락")?);
        while Instant::now() < deadline {
            if let Ok(bytes) = fs::read(ready) {
                if let Ok(record) = serde_json::from_slice::<Value>(&bytes) {
                    if record["id"] == id
                        && record["version"] == plan["version"]
                        && record["pid"] == child.id()
                    {
                        return Ok(());
                    }
                }
            }
            if child.try_wait().map_err(error)?.is_some() {
                return Err(
                    "업데이트 도우미가 준비되지 않았습니다. 현재 프로그램을 유지했습니다.".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err("설치 준비 시간이 초과됐습니다. 현재 프로그램을 유지했습니다.".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn archive(root: &Path, changed: bool, bad_path: bool) -> PathBuf {
        fs::create_dir_all(root).unwrap();
        let path = root.join("payload.zip");
        let mut z = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let hash = format!("{:x}", Sha256::digest(b"app"));
        let manifest =
            json!({"version":"0.11.0","files":{"CodexManager.exe":hash,"_internal/icon.ico":hash}});
        for (name, bytes) in [
            (
                "CodexManager/CodexManager.exe",
                if changed {
                    b"bad".as_slice()
                } else {
                    b"app".as_slice()
                },
            ),
            ("CodexManager/_internal/icon.ico", b"app".as_slice()),
            (
                "CodexManager/install-manifest.json",
                manifest.to_string().as_bytes(),
            ),
        ] {
            z.start_file(name, options).unwrap();
            z.write_all(bytes).unwrap();
        }
        if bad_path {
            z.start_file("CodexManager/../escape.txt", options).unwrap();
            z.write_all(b"escape").unwrap();
        }
        z.finish().unwrap();
        path
    }
    #[test]
    fn archive_integrity_escape_and_cancellation() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        for (name, changed, bad_path, expected) in [
            ("valid", false, false, true),
            ("changed", true, false, false),
            ("escape", false, true, false),
        ] {
            let root = base.join(name);
            let file = archive(&root, changed, bad_path);
            let result = extract_archive(&file, &root, "0.11.0", &Job::new());
            assert_eq!(result.is_ok(), expected, "{result:?}");
            assert!(!base.join("escape.txt").exists());
        }
        let root = base.join("cancel");
        let file = archive(&root, false, false);
        let job = Job::new();
        job.cancel();
        assert!(extract_archive(&file, &root, "0.11.0", &job).is_err());
        assert!(!root.join("CodexManager").exists());
    }
    #[test]
    fn version_order_and_rejects_unstable() {
        assert!(version("v0.11.0").unwrap() > version("0.10.4").unwrap());
        for text in ["0.11.0-beta", "1.2", "1.2.3.4", "1..3", "../1.2.3"] {
            assert!(version(text).is_err());
        }
    }
}

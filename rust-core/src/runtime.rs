use crate::file_manager::file_identity;
use crate::{all_threads, clean_path, ensure_plain, identity, roots, Catalog, Job};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
static CACHE: OnceLock<Mutex<HashMap<String, Vec<PathBuf>>>> = OnceLock::new();
fn parse_line(line: &[u8]) -> Result<Option<Vec<PathBuf>>, String> {
    if !line
        .windows(b"runtime_workspace_roots".len())
        .any(|s| s == b"runtime_workspace_roots")
    {
        return Ok(None);
    }
    let record: Value = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    let settings = match record["type"].as_str() {
        Some("session_meta" | "turn_context") => &record["payload"],
        Some("event_msg") if record["payload"]["type"] == "thread_settings_applied" => {
            &record["payload"]["thread_settings"]
        }
        _ => return Ok(None),
    };
    let Some(value) = settings.get("runtime_workspace_roots") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(Some(Vec::new()));
    }
    Ok(Some(
        value
            .as_array()
            .ok_or("작업 폴더 기록 오류")?
            .iter()
            .map(|p| {
                p.as_str()
                    .map(|s| clean_path(Path::new(s)))
                    .ok_or("작업 폴더 경로 오류".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?,
    ))
}
pub fn runtime_roots(path: &Path) -> Result<Vec<PathBuf>, String> {
    runtime_roots_with_job(path, &Job::new())
}
pub fn runtime_roots_with_job(path: &Path, job: &Job) -> Result<Vec<PathBuf>, String> {
    job.check()?;
    ensure_plain(path)?;
    let mut file = File::open(path).map_err(|e| format!("대화 작업 폴더 확인 실패: {e}"))?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    let modified = before.modified().map_err(|e| e.to_string())?;
    let file_id = file_identity(&file)?;
    let key = format!(
        "{}|{}|{}|{:?}",
        clean_path(path).to_string_lossy().to_lowercase(),
        file_id,
        before.len(),
        modified
    );
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(value) = cache
        .lock()
        .map_err(|_| "작업 폴더 캐시 오류")?
        .get(&key)
        .cloned()
    {
        return Ok(value);
    }
    let mut position = before.len();
    let mut fragments: Vec<Vec<u8>> = Vec::new();
    let mut result = None;
    'scan: while position > 0 {
        job.check()?;
        let count = position.min(65536) as usize;
        position -= count as u64;
        file.seek(SeekFrom::Start(position))
            .map_err(|e| e.to_string())?;
        let mut chunk = vec![0; count];
        file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
        let parts = chunk.split(|b| *b == b'\n').collect::<Vec<_>>();
        if parts.len() == 1 {
            fragments.push(chunk);
            continue;
        }
        let capacity = parts.last().unwrap().len() + fragments.iter().map(Vec::len).sum::<usize>();
        let mut line = Vec::with_capacity(capacity);
        line.extend_from_slice(parts.last().unwrap());
        for fragment in fragments.iter().rev() {
            line.extend_from_slice(fragment);
        }
        fragments.clear();
        if let Some(paths) = parse_line(&line)? {
            result = Some(paths);
            break;
        }
        for part in parts[1..parts.len() - 1].iter().rev() {
            job.check()?;
            if let Some(paths) = parse_line(part)? {
                result = Some(paths);
                break 'scan;
            }
        }
        fragments.push(parts[0].to_vec());
    }
    if result.is_none() && !fragments.is_empty() {
        let line = fragments.into_iter().rev().flatten().collect::<Vec<_>>();
        result = parse_line(&line)?;
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    ensure_plain(path)?;
    if before.len() != after.len()
        || after.modified().map_err(|e| e.to_string())? != modified
        || identity(path)? != file_id
    {
        return Err("확인 중 대화가 변경됐습니다. 다시 시도하세요.".into());
    }
    let result = result.unwrap_or_default();
    let mut cache = cache.lock().map_err(|_| "작업 폴더 캐시 오류")?;
    if cache.len() > 4096 {
        cache.clear();
    }
    cache.insert(key, result.clone());
    Ok(result)
}
pub fn protected_workspaces(catalog: &Catalog, job: &Job) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for group in &catalog.projects {
        paths.extend(roots(&group.project)?);
    }
    for thread in all_threads(catalog) {
        job.check()?;
        if let Some(cwd) = thread["cwd"].as_str() {
            paths.push(clean_path(Path::new(cwd)));
        }
        if let Some(path) = thread["path"].as_str() {
            paths.extend(runtime_roots_with_job(Path::new(path), job)?);
        }
    }
    Ok(paths)
}

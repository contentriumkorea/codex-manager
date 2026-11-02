use crate::{all_threads, clean_path, roots, Catalog, Job};
use serde_json::Value;
use std::path::{Path, PathBuf};
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
    let mut reader = crate::RolloutReader::open_live(path, None, job)?;
    let mut result = Vec::new();
    while let Some(record) = reader.next_record()? {
        if let Some(paths) = parse_line(record.line.as_bytes())? {
            result = paths;
        }
    }
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

use crate::{
    merge_legacy_assignments, merge_legacy_projects, organize_catalog,
    read_database_catalog_with_state, Catalog,
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Store {
    pub home: PathBuf,
    pub sqlite_home: PathBuf,
    pub database: Option<PathBuf>,
}
pub fn ensure_plain(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("절대 경로가 필요합니다.".into());
    }
    for ancestor in path.ancestors() {
        let meta = match fs::symlink_metadata(ancestor) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        if meta.file_type().is_symlink() {
            return Err("링크 경로는 변경할 수 없습니다.".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("정션·링크 경로는 사용할 수 없습니다.".into());
            }
        }
    }
    Ok(())
}
pub fn absolute(path: &Path) -> Result<PathBuf, String> {
    use std::path::Component;
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("상위 경로 없는 절대 경로를 선택하세요.".into());
    }
    ensure_plain(path)?;
    Ok(path.to_owned())
}
pub fn resolve_store(home: &Path) -> Result<Store, String> {
    let home = absolute(home)?;
    if !home.is_dir() {
        return Err("Codex 저장소 폴더를 찾을 수 없습니다.".into());
    }
    let config = home.join("config.toml");
    let mut sqlite_home = home.clone();
    if config.exists() {
        ensure_plain(&config)?;
        let config: toml::Value = fs::read_to_string(&config)
            .map_err(|e| e.to_string())?
            .parse::<toml::Value>()
            .map_err(|e| format!("Codex 설정 오류: {e}"))?;
        if let Some(value) = config.get("sqlite_home") {
            sqlite_home = PathBuf::from(
                value
                    .as_str()
                    .ok_or("sqlite_home은 경로 문자열이어야 합니다.")?,
            );
            if sqlite_home.starts_with("~") {
                let user =
                    std::env::var_os("USERPROFILE").ok_or("사용자 폴더를 찾을 수 없습니다.")?;
                sqlite_home = PathBuf::from(user).join(sqlite_home.strip_prefix("~").unwrap());
            }
            absolute(&sqlite_home)?;
        }
    }
    let mut databases = Vec::new();
    if sqlite_home.is_dir() {
        for entry in fs::read_dir(&sqlite_home).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let n = path
                .file_name()
                .and_then(|s| s.to_str())
                .and_then(|s| s.strip_prefix("state_"))
                .and_then(|s| s.strip_suffix(".sqlite"))
                .and_then(|s| s.parse::<u64>().ok());
            if let Some(n) = n {
                ensure_plain(&path)?;
                if path.is_file() {
                    databases.push((n, path));
                }
            }
        }
    }
    databases.sort_by_key(|(n, _)| *n);
    Ok(Store {
        home,
        sqlite_home,
        database: databases.pop().map(|(_, p)| p),
    })
}
pub fn read_store(store: &Store) -> Result<Catalog, String> {
    let path = store.home.join(".codex-global-state.json");
    let state: Value = if path.exists() {
        ensure_plain(&path)?;
        serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
    } else {
        serde_json::json!({})
    };
    let host = format!("local:{}", store.home.display());
    if let Some(db) = &store.database {
        return read_database_catalog_with_state(db, &state, &host);
    }
    let mut projects = Vec::new();
    let mut threads = Vec::new();
    merge_legacy_projects(&mut projects, &state, &host)?;
    merge_legacy_assignments(&projects, &mut threads, &state, &host)?;
    organize_catalog(projects, threads)
}
pub fn discover_stores(seeds: &[PathBuf], scan_roots: &[PathBuf]) -> Result<Vec<Store>, String> {
    discover_stores_with_job(seeds, scan_roots, &crate::Job::new())
}
pub fn discover_stores_with_job(
    seeds: &[PathBuf],
    scan_roots: &[PathBuf],
    job: &crate::Job,
) -> Result<Vec<Store>, String> {
    let mut candidates = seeds.to_vec();
    if let Some(home) = std::env::var_os("CODEX_HOME") {
        candidates.push(home.into());
    }
    if let Some(user) = std::env::var_os("USERPROFILE") {
        candidates.push(PathBuf::from(user).join(".codex"));
    }
    let mut stack: Vec<_> = scan_roots.iter().cloned().map(|p| (p, 0)).collect();
    let mut count = 0;
    while let Some((dir, depth)) = stack.pop() {
        job.check()?;
        job.advance(count, &dir.to_string_lossy(), 0);
        count += 1;
        if count > 100_000 {
            return Err("검색 범위가 너무 넓습니다. 저장소가 있던 상위 폴더를 선택하세요.".into());
        }
        if ensure_plain(&dir).is_err() {
            continue;
        }
        if dir.join("config.toml").is_file() || dir.join(".codex-global-state.json").is_file() {
            candidates.push(dir.clone());
        }
        if depth >= 5 {
            continue;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            job.check()?;
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if [
                "node_modules",
                ".git",
                "windows",
                "program files",
                "$recycle.bin",
                "target",
            ]
            .contains(&name.as_str())
            {
                continue;
            }
            let path = entry.path();
            if path.is_dir() && ensure_plain(&path).is_ok() {
                stack.push((path, depth + 1));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut stores = Vec::new();
    for path in candidates {
        job.check()?;
        if let Ok(store) = resolve_store(&path) {
            if (store.database.is_some() || store.home.join(".codex-global-state.json").is_file())
                && seen.insert(store.home.to_string_lossy().to_lowercase())
            {
                stores.push(store);
            }
        }
    }
    Ok(stores)
}

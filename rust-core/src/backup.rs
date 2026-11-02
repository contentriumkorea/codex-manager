use crate::{
    absolute, all_threads, digest, ensure_plain, load_verified_bundle, roots, runtime_roots,
    scan_tree, tree_size, valid_bundle_path, write_json_new, Catalog, Entry, Job, Tree,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub fn clean_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(tail) = s.strip_prefix("\\\\?\\UNC\\") {
        PathBuf::from(format!("\\\\{tail}"))
    } else if let Some(tail) = s.strip_prefix("\\\\?\\") {
        PathBuf::from(tail)
    } else {
        path.to_owned()
    }
}
pub fn paths_overlap(a: &Path, b: &Path) -> bool {
    let a = clean_path(a)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase();
    let b = clean_path(b)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase();
    a == b || a.starts_with(&(b.clone() + "/")) || b.starts_with(&(a + "/"))
}
pub fn map_workspace(path: &Path, mapping: &[(PathBuf, PathBuf)]) -> Result<PathBuf, String> {
    let path = clean_path(path);
    let components = path.components().collect::<Vec<_>>();
    let mut ordered = mapping.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(old, _)| std::cmp::Reverse(old.components().count()));
    for (old, new) in ordered {
        let old = clean_path(old);
        let prefix = old.components().collect::<Vec<_>>();
        if components.len() >= prefix.len()
            && components.iter().zip(&prefix).all(|(a, b)| {
                a.as_os_str().to_string_lossy().to_lowercase()
                    == b.as_os_str().to_string_lossy().to_lowercase()
            })
        {
            let mut result = new.clone();
            for c in components.iter().skip(prefix.len()) {
                result.push(c.as_os_str());
            }
            return Ok(result);
        }
    }
    Err(format!(
        "프로젝트 폴더 밖의 작업 경로입니다: {}",
        path.display()
    ))
}
fn conversation(thread: &Value, relative: String) -> Result<Value, String> {
    let id = thread["id"].as_str().ok_or("대화 ID 오류")?;
    uuid::Uuid::parse_str(id).map_err(|_| "대화 ID 오류")?;
    let path = Path::new(thread["path"].as_str().ok_or("대화 기록이 없습니다.")?);
    Ok(
        json!({"id":id,"project_id":thread["projectId"],"cwd":thread["cwd"],"runtime_roots":runtime_roots(path)?,"archived":thread["archived"].as_bool().unwrap_or(false),"parent_id":thread["parentThreadId"],"revision":"rust-export","running":false,"title":thread["name"].as_str().unwrap_or("제목 없는 대화"),"rollout":relative,"updated_at":thread["updatedAt"].as_i64().unwrap_or(0),"attachments":[],"internal":thread["internal"].as_bool().unwrap_or(false)}),
    )
}
pub fn backup_project(
    catalog: &Catalog,
    id: &str,
    destination: &Path,
    job: &Job,
) -> Result<Value, String> {
    backup_project_observed(catalog, id, destination, job, &|_| Ok(()), &|_, _, _, _| {
        Ok(())
    })
}
pub(crate) fn backup_project_observed(
    catalog: &Catalog,
    id: &str,
    destination: &Path,
    job: &Job,
    created: &dyn Fn(&Path) -> Result<(), String>,
    copied: &crate::transfer::CopyObserver<'_>,
) -> Result<Value, String> {
    absolute(destination)?;
    if destination.exists() {
        return Err("백업 폴더가 이미 있습니다. 새 폴더를 선택하세요.".into());
    }
    let group = catalog
        .projects
        .iter()
        .find(|g| g.project["id"] == id)
        .ok_or("프로젝트가 없습니다.")?;
    let paths = roots(&group.project)?;
    for path in &paths {
        if paths_overlap(path, destination) {
            return Err("백업 대상이 원본 폴더와 겹칩니다.".into());
        }
    }
    let mut captures = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        job.check()?;
        captures.push((
            format!("root-{:02}", index + 1),
            path.clone(),
            scan_tree(path, job)?,
        ));
    }
    let all = all_threads(catalog);
    let mut ids: HashSet<String> = group
        .threads
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect();
    loop {
        let before = ids.len();
        for t in &all {
            if ["parentThreadId", "attachedThreadParentId"]
                .iter()
                .any(|key| t[*key].as_str().is_some_and(|id| ids.contains(id)))
            {
                if let Some(id) = t["id"].as_str() {
                    ids.insert(id.into());
                }
            }
        }
        if ids.len() == before {
            break;
        }
    }
    let selected: Vec<_> = all
        .iter()
        .filter(|t| t["id"].as_str().is_some_and(|id| ids.contains(id)))
        .collect();
    let mut records = Vec::new();
    let mut dependencies = Vec::new();
    for t in &selected {
        job.check()?;
        let resolved = crate::resolve_rollout_path(Path::new(
            t["path"].as_str().ok_or("Missing rollout path")?,
        ))?;
        let suffix = if resolved.extension().is_some_and(|e| e == "zst") {
            "jsonl.zst"
        } else {
            "jsonl"
        };
        let relative = format!(
            "conversations/{}.{suffix}",
            t["id"].as_str().ok_or("Invalid thread ID")?
        );
        let record = conversation(t, relative)?;
        let map = paths
            .iter()
            .map(|p| (p.clone(), p.clone()))
            .collect::<Vec<_>>();
        if map_workspace(
            Path::new(record["cwd"].as_str().ok_or("작업 경로 오류")?),
            &map,
        )
        .is_err()
            || record["runtime_roots"].as_array().unwrap().iter().any(|r| {
                r.as_str()
                    .is_none_or(|r| map_workspace(Path::new(r), &map).is_err())
            })
        {
            dependencies
                .push(json!({"kind":"external_workspace","thread_id":t["id"],"resolved":false}));
        }
        if t["attachmentCount"].as_u64().unwrap_or(0) > 0 || !t["attachedThreadParentId"].is_null()
        {
            dependencies
                .push(json!({"kind":"thread_attachments","thread_id":t["id"],"resolved":false}));
        }
        if t["projectId"] != id && !t["projectId"].is_null() {
            dependencies.push(json!({"kind":"cross_project_child","thread_id":t["id"]}));
        }
        let source = Path::new(t["path"].as_str().ok_or("대화 기록 경로 누락")?);
        let mut reader = crate::RolloutReader::open(source, t["id"].as_str(), job)?;
        while let Some(decoded) = reader.next_record()? {
            let line = decoded.line;
            if !line.contains("image_path") && !line.contains("\"path\"") {
                continue;
            }
            let item: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            if item["type"] != "response_item" {
                continue;
            }
            if let Some(parts) = item["payload"]["content"].as_array() {
                for part in parts {
                    if let Some(raw) = part["path"].as_str().or(part["image_path"].as_str()) {
                        if Path::new(raw).is_absolute()
                            && map_workspace(Path::new(raw), &map).is_err()
                        {
                            dependencies.push(
                                json!({"kind":"external_attachment","path":raw,"resolved":false}),
                            );
                        }
                    }
                }
            }
        }
        records.push(record);
    }
    for path in &paths {
        if path.join(".git").is_file() {
            dependencies.push(json!({"kind":"linked_worktree","path":path,"resolved":false}));
        }
    }
    job.check()?;
    if let Some(parent) = destination.parent() {
        ensure_plain(parent)?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir(destination).map_err(|e| e.to_string())?;
    created(destination)?;
    for dir in ["files", "conversations"] {
        let path = destination.join(dir);
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        copied(&path, None, &crate::identity(&path)?, None)?;
    }
    let mut inventory = Vec::new();
    let mut directories = Vec::new();
    for (rid, source, tree) in &captures {
        crate::transfer::copy_tree_observed(
            source,
            &destination.join("files").join(rid),
            tree,
            job,
            copied,
        )?;
        for e in &tree.files {
            let meta = fs::metadata(source.join(&e.path)).map_err(|e| e.to_string())?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|t| t.as_nanos() as u64)
                .unwrap_or(0);
            inventory.push(json!({"root_id":rid,"relative_path":e.path,"size":e.size,"sha256":e.sha256,"file_id":"rust-captured","mtime_ns":mtime}));
        }
        for dir in &tree.directories {
            directories.push(json!([rid, dir]));
        }
    }
    let total = selected.iter().try_fold(0u64, |sum, t| {
        let size = fs::metadata(crate::resolve_rollout_path(Path::new(
            t["path"].as_str().ok_or("대화 기록 경로 누락")?,
        ))?)
        .map_err(|e| e.to_string())?
        .len();
        sum.checked_add(size)
            .ok_or_else(|| "대화 크기 초과".to_owned())
    })?;
    job.phase("대화 기록 복사·검증", total);
    let mut done = 0;
    for (t, record) in selected.iter().zip(&records) {
        let resolved =
            crate::resolve_rollout_path(Path::new(t["path"].as_str().ok_or("대화 경로 누락")?))?;
        let source = resolved.as_path();
        let entry = Entry {
            identity: Some(crate::identity(source)?),
            path: record["rollout"].as_str().unwrap().into(),
            size: fs::metadata(source).map_err(|e| e.to_string())?.len(),
            sha256: digest(source, job)?,
        };
        let owned_identity = crate::transfer::copy_file_owned(
            source,
            &destination.join(&entry.path),
            &entry,
            job,
            &mut done,
            total,
        )?;
        copied(
            &destination.join(&entry.path),
            Some(source),
            &owned_identity,
            Some(&entry.sha256),
        )?;
    }
    let manifest = json!({"format_version":1,"bundle_id":uuid::Uuid::new_v4().to_string(),"state":"complete","captured_at":chrono::Utc::now().to_rfc3339(),"project":{"id":id,"name":group.project["name"],"roots":paths,"legacy_ids":[]},"roots":captures.iter().map(|(rid,p,_)|json!({"id":rid,"original_path":p,"bundle_path":format!("files/{rid}")})).collect::<Vec<_>>(),"conversations":records,"dependencies":dependencies,"snapshot_revision":"rust-export","inventory":inventory,"directories":directories,"folder_grouped_threads":[]});
    write_json_new(&destination.join("manifest.json"), &manifest)?;
    copied(
        &destination.join("manifest.json"),
        None,
        &crate::identity(&destination.join("manifest.json"))?,
        None,
    )?;
    let captured = scan_tree(destination, job)?;
    let total = tree_size(&captured)?;
    job.phase("백업 검증 목록 생성", total);
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination.join("checksums.jsonl"))
        .map_err(|e| e.to_string())?;
    for entry in captured.files {
        job.check()?;
        writeln!(
            output,
            "{}",
            json!({"path":entry.path,"size":entry.size,"sha256":entry.sha256})
        )
        .map_err(|e| e.to_string())?;
    }
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    copied(
        &destination.join("checksums.jsonl"),
        None,
        &crate::identity(&destination.join("checksums.jsonl"))?,
        None,
    )?;
    load_verified_bundle(destination, job)?;
    write_json_new(
        &destination.join("verification.json"),
        &json!({"integrity":true,"restore_verified":false,"errors":[]}),
    )?;
    copied(
        &destination.join("verification.json"),
        None,
        &crate::identity(&destination.join("verification.json"))?,
        None,
    )?;
    Ok(manifest)
}
pub fn discover_bundles(parent: &Path, job: &Job) -> Result<Vec<Value>, String> {
    absolute(parent)?;
    let mut stack = vec![(parent.to_owned(), 0)];
    let mut result = Vec::new();
    let mut visited = 0;
    job.phase("백업 폴더 검색", 0);
    while let Some((dir, depth)) = stack.pop() {
        job.check()?;
        visited += 1;
        if visited > 20000 {
            return Err("백업 검색 범위가 너무 큽니다. 더 가까운 상위 폴더를 선택하세요.".into());
        }
        ensure_plain(&dir)?;
        if dir.join("manifest.json").is_file() && dir.join("checksums.jsonl").is_file() {
            let manifest: Value = serde_json::from_slice(
                &fs::read(dir.join("manifest.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if manifest["format_version"] == 1 {
                result.push(json!({"path":dir,"manifest":manifest}));
            }
            continue;
        }
        if depth >= 5 {
            continue;
        }
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() && ensure_plain(&path).is_ok() {
                stack.push((path, depth + 1));
            }
        }
        job.advance(visited, "백업 폴더 검색", 0);
    }
    Ok(result)
}
pub fn manifest_tree(manifest: &Value, rid: &str) -> Result<Tree, String> {
    let files = manifest["inventory"]
        .as_array()
        .ok_or("백업 파일 목록 누락")?
        .iter()
        .filter(|v| v["root_id"] == rid)
        .map(|v| {
            let path = v["relative_path"].as_str().ok_or("파일 경로 누락")?;
            if !valid_bundle_path(path) {
                return Err("잘못된 백업 파일 경로".into());
            }
            Ok(Entry {
                identity: None,
                path: path.into(),
                size: v["size"].as_u64().ok_or("파일 크기 누락")?,
                sha256: v["sha256"].as_str().ok_or("파일 해시 누락")?.into(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let directories = manifest["directories"]
        .as_array()
        .ok_or("폴더 목록 누락")?
        .iter()
        .filter(|v| v[0] == rid)
        .map(|v| {
            v[1].as_str()
                .map(str::to_owned)
                .ok_or("폴더 경로 누락".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Tree { files, directories })
}

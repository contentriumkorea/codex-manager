//! Folder-local verified snapshots. Ordinary files already in the anchor are never copied.
#[path = "portable_workspace.rs"]
mod workspace;
use crate::{
    absolute, all_threads, digest, ensure_plain, map_workspace, roots, Catalog, Job, RolloutReader,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_key(s: &str) -> Result<&str, String> {
    if s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Ok(s)
    } else {
        Err("Invalid portable content key".into())
    }
}
fn string<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v[k].as_str()
        .ok_or_else(|| format!("Missing portable field: {k}"))
}
fn directory(anchor: &Path, namespace: &str) -> Result<PathBuf, String> {
    absolute(anchor)?;
    hash_key(namespace)?;
    let d = anchor.join(".codex-manager/projects").join(namespace);
    ensure_plain(&d)?;
    Ok(d)
}
fn read_json(path: &Path) -> Result<Value, String> {
    ensure_plain(path)?;
    let m = fs::metadata(path).map_err(|e| e.to_string())?;
    if m.len() > 32 * 1024 * 1024 {
        return Err("Portable metadata exceeds 32 MiB".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("Portable metadata exceeds 32 MiB; not published".into());
    }
    ensure_plain(path)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}
fn replace(from: &Path, to: &Path) -> Result<(), String> {
    ensure_plain(from)?;
    ensure_plain(to)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
        }
        let from = from.canonicalize().map_err(|e| e.to_string())?;
        let to = to
            .parent()
            .ok_or("Missing pointer parent")?
            .canonicalize()
            .map_err(|e| e.to_string())?
            .join(to.file_name().ok_or("Missing pointer name")?);
        let a: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 1 | 8) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to).map_err(|e| e.to_string())
    }
}
fn atomic_json(dir: &Path, name: &str, value: &Value) -> Result<(), String> {
    let tmp = dir.join(format!(".{}.partial", uuid::Uuid::new_v4()));
    write_new(&tmp, &serde_json::to_vec(value).map_err(|e| e.to_string())?)?;
    let result = replace(&tmp, &dir.join(name));
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
// OS-held exclusive lock covers publication and collection; process exit releases it automatically.
struct Lock {
    _file: File,
}
fn lock(dir: &Path) -> Result<Lock, String> {
    let p = dir.join("snapshot.lock");
    ensure_plain(&p)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    #[cfg(not(windows))]
    {
        return Err("Portable mutations require the supported Windows lock implementation".into());
    }
    Ok(Lock {
        _file: options
            .open(&p)
            .map_err(|e| format!("Portable folder is busy: {e}"))?,
    })
}
fn current(dir: &Path) -> Result<(Value, Value), String> {
    let p = read_json(&dir.join("current.json"))?;
    let id = hash_key(string(&p, "snapshotId")?)?;
    let m = read_json(&dir.join("manifests").join(format!("{id}.json")))?;
    if hash(&serde_json::to_vec(&m).map_err(|e| e.to_string())?) != id {
        return Err("Portable manifest checksum mismatch".into());
    }
    if m["schema"] != 1 || m["complete"] != true {
        return Err("Incomplete or unsupported portable manifest".into());
    }
    Ok((p, m))
}
fn mapped(anchor: &Path, original: &str, path: &str) -> Result<PathBuf, String> {
    absolute(Path::new(original))?;
    absolute(Path::new(path))?;
    let p = map_workspace(
        Path::new(path),
        &[(PathBuf::from(original), anchor.to_owned())],
    )?;
    if map_workspace(&p, &[(anchor.join(".codex-manager"), anchor.to_owned())]).is_ok() {
        return Err("Sidecar cannot be a working root".into());
    }
    absolute(&p)?;
    Ok(p)
}

/// Cheap metadata status; this does not certify current blob integrity. Use inspect before restore.
pub fn portable_status(anchor: &Path, namespace: &str) -> Result<Value, String> {
    let dir = directory(anchor, namespace)?;
    if !dir.join("current.json").exists() {
        let attempt = read_json(&dir.join("status.json"))?;
        return Ok(
            json!({"namespace":namespace,"anchor":anchor,"snapshotId":null,"lastSuccessfulSave":null,"complete":false,"integrityVerified":false,"project":attempt["project"],"lastAttempt":attempt}),
        );
    }
    let (p, m) = current(&dir)?;
    Ok(
        json!({"namespace":namespace,"anchor":anchor,"snapshotId":p["snapshotId"],"lastSuccessfulSave":m["createdAt"],"project":m["project"],"conversationCount":m["conversations"].as_array().map(Vec::len),"complete":true,"integrityVerified":false,"lastAttempt":if dir.join("status.json").exists(){read_json(&dir.join("status.json"))?}else{Value::Null}}),
    )
}

/// Verify the manifest, every immutable raw blob and all decoded records before trusting it.
pub fn portable_inspect(anchor: &Path, namespace: &str, job: &Job) -> Result<Value, String> {
    let dir = directory(anchor, namespace)?;
    let (p, mut m) = current(&dir)?;
    if m["namespace"] != namespace {
        return Err("Portable namespace mismatch".into());
    }
    workspace_links(anchor, job)?;
    workspace::verify(anchor, &dir, &m, job)?;
    let mut ids = HashSet::new();
    for r in m["roots"].as_array().ok_or("Missing portable roots")? {
        let original = r.as_str().ok_or("Invalid root")?;
        let path = workspace::map(anchor, &m, original)?;
        let external = m["externalRoots"]
            .as_array()
            .is_some_and(|a| a.iter().any(|r| r["original"] == original));
        if !external && !path.is_dir() {
            return Err(format!(
                "Portable working folder missing: {}",
                path.display()
            ));
        }
    }
    if !m["dependencies"]
        .as_array()
        .ok_or("Missing dependency report")?
        .is_empty()
    {
        return Err("Unresolved portable dependencies".into());
    }
    for t in m["conversations"]
        .as_array()
        .ok_or("Missing conversations")?
    {
        job.check()?;
        let id = string(t, "id")?;
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid thread UUID")?;
        if !ids.insert(id) {
            return Err("Duplicate portable thread UUID".into());
        }
        workspace::map(anchor, &m, string(t, "cwd")?)?;
        for r in t["runtimeRoots"]
            .as_array()
            .ok_or("Missing runtime roots")?
        {
            workspace::map(anchor, &m, r.as_str().ok_or("Invalid runtime root")?)?;
        }
        let key = hash_key(string(t, "sha256")?)?;
        let blob = dir.join("blobs").join(key);
        let mut reader = RolloutReader::open(&blob, Some(id), job)?;
        if reader.raw_sha256() != key
            || reader.raw_size() != t["size"].as_u64().ok_or("Missing blob size")?
        {
            return Err("Portable blob checksum mismatch".into());
        }
        let mut decoded = Sha256::new();
        while let Some(r) = reader.next_record()? {
            decoded.update(r.line.as_bytes());
        }
        if format!("{:x}", decoded.finalize()) != string(t, "decodedSha256")? {
            return Err("Portable decoded checksum mismatch".into());
        }
    }
    m["snapshotId"] = p["snapshotId"].clone();
    m["anchor"] = json!(anchor);
    m["integrityVerified"] = json!(true);
    Ok(m)
}

// Raw live read permits Codex's active handle, but binds bytes to identity and metadata on both ends.
fn live_hash(
    path: &Path,
    job: &Job,
    mut output: Option<&mut File>,
) -> Result<(String, String, u64), String> {
    ensure_plain(path)?;
    let before = fs::metadata(path).map_err(|e| e.to_string())?;
    let ident = crate::identity(path)?;
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    let mut sha = Sha256::new();
    let mut buf = vec![0; 1024 * 1024];
    let mut size = 0;
    loop {
        job.check()?;
        let n = input.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
        if let Some(out) = output.as_mut() {
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
        size += n as u64;
        job.advance(size, &path.to_string_lossy(), n as u64);
    }
    let after = fs::metadata(path).map_err(|e| e.to_string())?;
    ensure_plain(path)?;
    if ident != crate::identity(path)?
        || before.len() != size
        || after.len() != size
        || before.modified().ok() != after.modified().ok()
    {
        return Err("Source changed during portable capture".into());
    }
    Ok((format!("{:x}", sha.finalize()), ident, size))
}
fn scan_record(
    record: &Value,
    dependencies: &mut Vec<Value>,
    id: &str,
    runtime: &mut Vec<Value>,
) -> Result<(), String> {
    if record["type"] == "event_msg" {
        for key in ["local_images", "images", "attachments"] {
            if record["payload"][key]
                .as_array()
                .is_some_and(|a| !a.is_empty())
            {
                dependencies.push(
                    json!({"kind":"event_attachment","threadId":id,"field":key,"resolved":false}),
                );
            }
        }
    }
    let settings = match record["type"].as_str() {
        Some("session_meta" | "turn_context") => Some(&record["payload"]),
        Some("event_msg") if record["payload"]["type"] == "thread_settings_applied" => {
            Some(&record["payload"]["thread_settings"])
        }
        _ => None,
    };
    if let Some(s) = settings {
        if let Some(r) = s.get("runtime_workspace_roots") {
            *runtime = if r.is_null() {
                vec![]
            } else {
                r.as_array()
                    .ok_or("Invalid runtime roots in source record")?
                    .clone()
            };
        }
    }
    if record["type"] == "response_item" {
        fn paths(v: &Value, dependencies: &mut Vec<Value>, id: &str) {
            match v {
                Value::Object(o) => {
                    for (k, v) in o {
                        if [
                            "path",
                            "image_path",
                            "file_path",
                            "file_id",
                            "attachment_id",
                        ]
                        .contains(&k.as_str())
                            && v.is_string()
                        {
                            dependencies.push(json!({"kind":"attachment","threadId":id,"path":v,"resolved":false}));
                        } else if k == "image_url"
                            && v.as_str()
                                .or_else(|| v["url"].as_str())
                                .is_some_and(|s| !s.starts_with("data:"))
                        {
                            dependencies.push(json!({"kind":"attachment","threadId":id,"path":v,"resolved":false}));
                        } else {
                            paths(v, dependencies, id)
                        }
                    }
                }
                Value::Array(a) => {
                    for v in a {
                        paths(v, dependencies, id)
                    }
                }
                _ => {}
            }
        }
        paths(&record["payload"], dependencies, id);
    }
    Ok(())
}

fn workspace_links(anchor: &Path, job: &Job) -> Result<(), String> {
    let mut stack = vec![anchor.to_owned()];
    while let Some(dir) = stack.pop() {
        job.check()?;
        ensure_plain(&dir)?;
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            job.check()?;
            let p = entry.map_err(|e| e.to_string())?.path();
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(".codex-manager"))
            {
                continue;
            }
            ensure_plain(&p).map_err(|e| {
                format!(
                    "Unresolved linked workspace dependency {}: {e}",
                    p.display()
                )
            })?;
            if p.is_dir() {
                stack.push(p);
            }
        }
    }
    Ok(())
}

/// Snapshot all project threads and internal descendants; anchor files and global state are not copied.
/// Errors preserve current.json and are durably recorded in status.json when the sidecar is writable.
pub fn portable_snapshot(
    catalog: &Catalog,
    project_id: &str,
    anchor: &Path,
    job: &Job,
) -> Result<Value, String> {
    portable_snapshot_namespace(
        catalog,
        project_id,
        anchor,
        &hash(project_id.as_bytes()),
        job,
    )
}
fn portable_snapshot_namespace(
    catalog: &Catalog,
    project_id: &str,
    anchor: &Path,
    namespace: &str,
    job: &Job,
) -> Result<Value, String> {
    absolute(anchor)?;
    if !anchor.is_dir() {
        return Err("Portable anchor must be an existing folder".into());
    }
    let group = catalog
        .projects
        .iter()
        .find(|g| g.project["id"] == project_id)
        .ok_or("Project not found")?;
    hash_key(namespace)?;
    let dir = directory(anchor, namespace)?;
    fs::create_dir_all(dir.join("manifests")).map_err(|e| e.to_string())?;
    fs::create_dir_all(dir.join("blobs")).map_err(|e| e.to_string())?;
    let ignore = anchor.join(".codex-manager/.gitignore");
    ensure_plain(&ignore)?;
    if !ignore.exists() {
        write_new(&ignore, b"*\n")?;
    } else if fs::read_to_string(&ignore)
        .map_err(|e| e.to_string())?
        .trim()
        != "*"
    {
        return Err(
            "Existing sidecar .gitignore does not protect private snapshots; original preserved"
                .into(),
        );
    }
    let _lock = lock(&dir)?;
    let result = (|| {
        job.check()?;
        workspace_links(anchor, job)?;
        let project_roots = roots(&group.project)?;
        if !project_roots
            .iter()
            .any(|p| crate::clean_path(p) == crate::clean_path(anchor))
        {
            return Err("Anchor must be a selected project root".into());
        }
        let previous = if dir.join("current.json").exists() {
            Some(current(&dir)?)
        } else {
            None
        };
        let mut dependencies = vec![];
        let external_roots = workspace::capture(anchor, &dir, &project_roots, job)?;
        let location =
            json!({"namespace":namespace,"originalAnchor":anchor,"externalRoots":external_roots});
        let all = all_threads(catalog);
        let mut ids: HashSet<String> = group
            .threads
            .iter()
            .filter_map(|t| t["id"].as_str().map(str::to_owned))
            .collect();
        loop {
            let n = ids.len();
            for t in &all {
                if ["parentThreadId", "attachedThreadParentId"]
                    .iter()
                    .any(|k| t[*k].as_str().is_some_and(|id| ids.contains(id)))
                {
                    ids.insert(string(t, "id")?.into());
                }
            }
            if n == ids.len() {
                break;
            }
        }
        let mut selected: Vec<_> = all
            .iter()
            .filter(|t| t["id"].as_str().is_some_and(|id| ids.contains(id)))
            .collect();
        selected.sort_by_key(|t| t["id"].as_str());
        let mut records = vec![];
        let mut copied = 0;
        let mut reused = 0;
        for t in selected {
            job.check()?;
            let id = string(t, "id")?;
            uuid::Uuid::parse_str(id).map_err(|_| "Invalid thread UUID")?;
            if !t["projectId"].is_null() && t["projectId"] != project_id {
                dependencies
                    .push(json!({"kind":"cross_project_child","threadId":id,"resolved":false}));
            }
            if t["attachmentCount"].as_u64().unwrap_or(0) > 0
                || !t["attachedThreadParentId"].is_null()
            {
                dependencies
                    .push(json!({"kind":"thread_attachment","threadId":id,"resolved":false}));
            }
            if t["parentThreadId"]
                .as_str()
                .is_some_and(|p| !ids.contains(p))
            {
                dependencies.push(json!({"kind":"missing_parent","threadId":id,"parentId":t["parentThreadId"],"resolved":false}));
            }
            let context = |error: String| {
                format!(
                    "Thread [{}] ({id}) at {}: {error}",
                    t["name"]
                        .as_str()
                        .or_else(|| t["title"].as_str())
                        .unwrap_or("Untitled"),
                    t["path"].as_str().unwrap_or("unknown path")
                )
            };
            let source =
                crate::resolve_rollout_path(Path::new(string(t, "path")?)).map_err(&context)?;
            if map_workspace(
                &source,
                &[(anchor.join(".codex-manager"), anchor.to_owned())],
            )
            .is_ok()
            {
                return Err("Sidecar recursion rejected".into());
            }
            let (sha, ident, size) = live_hash(&source, job, None).map_err(&context)?;
            let blob = dir.join("blobs").join(&sha);
            let cached = previous
                .as_ref()
                .and_then(|(_, m)| m["conversations"].as_array())
                .and_then(|a| a.iter().find(|v| v["id"] == id && v["sha256"] == sha));
            let (mut runtime, mut deps, decoded) = if let Some(c) = cached {
                if digest(&blob, job)? != sha {
                    return Err("Previously verified portable blob changed".into());
                }
                reused += 1;
                (
                    c["runtimeRoots"]
                        .as_array()
                        .ok_or("Invalid cached roots")?
                        .clone(),
                    c["dependencies"]
                        .as_array()
                        .ok_or("Invalid cached dependencies")?
                        .clone(),
                    string(c, "decodedSha256")?.to_owned(),
                )
            } else {
                let mut reader =
                    RolloutReader::open_live(&source, Some(id), job).map_err(&context)?;
                if reader.raw_sha256() != sha || reader.raw_identity() != ident {
                    return Err("Source changed before verification".into());
                }
                let mut runtime = vec![];
                let mut deps = vec![];
                let mut decoded = Sha256::new();
                while let Some(r) = reader.next_record().map_err(&context)? {
                    decoded.update(r.line.as_bytes());
                    scan_record(&r.value, &mut deps, id, &mut runtime).map_err(&context)?;
                }
                if blob.exists() {
                    if digest(&blob, job)? != sha {
                        return Err("Portable content address collision or tamper".into());
                    }
                    reused += 1;
                } else {
                    let tmp = dir.join(format!(".{}.partial", uuid::Uuid::new_v4()));
                    let capture = (|| {
                        let mut out = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&tmp)
                            .map_err(|e| e.to_string())?;
                        let receipt = live_hash(&source, job, Some(&mut out))?;
                        if receipt != (sha.clone(), ident.clone(), size) {
                            return Err("Source changed while copying".into());
                        }
                        out.sync_all().map_err(|e| e.to_string())?;
                        drop(out);
                        if digest(&tmp, job)? != sha {
                            return Err("Copied blob checksum mismatch".into());
                        }
                        crate::rename_new(&tmp, &blob)
                    })();
                    if capture.is_err() {
                        let _ = fs::remove_file(&tmp);
                    }
                    capture?;
                    copied += 1;
                }
                (runtime, deps, format!("{:x}", decoded.finalize()))
            };
            let cwd = string(t, "cwd")?;
            if workspace::map(anchor, &location, cwd).is_err() {
                deps.push(json!({"kind":"external_cwd","threadId":id,"path":cwd,"resolved":false}));
            }
            for r in &mut runtime {
                if r.as_str()
                    .is_none_or(|p| workspace::map(anchor, &location, p).is_err())
                {
                    deps.push(
                        json!({"kind":"external_runtime","threadId":id,"path":r,"resolved":false}),
                    );
                }
            }
            dependencies.extend(deps.clone());
            records.push(json!({"id":id,"title":t["name"].as_str().unwrap_or("Untitled"),"archived":t["archived"].as_bool().unwrap_or(false),"internal":t["internal"].as_bool().unwrap_or(false),"parentId":t["parentThreadId"],"attachedThreadParentId":t["attachedThreadParentId"],"cwd":cwd,"runtimeRoots":runtime,"sha256":sha,"decodedSha256":decoded,"size":size,"dependencies":deps}));
        }
        if !dependencies.is_empty() {
            return Err(format!(
                "Unresolved portable dependencies: {}",
                json!(dependencies)
            ));
        }
        let manifest = json!({"schema":1,"namespace":namespace,"createdAt":chrono::Utc::now().to_rfc3339(),"complete":true,"project":{"id":project_id,"name":group.project["name"]},"originalAnchor":anchor,"roots":project_roots,"externalRoots":external_roots,"conversations":records,"dependencies":[]});
        let bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
        let sid = hash(&bytes);
        let file = dir.join("manifests").join(format!("{sid}.json"));
        write_new(&file, &bytes)?;
        // All referenced blobs have a verified receipt before the sole publication point.
        job.check()?;
        let pointer = json!({"snapshotId":sid,"previous":previous.as_ref().map(|(p,_)|p["snapshotId"].clone())});
        atomic_json(&dir, "current.json", &pointer)?;
        let gc = collect(&dir, &pointer, job).err();
        Ok(
            json!({"namespace":namespace,"snapshotId":sid,"complete":true,"lastSuccessfulSave":manifest["createdAt"],"conversationCount":records.len(),"copiedBlobs":copied,"reusedBlobs":reused,"retentionWarning":gc}),
        )
    })();
    let status = match &result {
        Ok(v) => {
            json!({"state":"saved","at":chrono::Utc::now().to_rfc3339(),"snapshotId":v["snapshotId"],"retentionWarning":v["retentionWarning"],"project":{"id":project_id,"name":group.project["name"]}})
        }
        Err(e) => {
            json!({"state":"failed","at":chrono::Utc::now().to_rfc3339(),"error":e,"project":{"id":project_id,"name":group.project["name"]}})
        }
    };
    let _ = atomic_json(&dir, "status.json", &status);
    result
}
fn collect(dir: &Path, pointer: &Value, job: &Job) -> Result<(), String> {
    let keep: HashSet<_> = [pointer["snapshotId"].as_str(), pointer["previous"].as_str()]
        .into_iter()
        .flatten()
        .collect();
    let mut blobs = HashSet::new();
    let mut retained_manifests = vec![];
    for id in &keep {
        hash_key(id)?;
        let m = read_json(&dir.join("manifests").join(format!("{id}.json")))?;
        if hash(&serde_json::to_vec(&m).map_err(|e| e.to_string())?) != *id {
            return Err("Retained manifest changed; collection blocked".into());
        }
        retained_manifests.push(m.clone());
        for t in m["conversations"]
            .as_array()
            .ok_or("Invalid retained conversations")?
        {
            blobs.insert(hash_key(string(t, "sha256")?)?.to_owned());
        }
    }
    for e in fs::read_dir(dir.join("manifests")).map_err(|e| e.to_string())? {
        job.check()?;
        let p = e.map_err(|e| e.to_string())?.path();
        ensure_plain(&p)?;
        let Some(id) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if p.extension().is_some_and(|e| e == "json") && hash_key(id).is_ok() && !keep.contains(id)
        {
            let m = read_json(&p)?;
            if hash(&serde_json::to_vec(&m).map_err(|e| e.to_string())?) == id {
                fs::remove_file(&p).map_err(|e| e.to_string())?;
            }
        }
    }
    for e in fs::read_dir(dir.join("blobs")).map_err(|e| e.to_string())? {
        job.check()?;
        let p = e.map_err(|e| e.to_string())?.path();
        ensure_plain(&p)?;
        let Some(key) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if hash_key(key).is_ok() && !blobs.contains(key) && digest(&p, job)? == key {
            fs::remove_file(p).map_err(|e| e.to_string())?;
        }
    }
    workspace::collect(dir, &retained_manifests, job)?;
    Ok(())
}
/// Bounded metadata-only discovery. Diagnostics are explicit; inspect verifies selected snapshots.
pub fn portable_discover(
    parent: &Path,
    max_depth: usize,
    max_folders: usize,
    job: &Job,
) -> Result<Value, String> {
    absolute(parent)?;
    if max_depth > 32 || max_folders == 0 || max_folders > 100_000 {
        return Err("Invalid discovery limits".into());
    }
    let mut stack = vec![(parent.to_owned(), 0)];
    let mut projects = vec![];
    let mut diagnostics = vec![];
    let mut visited = 0;
    let mut truncated = false;
    while let Some((dir, depth)) = stack.pop() {
        job.check()?;
        if visited >= max_folders {
            truncated = true;
            break;
        }
        visited += 1;
        if let Err(e) = ensure_plain(&dir) {
            diagnostics.push(json!({"path":dir,"error":e}));
            continue;
        }
        let side = dir.join(".codex-manager/projects");
        if side.exists() {
            let found = (|| {
                ensure_plain(&side)?;
                for e in fs::read_dir(&side).map_err(|e| e.to_string())? {
                    let p = e.map_err(|e| e.to_string())?.path();
                    let ns = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .ok_or("Invalid namespace")?;
                    match portable_status(&dir, ns) {
                        Ok(v) => projects.push(v),
                        Err(e) => diagnostics.push(json!({"path":p,"error":e})),
                    }
                }
                Ok::<_, String>(())
            })();
            if let Err(e) = found {
                diagnostics.push(json!({"path":side,"error":e}));
            }
        }
        if depth == max_depth {
            continue;
        }
        match fs::read_dir(&dir) {
            Ok(entries) => {
                for e in entries {
                    match e {
                        Ok(e) => {
                            let p = e.path();
                            if p.file_name()
                                .is_some_and(|s| s == ".codex-manager" || s == ".git")
                            {
                                continue;
                            }
                            match e.file_type() {
                                Ok(t) if t.is_dir() || t.is_symlink() => stack.push((p, depth + 1)),
                                Err(e) => diagnostics.push(json!({"path":p,"error":e.to_string()})),
                                _ => {}
                            }
                        }
                        Err(e) => diagnostics.push(json!({"path":dir,"error":e.to_string()})),
                    }
                }
            }
            Err(e) => diagnostics.push(json!({"path":dir,"error":e.to_string()})),
        }
    }
    Ok(
        json!({"projects":projects,"diagnostics":diagnostics,"visitedFolders":visited,"truncated":truncated}),
    )
}

use crate::{Adapter, ThreadImport};
use std::collections::HashMap;
fn mapped_thread(anchor: &Path, m: &Value, t: &Value) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let cwd = workspace::map(anchor, &m, string(t, "cwd")?)?;
    let runtime = t["runtimeRoots"]
        .as_array()
        .ok_or("Missing runtime roots")?
        .iter()
        .map(|r| workspace::map(anchor, m, r.as_str().ok_or("Invalid runtime root")?))
        .collect::<Result<Vec<_>, String>>()?;
    Ok((cwd, runtime))
}
fn metadata_matches(
    existing: &Value,
    t: &Value,
    cwd: &Path,
    runtime: &[PathBuf],
    target: Option<&str>,
    job: &Job,
) -> Result<bool, String> {
    if existing["projectId"].as_str() != target
        || target.is_none()
        || existing["name"] != t["title"]
        || existing["archived"].as_bool().unwrap_or(false)
            != t["archived"].as_bool().unwrap_or(false)
        || existing["parentThreadId"] != t["parentId"]
        || existing["internal"].as_bool().unwrap_or(false)
            != t["internal"].as_bool().unwrap_or(false)
        || crate::clean_path(Path::new(string(existing, "cwd")?)) != crate::clean_path(cwd)
    {
        return Ok(false);
    }
    Ok(crate::runtime_roots_with_job(Path::new(string(existing, "path")?), job)? == runtime)
}
impl Adapter {
    fn portable_owned_content(
        &self,
        operation: &str,
        thread: &Value,
        job: &Job,
    ) -> Result<bool, String> {
        if !self.owns_rollout(operation, thread, job)? {
            return Ok(false);
        }
        let owner = self
            .rollout_owner(operation, string(thread, "id")?)?
            .ok_or("Missing ownership receipt")?;
        let expected = string(&owner, "decodedSha256")?;
        Ok(crate::decoded_rollout_digest(Path::new(string(thread, "path")?), job)? == expected)
    }
    /// Read-only against the Codex store; serialise this writer with other service jobs.
    pub fn portable_snapshot(
        &self,
        project_id: &str,
        anchor: &Path,
        job: &Job,
    ) -> Result<Value, String> {
        if self
            .protected_storage()
            .iter()
            .any(|p| crate::paths_overlap(anchor, p))
        {
            return Err("Portable anchor overlaps protected Codex/Manager storage".into());
        }
        let catalog = self.snapshot()?;
        let project = &catalog
            .projects
            .iter()
            .find(|g| g.project["id"] == project_id)
            .ok_or("Project not found")?
            .project;
        if roots(project)?.iter().any(|r| {
            self.protected_storage()
                .iter()
                .any(|p| crate::paths_overlap(r, p))
        }) {
            return Err("Connected root overlaps protected Codex/Manager storage".into());
        }
        let namespace = self.portable_namespace(project_id, anchor)?;
        portable_snapshot_namespace(&catalog, project_id, anchor, &namespace, job)
    }
    /// Trusted portable lineage for UI status and subsequent saves. Explicit merges keep the target's own namespace.
    pub fn portable_namespace(&self, project_id: &str, anchor: &Path) -> Result<String, String> {
        absolute(anchor)?;
        let catalog = self.snapshot()?;
        let project = &catalog
            .projects
            .iter()
            .find(|g| g.project["id"] == project_id)
            .ok_or("Project not found for portable lineage")?
            .project;
        if !roots(project)?
            .iter()
            .any(|r| crate::clean_path(r) == crate::clean_path(anchor))
        {
            return Err("Portable lineage anchor is not a current project root".into());
        }
        let mut candidates = HashMap::new();
        for op in self
            .journal
            .operations_for("rust-portable-restore", &self.store.home)?
        {
            let p = &op["payload"];
            if p["kind"] != "rust-portable-restore"
                || op["state"] != "completed"
                || p["home"] != json!(self.store.home)
                || !p["target"].is_null()
            {
                continue;
            }
            let events = self.journal.events(string(&op, "id")?)?;
            if events
                .iter()
                .any(|e| e["phase"] == "completed" && e["payload"]["projectId"] == project_id)
            {
                candidates.insert(
                    hash_key(string(p, "namespace")?)?.to_owned(),
                    p["sourceProjectId"].clone(),
                );
            }
        }
        if candidates.len() > 1 {
            return Err("Multiple restored portable lineages claim this project; explicit resolution is required".into());
        }
        if let Some((namespace, source_id)) = candidates.into_iter().next() {
            let dir = directory(anchor, &namespace)?;
            let (_, m) = current(&dir)?;
            if m["project"]["id"] != project_id && m["project"]["id"] != source_id {
                return Err("Portable lineage now belongs to a different project; automatic overwrite blocked".into());
            }
            return Ok(namespace);
        }
        Ok(hash(project_id.as_bytes()))
    }
    /// Read-only conflict preview. An existing UUID is skipped only with full-content and metadata proof.
    pub fn portable_restore_plan(
        &self,
        anchor: &Path,
        namespace: &str,
        target: Option<&str>,
        job: &Job,
    ) -> Result<Value, String> {
        let m = portable_inspect(anchor, namespace, job)?;
        self.portable_plan_verified(anchor, &m, target, None, job)
    }
    fn portable_plan_verified(
        &self,
        anchor: &Path,
        m: &Value,
        target: Option<&str>,
        resume: Option<&str>,
        job: &Job,
    ) -> Result<Value, String> {
        if self
            .protected_storage()
            .iter()
            .any(|p| crate::paths_overlap(anchor, p))
        {
            return Err("Portable anchor overlaps protected storage".into());
        }
        let catalog = self.snapshot()?;
        if let Some(pid) = target {
            if !catalog.projects.iter().any(|p| p.project["id"] == pid) {
                return Err("Restore target project does not exist".into());
            }
        }
        let existing = all_threads(&catalog);
        let history = self
            .journal
            .operations_for("rust-portable-restore", &self.store.home)?;
        let mut conflicts = workspace::conflicts(
            self,
            anchor,
            &directory(anchor, string(m, "namespace")?)?,
            m,
            resume,
            job,
        )?;
        let mut skipped = vec![];
        let mut importing = vec![];
        for t in m["conversations"]
            .as_array()
            .ok_or("Missing conversations")?
        {
            job.check()?;
            let id = string(t, "id")?;
            let (cwd, runtime) = mapped_thread(anchor, m, t)?;
            if let Some(e) = existing.iter().find(|e| e["id"] == id) {
                let mut owned = false;
                for op in &history {
                    let p = &op["payload"];
                    if p["kind"] == "rust-portable-restore"
                        && p["home"] == json!(self.store.home)
                        && p["snapshotId"] == m["snapshotId"]
                        && p["anchor"] == json!(anchor)
                        && (op["state"] == "completed" || op["id"].as_str() == resume)
                        && self.portable_owned_content(string(op, "id")?, e, job)?
                    {
                        owned = true;
                        break;
                    }
                }
                if resume.is_some() && owned {
                    importing.push(json!(id));
                    continue;
                }
                let same = metadata_matches(e, t, &cwd, &runtime, target, job)?
                    && (owned
                        || crate::decoded_rollout_digest(Path::new(string(e, "path")?), job)?
                            == string(t, "decodedSha256")?);
                if same {
                    skipped.push(json!(id));
                } else {
                    conflicts.push(json!({"threadId":id,"reason":"Existing UUID has different full content, metadata, active paths or project association","existingProjectId":e["projectId"]}));
                }
            } else {
                importing.push(json!(id));
            }
        }
        let destinations = m["roots"]
            .as_array()
            .ok_or("Missing roots")?
            .iter()
            .map(|r| workspace::map(anchor, m, r.as_str().ok_or("Invalid root")?))
            .collect::<Result<Vec<_>, String>>()?;
        Ok(
            json!({"namespace":m["namespace"],"snapshotId":m["snapshotId"],"anchor":anchor,"project":m["project"],"roots":destinations,"conversationCount":m["conversations"].as_array().unwrap().len(),"targetProjectId":target,"conflicts":conflicts,"skipped":skipped,"importing":importing,"canRestore":conflicts.is_empty()}),
        )
    }
    /// Journaled restore into the existing anchor; resume using the operationId from journal history.
    pub fn portable_restore(
        &mut self,
        anchor: &Path,
        namespace: &str,
        target: Option<&str>,
        resume: Option<&str>,
        job: &Job,
    ) -> Result<Value, String> {
        self.guard()?;
        self.release_server();
        job.check()?;
        let dir = directory(anchor, namespace)?;
        let _lock = lock(&dir)?;
        let m = portable_inspect(anchor, namespace, job)?;
        let previous = resume
            .map(|id| {
                self.journal
                    .operation(id)
                    .and_then(|v| v.ok_or("Portable restore journal missing".into()))
            })
            .transpose()?;
        if let Some(op) = &previous {
            let p = &op["payload"];
            if p["kind"] != "rust-portable-restore"
                || p["home"] != json!(self.store.home)
                || p["anchor"] != json!(anchor)
                || p["snapshotId"] != m["snapshotId"]
                || p["target"].as_str() != target
                || op["state"] == "cancelled"
            {
                return Err("Restore resume settings differ from recorded operation".into());
            }
            if op["state"] == "completed" {
                return Err("Restore operation already completed; inspect current conflicts before a new restore".into());
            }
        }
        let plan = self.portable_plan_verified(anchor, &m, target, resume, job)?;
        if !plan["conflicts"].as_array().unwrap().is_empty() {
            return Err(format!("Portable restore conflicts: {}", plan["conflicts"]));
        }
        let destinations: Vec<PathBuf> = plan["roots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| PathBuf::from(v.as_str().unwrap()))
            .collect();
        let target_before = if let Some(id) = target {
            self.project(id)?
        } else {
            Value::Null
        };
        let op = if let Some(id) = resume {
            id.to_owned()
        } else {
            self.begin(json!({"kind":"rust-portable-restore","namespace":namespace,"sourceProjectId":m["project"]["id"],"snapshotId":m["snapshotId"],"anchor":anchor,"target":target,"targetBefore":target_before,"skipped":plan["skipped"],"projectKey":uuid::Uuid::new_v4().to_string(),"resources":[format!("portable:{}:{namespace}",anchor.display())]}))?
        };
        let payload = self
            .journal
            .operation(&op)?
            .ok_or("Missing restore operation")?["payload"]
            .clone();
        let skipped: HashSet<String> = payload["skipped"]
            .as_array()
            .ok_or("Missing skip receipt")?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let result = (|| {
            job.check()?;
            workspace::materialize(self, anchor, &dir, &m, &op, job)?;
            let project = if let Some(pid) = target {
                let mut p = self.project(pid)?;
                let before = roots(&payload["targetBefore"])?;
                let mut after = before.clone();
                for d in &destinations {
                    if !after
                        .iter()
                        .any(|r| map_workspace(d, &[(r.clone(), r.clone())]).is_ok())
                    {
                        after.push(d.clone());
                    }
                }
                let current = roots(&p)?;
                if current != before && current != after {
                    return Err("Target project roots changed since restore began".into());
                }
                p["roots"] = json!(after.iter().map(|p| json!({"path":p})).collect::<Vec<_>>());
                self.update_project(&p)?;
                p
            } else {
                let p = self.create_key(
                    string(&m["project"], "name")?,
                    &destinations,
                    string(&payload, "projectKey")?,
                )?;
                if roots(&p)? != destinations {
                    return Err("Restoring project roots changed".into());
                }
                p
            };
            let pid = string(&project, "id")?.to_owned();
            self.journal
                .record(&op, "portable-project-created", json!({"projectId":pid}))?;
            let mut assignments = HashMap::new();
            let conversations = m["conversations"].as_array().unwrap();
            job.phase_items("Portable conversations restore", conversations.len() as u64);
            for (i, t) in conversations.iter().enumerate() {
                job.check()?;
                let id = string(t, "id")?;
                if skipped.contains(id) {
                    continue;
                }
                let (cwd, runtime) = mapped_thread(anchor, &m, t)?;
                let source = dir.join("blobs").join(hash_key(string(t, "sha256")?)?);
                self.import_owned(
                    &ThreadImport {
                        source: &source,
                        id,
                        cwd: &cwd,
                        runtime: &runtime,
                        project: Some(&pid),
                        previous_project: None,
                        title: string(t, "title")?,
                        archived: t["archived"] == true,
                        expected_sha256: Some(string(t, "sha256")?),
                    },
                    &op,
                    job,
                )?;
                assignments.insert(id.to_owned(), Some(pid.clone()));
                job.advance((i + 1) as u64, id, 0);
            }
            self.release_server();
            let active = all_threads(&self.snapshot()?);
            for t in conversations {
                let id = string(t, "id")?;
                let e = active
                    .iter()
                    .find(|e| e["id"] == id)
                    .ok_or("Restored thread missing")?;
                let (cwd, runtime) = mapped_thread(anchor, &m, t)?;
                if !metadata_matches(e, t, &cwd, &runtime, Some(&pid), job)? {
                    return Err(format!("Restored metadata or graph differs for {id}"));
                }
                let source = dir.join("blobs").join(string(t, "sha256")?);
                if crate::messages_digest(Path::new(string(e, "path")?), job)?
                    != crate::messages_digest(&source, job)?
                {
                    return Err(format!("Restored messages differ for {id}"));
                }
            }
            self.sync(&[project], &assignments, &[], &[])?;
            let result = json!({"operationId":op,"projectId":pid,"anchor":anchor,"snapshotId":m["snapshotId"],"imported":conversations.len()-skipped.len(),"skipped":skipped.len(),"verified":true});
            self.journal.record(&op, "completed", result.clone())?;
            Ok(result)
        })();
        if let Err(e) = &result {
            self.release_server();
            self.journal
                .record(&op, "needs_recovery", json!({"error":e,"operationId":op}))?;
        }
        result.map_err(|e| format!("{e}\nPortable restore operation: {op}"))
    }
}
#[cfg(test)]
mod portable_metadata_tests {
    use super::*;
    #[test]
    fn oversized_metadata_is_rejected_before_publication() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&dir).unwrap();
        assert!(write_new(
            &dir.join("metadata.json"),
            &vec![b' '; 32 * 1024 * 1024 + 1]
        )
        .is_err());
        assert!(!dir.join("metadata.json").exists());
    }
}

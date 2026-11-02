//! Immutable external-root payloads and non-overwriting visible restore folders.
use super::*;
use crate::{Entry, Tree};
fn tree_value(tree: &Tree) -> Value {
    json!({"directories":tree.directories,"files":tree.files.iter().map(|e|json!({"path":e.path,"size":e.size,"sha256":e.sha256})).collect::<Vec<_>>()})
}
fn tree_read(value: &Value) -> Result<Tree, String> {
    let mut names = HashSet::new();
    let mut directories = vec![];
    let mut files = vec![];
    for d in value["directories"]
        .as_array()
        .ok_or("Missing workspace directories")?
    {
        let p = d.as_str().ok_or("Invalid workspace directory")?;
        relative(p)?;
        if !names.insert(p.to_lowercase()) {
            return Err("Duplicate workspace path".into());
        }
        directories.push(p.to_owned());
    }
    for e in value["files"].as_array().ok_or("Missing workspace files")? {
        let p = string(e, "path")?;
        relative(p)?;
        if !names.insert(p.to_lowercase()) {
            return Err("Duplicate workspace path".into());
        }
        files.push(Entry {
            path: p.to_owned(),
            size: e["size"].as_u64().ok_or("Invalid workspace size")?,
            sha256: hash_key(string(e, "sha256")?)?.into(),
            identity: None,
        });
    }
    directories.sort();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for e in &files {
        let mut parent = Path::new(&e.path).parent();
        while let Some(p) = parent {
            if p.as_os_str().is_empty() {
                break;
            }
            let rel = p.to_string_lossy().replace('\\', "/");
            if !directories.contains(&rel) {
                return Err("Workspace file has undeclared parent".into());
            }
            parent = p.parent();
        }
    }
    Ok(Tree { directories, files })
}
fn relative(p: &str) -> Result<(), String> {
    if !crate::valid_bundle_path(p)
        || p.split('/')
            .any(|c| c.eq_ignore_ascii_case(".codex-manager"))
    {
        return Err("Unsafe workspace relative path".into());
    }
    Ok(())
}
fn scan(source: &Path, job: &Job) -> Result<Tree, String> {
    absolute(source)?;
    if !source.is_dir() {
        return Err(format!("Connected root missing: {}", source.display()));
    }
    let mut tree = Tree {
        files: vec![],
        directories: vec![],
    };
    let mut stack = vec![source.to_owned()];
    while let Some(dir) = stack.pop() {
        job.check()?;
        ensure_plain(&dir)?;
        for e in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let p = e.map_err(|e| e.to_string())?.path();
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(".codex-manager"))
            {
                continue;
            }
            ensure_plain(&p)?;
            let rel = p
                .strip_prefix(source)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            relative(&rel)?;
            let meta = fs::metadata(&p).map_err(|e| e.to_string())?;
            if meta.is_dir() {
                tree.directories.push(rel);
                stack.push(p);
            } else if meta.is_file() {
                tree.files.push(Entry {
                    path: rel,
                    size: meta.len(),
                    sha256: digest(&p, job)?,
                    identity: Some(crate::identity(&p)?),
                });
            } else {
                return Err("Unsupported connected-root file type".into());
            }
        }
    }
    tree.directories.sort();
    tree.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(tree)
}
fn copy_payload(source: &Path, destination: &Path, tree: &Tree, job: &Job) -> Result<(), String> {
    ensure_plain(destination)?;
    fs::create_dir(destination).map_err(|e| e.to_string())?;
    for d in &tree.directories {
        let p = destination.join(d);
        ensure_plain(&p)?;
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let total = crate::tree_size(tree)?;
    let mut done = 0;
    job.phase("Connected workspace copy", total);
    for e in &tree.files {
        crate::copy_file(
            &source.join(&e.path),
            &destination.join(&e.path),
            e,
            job,
            &mut done,
            total,
        )?;
    }
    Ok(())
}
fn tree_hash(value: &Value) -> Result<String, String> {
    Ok(hash(&serde_json::to_vec(value).map_err(|e| e.to_string())?))
}
fn verified_payload(dir: &Path, root: &Value, job: &Job) -> Result<(PathBuf, Tree), String> {
    let key = hash_key(string(root, "treeHash")?)?;
    let folder = dir.join("workspaces").join(key);
    let receipt = read_json(&folder.join("receipt.json"))?;
    let tree = tree_read(&receipt["tree"])?;
    if tree_hash(&tree_value(&tree))? != key {
        return Err("Workspace receipt checksum mismatch".into());
    }
    let data = folder.join("data");
    ensure_plain(&data)?;
    let actual = crate::scan_tree(&data, job)?;
    if tree_value(&actual) != tree_value(&tree) {
        return Err("Workspace payload bytes changed".into());
    }
    Ok((data, tree))
}
pub(super) fn capture(
    anchor: &Path,
    dir: &Path,
    roots: &[PathBuf],
    job: &Job,
) -> Result<Vec<Value>, String> {
    let mut external = vec![];
    for r in roots {
        if r.join(".git").is_file() {
            return Err(format!("Unresolved linked worktree: {}", r.display()));
        }
        if mapped(anchor, &anchor.to_string_lossy(), &r.to_string_lossy()).is_ok() {
            continue;
        }
        if crate::paths_overlap(anchor, r)
            || external.iter().any(|v: &Value| {
                crate::paths_overlap(Path::new(v["original"].as_str().unwrap()), r)
            })
        {
            return Err("Overlapping connected workspace roots are unsupported".into());
        }
        let tree = scan(r, job)?;
        let tv = tree_value(&tree);
        let key = tree_hash(&tv)?;
        let id = hash(
            crate::clean_path(r)
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase()
                .as_bytes(),
        );
        let root = json!({"original":r,"id":id,"treeHash":key,"fileCount":tree.files.len(),"bytes":crate::tree_size(&tree)?});
        let folder = dir.join("workspaces").join(&key);
        ensure_plain(&folder)?;
        if folder.exists() {
            verified_payload(dir, &root, job)?;
        } else {
            let store = dir.join("workspaces");
            ensure_plain(&store)?;
            fs::create_dir_all(&store).map_err(|e| e.to_string())?;
            let stage = store.join(format!(".{}.partial", uuid::Uuid::new_v4()));
            fs::create_dir(&stage).map_err(|e| e.to_string())?;
            copy_payload(r, &stage.join("data"), &tree, job)?;
            if tree_value(&scan(r, job)?) != tv
                || tree_value(&crate::scan_tree(&stage.join("data"), job)?) != tv
            {
                return Err("Connected workspace changed during capture".into());
            }
            write_new(
                &stage.join("receipt.json"),
                &serde_json::to_vec(
                    &json!({"tree":tv,"identity":crate::identity(&stage.join("data"))?}),
                )
                .map_err(|e| e.to_string())?,
            )?;
            job.check()?;
            crate::rename_new(&stage, &folder)?;
        }
        external.push(root);
    }
    Ok(external)
}
fn external_roots(m: &Value) -> Result<&[Value], String> {
    match m.get("externalRoots") {
        None => Ok(&[]),
        Some(v) => v
            .as_array()
            .map(Vec::as_slice)
            .ok_or("Invalid external roots".into()),
    }
}
pub(super) fn mappings(anchor: &Path, m: &Value) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let original = PathBuf::from(string(m, "originalAnchor")?);
    absolute(&original)?;
    let ns = hash_key(string(m, "namespace")?)?;
    let mut mapping = vec![(original.clone(), anchor.to_owned())];
    let mut ids = HashSet::new();
    for r in external_roots(m)? {
        let source = PathBuf::from(string(r, "original")?);
        absolute(&source)?;
        let id = hash_key(string(r, "id")?)?;
        if !ids.insert(id)
            || mapping
                .iter()
                .any(|(p, _)| crate::paths_overlap(p, &source))
        {
            return Err("Overlapping or duplicate external root mapping".into());
        }
        let destination = anchor.join("Connected workspaces").join(ns).join(id);
        absolute(&destination)?;
        mapping.push((source, destination));
    }
    Ok(mapping)
}
pub(super) fn map(anchor: &Path, m: &Value, path: &str) -> Result<PathBuf, String> {
    absolute(Path::new(path))?;
    let result = map_workspace(Path::new(path), &mappings(anchor, m)?)?;
    absolute(&result)?;
    if map_workspace(
        &result,
        &[(anchor.join(".codex-manager"), anchor.to_owned())],
    )
    .is_ok()
    {
        return Err("Sidecar working root is forbidden".into());
    }
    Ok(result)
}
pub(super) fn verify(anchor: &Path, dir: &Path, m: &Value, job: &Job) -> Result<(), String> {
    mappings(anchor, m)?;
    for r in external_roots(m)? {
        verified_payload(dir, r, job)?;
    }
    Ok(())
}
fn has_receipt(
    adapter: &Adapter,
    m: &Value,
    anchor: &Path,
    destination: &Path,
    root: &Value,
    resume: Option<&str>,
) -> Result<bool, String> {
    for op in adapter
        .journal
        .operations_for("rust-portable-restore", &adapter.store.home)?
    {
        let p = &op["payload"];
        if p["kind"] != "rust-portable-restore"
            || p["home"] != json!(adapter.store.home)
            || p["anchor"] != json!(anchor)
            || p["snapshotId"] != m["snapshotId"]
            || !(op["state"] == "completed" || op["id"].as_str() == resume)
        {
            continue;
        }
        for e in adapter.journal.events(string(&op, "id")?)? {
            if e["phase"] == "portable-workspace-created"
                && e["payload"]["destination"] == json!(destination)
                && e["payload"]["treeHash"] == root["treeHash"]
                && e["payload"]["identity"] == json!(crate::identity(destination)?)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
pub(super) fn conflicts(
    adapter: &Adapter,
    anchor: &Path,
    dir: &Path,
    m: &Value,
    resume: Option<&str>,
    job: &Job,
) -> Result<Vec<Value>, String> {
    let mut out = vec![];
    for r in external_roots(m)? {
        let dest = map(anchor, m, string(r, "original")?)?;
        if dest.exists() {
            let (_, tree) = verified_payload(dir, r, job)?;
            let receipt = has_receipt(adapter, m, anchor, &dest, r, resume)?;
            let actual = crate::scan_tree(&dest, job)?;
            let compatible = if resume.is_some() {
                actual.files.iter().all(|f| {
                    tree.files
                        .iter()
                        .any(|e| e.path == f.path && e.size == f.size && e.sha256 == f.sha256)
                }) && actual
                    .directories
                    .iter()
                    .all(|d| tree.directories.contains(d))
            } else {
                tree_value(&actual) == tree_value(&tree)
            };
            if !receipt || !compatible {
                out.push(json!({"rootId":r["id"],"path":dest,"reason":"Connected workspace destination already exists without matching owned content; no overwrite"}));
            }
        }
    }
    Ok(out)
}
pub(super) fn materialize(
    adapter: &Adapter,
    anchor: &Path,
    dir: &Path,
    m: &Value,
    operation: &str,
    job: &Job,
) -> Result<(), String> {
    for r in external_roots(m)? {
        job.check()?;
        let dest = map(anchor, m, string(r, "original")?)?;
        let (data, tree) = verified_payload(dir, r, job)?;
        if !dest.exists() {
            let parent = dest.parent().ok_or("Missing connected root parent")?;
            ensure_plain(parent)?;
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            adapter.journal.record(
                operation,
                "portable-workspace-intent",
                json!({"destination":dest,"treeHash":r["treeHash"]}),
            )?;
            fs::create_dir(&dest).map_err(|e| e.to_string())?;
            adapter.journal.record(operation,"portable-workspace-created",json!({"destination":dest,"treeHash":r["treeHash"],"identity":crate::identity(&dest)?}))?;
        } else if !has_receipt(adapter, m, anchor, &dest, r, Some(operation))? {
            return Err("Connected workspace has no owned restore receipt".into());
        }
        crate::resume_copy_tree(&data, &dest, &tree, job)?;
    }
    Ok(())
}
// Only immutable sidecar payloads are collected. Visible materialized folders are never candidates.
pub(super) fn collect(dir: &Path, manifests: &[Value], job: &Job) -> Result<(), String> {
    let store = dir.join("workspaces");
    if !store.exists() {
        return Ok(());
    }
    ensure_plain(&store)?;
    let mut retained = HashSet::new();
    for m in manifests {
        for r in external_roots(m)? {
            retained.insert(string(r, "treeHash")?.to_owned());
        }
    }
    let mut warnings = vec![];
    for entry in fs::read_dir(&store).map_err(|e| e.to_string())? {
        job.check()?;
        let p = entry.map_err(|e| e.to_string())?.path();
        ensure_plain(&p)?;
        let key = p
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid workspace payload name")?;
        if retained.contains(key) {
            continue;
        }
        if hash_key(key).is_err() {
            warnings.push(format!(
                "Unfinished/unowned workspace retained: {}",
                p.display()
            ));
            continue;
        }
        let result = (|| {
            let (data, tree) = verified_payload(dir, &json!({"treeHash":key}), job)?;
            for e in &tree.files {
                delete_owned(&data.join(&e.path), e, job)?;
            }
            let mut dirs = tree.directories.clone();
            dirs.sort_by_key(|s| std::cmp::Reverse(s.split('/').count()));
            for d in dirs {
                ensure_plain(&data.join(&d))?;
                fs::remove_dir(data.join(d)).map_err(|e| e.to_string())?;
            }
            fs::remove_dir(&data).map_err(|e| e.to_string())?;
            ensure_plain(&p.join("receipt.json"))?;
            fs::remove_file(p.join("receipt.json")).map_err(|e| e.to_string())?;
            fs::remove_dir(&p).map_err(|e| e.to_string())?;
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            warnings.push(format!("Workspace retained: {}: {e}", p.display()));
        }
    }
    if warnings.is_empty() {
        Ok(())
    } else {
        Err(warnings.join("\n"))
    }
}
fn delete_owned(path: &Path, entry: &Entry, job: &Job) -> Result<(), String> {
    ensure_plain(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        #[link(name = "kernel32")]
        extern "system" {
            fn SetFileInformationByHandle(
                file: *mut std::ffi::c_void,
                class: i32,
                info: *const i32,
                size: u32,
            ) -> i32;
        }
        let mut f = OpenOptions::new()
            .read(true)
            .access_mode(0x80010000)
            .share_mode(1)
            .open(path)
            .map_err(|e| e.to_string())?;
        let mut sha = Sha256::new();
        let mut size = 0;
        let mut buf = vec![0; 1024 * 1024];
        loop {
            job.check()?;
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            size += n as u64;
            sha.update(&buf[..n]);
        }
        if size != entry.size || format!("{:x}", sha.finalize()) != entry.sha256 {
            return Err("Workspace edited; collection stopped".into());
        }
        job.check()?;
        if unsafe { SetFileInformationByHandle(f.as_raw_handle(), 4, &1, 4) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (entry, job);
        Err("Safe workspace collection requires Windows".into())
    }
}

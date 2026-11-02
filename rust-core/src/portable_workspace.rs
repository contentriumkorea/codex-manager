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
// Each stage records its directory identities before copying and a durable receipt
// for every completed file. Recovery never infers ownership from a partial name.
fn copy_payload(source: &Path, stage: &Path, tree: &Tree, job: &Job) -> Result<(), String> {
    let destination = stage.join("data");
    let receipts = stage.join("completed");
    fs::create_dir(&destination).map_err(|e| e.to_string())?;
    fs::create_dir(&receipts).map_err(|e| e.to_string())?;
    let mut directories = serde_json::Map::new();
    for d in &tree.directories {
        let p = destination.join(d);
        ensure_plain(&p)?;
        fs::create_dir_all(&p).map_err(|e| e.to_string())?;
        directories.insert(d.clone(), json!(crate::identity(&p)?));
    }
    write_new(&stage.join("owner.json"), &serde_json::to_vec(&json!({
        "schema":1,"stage":crate::identity(stage)?,"data":crate::identity(&destination)?,
        "completed":crate::identity(&receipts)?,"directories":directories,"tree":tree_value(tree)
    })).map_err(|e|e.to_string())?)?;
    let total = crate::tree_size(tree)?;
    let mut done = 0;
    job.phase("Connected workspace copy", total);
    for e in &tree.files {
        let path = destination.join(&e.path);
        let identity = crate::transfer::copy_file_owned(
            &source.join(&e.path),
            &path,
            e,
            job,
            &mut done,
            total,
        )?;
        let receipt = json!({"path":e.path,"size":e.size,"sha256":e.sha256,"identity":identity});
        if let Err(error) = write_new(
            &receipts.join(format!("{}.json", hash(e.path.as_bytes()))),
            &serde_json::to_vec(&receipt).map_err(|e| e.to_string())?,
        ) {
            // This operation still has a receipt in memory if persisting it failed.
            let mut owned = e.clone();
            owned.identity = Some(identity);
            let cleanup = delete_owned(&path, &owned, job);
            return Err(format!(
                "Workspace receipt write failed: {error}; cleanup: {cleanup:?}"
            ));
        }
    }
    Ok(())
}
fn remove_metadata(path: &Path, job: &Job) -> Result<(), String> {
    ensure_plain(path)?;
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    delete_owned(
        path,
        &Entry {
            path: String::new(),
            size: bytes.len() as u64,
            sha256: hash(&bytes),
            identity: Some(crate::identity(path)?),
        },
        job,
    )
}
fn owned_directory(path: &Path, expected: &Value) -> Result<(), String> {
    ensure_plain(path)?;
    if expected.as_str() != Some(crate::identity(path)?.as_str()) {
        return Err("Partial workspace directory identity changed".into());
    }
    Ok(())
}
fn recover_partial(stage: &Path, job: &Job) -> Result<(), String> {
    ensure_plain(stage)?;
    let owner = read_json(&stage.join("owner.json"))?;
    if owner["schema"] != 1 {
        return Err("Unknown partial ownership receipt".into());
    }
    owned_directory(stage, &owner["stage"])?;
    let data = stage.join("data");
    let completed = stage.join("completed");
    if data.try_exists().map_err(|e| e.to_string())? {
        owned_directory(&data, &owner["data"])?;
    }
    if completed.try_exists().map_err(|e| e.to_string())? {
        owned_directory(&completed, &owner["completed"])?;
    }
    let tree = tree_read(&owner["tree"])?;
    let mut warnings = vec![];
    for entry in &tree.files {
        job.check()?;
        let receipt_path = completed.join(format!("{}.json", hash(entry.path.as_bytes())));
        if !receipt_path.try_exists().map_err(|e| e.to_string())? {
            continue;
        }
        let result = (|| {
            let r = read_json(&receipt_path)?;
            if r["path"] != entry.path || r["size"] != entry.size || r["sha256"] != entry.sha256 {
                return Err("Completed partial receipt changed".into());
            }
            let mut owned = entry.clone();
            owned.identity = Some(string(&r, "identity")?.into());
            let path = data.join(&entry.path);
            if path.try_exists().map_err(|e| e.to_string())? {
                let mut parent = Path::new(&entry.path).parent();
                while let Some(p) = parent {
                    if p.as_os_str().is_empty() {
                        break;
                    }
                    let relative = p.to_string_lossy().replace('\\', "/");
                    owned_directory(&data.join(p), &owner["directories"][&relative])?;
                    parent = p.parent();
                }
                delete_owned(&path, &owned, job)?;
            }
            remove_metadata(&receipt_path, job)
        })();
        if let Err(e) = result {
            warnings.push(format!("{}: {e}", entry.path));
        }
    }
    job.check()?;
    let mut dirs = tree.directories.clone();
    dirs.sort_by_key(|s| std::cmp::Reverse(s.split('/').count()));
    for d in dirs {
        let p = data.join(&d);
        if !p.try_exists().map_err(|e| e.to_string())? {
            continue;
        }
        let result = owned_directory(&p, &owner["directories"][&d])
            .and_then(|_| fs::remove_dir(&p).map_err(|e| e.to_string()));
        if let Err(e) = result {
            warnings.push(format!("{d}: {e}"));
        }
    }
    // Unknown/edited entries keep the stage and its remaining receipts. Empty-directory
    // removal is deliberately nonrecursive and refuses any concurrent extra entry.
    for folder in [&data, &completed] {
        if folder.try_exists().map_err(|e| e.to_string())?
            && fs::read_dir(folder)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            warnings.push("Edited or unknown partial contents retained".into());
        }
    }
    for item in fs::read_dir(stage).map_err(|e| e.to_string())? {
        let name = item.map_err(|e| e.to_string())?.file_name();
        if !["data", "completed", "owner.json", "receipt.json"]
            .iter()
            .any(|n| name == *n)
        {
            warnings.push("Unknown stage entry retained".into());
        }
    }
    if !warnings.is_empty() {
        return Err(warnings.join("; "));
    }
    // A completed immutable receipt can exist after a crash just before publication.
    let receipt = stage.join("receipt.json");
    if receipt.try_exists().map_err(|e| e.to_string())? {
        if read_json(&receipt)?["tree"] != owner["tree"] {
            return Err("Changed final receipt retained".into());
        }
        remove_metadata(&receipt, job)?;
    }
    for (folder, key) in [(&data, "data"), (&completed, "completed")] {
        if folder.try_exists().map_err(|e| e.to_string())? {
            owned_directory(folder, &owner[key])?;
            fs::remove_dir(folder).map_err(|e| e.to_string())?;
        }
    }
    remove_metadata(&stage.join("owner.json"), job)?;
    fs::remove_dir(stage).map_err(|e| e.to_string())
}
fn recover_partials(dir: &Path, job: &Job) -> Result<(), String> {
    let store = dir.join("workspaces");
    if !store.try_exists().map_err(|e| e.to_string())? {
        return Ok(());
    }
    ensure_plain(&store)?;
    for item in fs::read_dir(store).map_err(|e| e.to_string())? {
        job.check()?;
        let path = item.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "partial") {
            // Retained-content warnings are returned by collection after publication.
            let _ = recover_partial(&path, job);
            job.check()?;
        }
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
    recover_partials(dir, job)?;
    let mut external = vec![];
    for r in roots {
        require_directory(r)?;
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
            copy_payload(r, &stage, &tree, job)?;
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
        if p.extension().is_some_and(|e| e == "partial") {
            if let Err(error) = recover_partial(&p, job) {
                warnings.push(format!(
                    "Partial workspace retained: {}: {error}",
                    p.display()
                ));
            }
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
            // Capture-only receipts are retained with the immutable payload so a crash
            // before rename always has recovery evidence; verify them before removal.
            if p.join("owner.json")
                .try_exists()
                .map_err(|e| e.to_string())?
            {
                let owner = read_json(&p.join("owner.json"))?;
                if owner["tree"] != tree_value(&tree) {
                    return Err("Changed owner receipt retained".into());
                }
                let completed = p.join("completed");
                for entry in &tree.files {
                    let receipt = completed.join(format!("{}.json", hash(entry.path.as_bytes())));
                    let r = read_json(&receipt)?;
                    if r["path"] != entry.path
                        || r["sha256"] != entry.sha256
                        || r["size"] != entry.size
                    {
                        return Err("Changed completion receipt retained".into());
                    }
                    remove_metadata(&receipt, job)?;
                }
                fs::remove_dir(&completed).map_err(|e| e.to_string())?;
                remove_metadata(&p.join("owner.json"), job)?;
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
pub(super) fn delete_owned(path: &Path, entry: &Entry, job: &Job) -> Result<(), String> {
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
        if entry
            .identity
            .as_ref()
            .is_some_and(|id| crate::file_identity(&f).as_ref() != Ok(id))
        {
            return Err("Workspace identity changed; collection stopped".into());
        }
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

pub(super) fn require_directory(path: &Path) -> Result<(), String> {
    absolute(path)
        .map_err(|e| format!("Unresolved workspace directory {}: {e}", path.display()))?;
    if !path.is_dir() {
        return Err(format!(
            "Unresolved workspace directory (missing or not a directory): {}",
            path.display()
        ));
    }
    Ok(())
}
pub(super) fn require_mapped_directory(
    anchor: &Path,
    dir: &Path,
    m: &Value,
    path: &str,
) -> Result<(), String> {
    let mut physical = map(anchor, m, path)?;
    for r in external_roots(m)? {
        let payload = dir
            .join("workspaces")
            .join(hash_key(string(r, "treeHash")?)?)
            .join("data");
        if let Ok(mapped) = map_workspace(
            Path::new(path),
            &[(PathBuf::from(string(r, "original")?), payload)],
        ) {
            physical = mapped;
            break;
        }
    }
    require_directory(&physical)
}

#[cfg(test)]
mod partial_tests {
    use super::*;
    fn stage() -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        let source = base.join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/a.txt"), b"owned a").unwrap();
        fs::write(source.join("b.txt"), b"owned b").unwrap();
        let stage = base.join("owned.partial");
        fs::create_dir(&stage).unwrap();
        let tree = scan(&source, &Job::new()).unwrap();
        copy_payload(&source, &stage, &tree, &Job::new()).unwrap();
        stage
    }
    #[test]
    fn partial_recovery_reclaims_complete_receipts_and_respects_cancel() {
        let stage = stage();
        let job = Job::new();
        job.cancel();
        assert!(recover_partial(&stage, &job).is_err());
        assert!(stage.join("data/b.txt").exists());
        recover_partial(&stage, &Job::new()).unwrap();
        assert!(!stage.exists());
    }
    #[test]
    fn partial_unknown_entries_and_corrupt_receipts_survive() {
        let stage = stage();
        fs::write(stage.join("data/user.txt"), b"unknown").unwrap();
        let receipt = stage
            .join("completed")
            .join(format!("{}.json", hash(b"b.txt")));
        fs::write(&receipt, b"corrupt receipt").unwrap();
        assert!(recover_partial(&stage, &Job::new()).is_err());
        assert_eq!(fs::read(stage.join("data/user.txt")).unwrap(), b"unknown");
        assert_eq!(fs::read(stage.join("data/b.txt")).unwrap(), b"owned b");
        assert_eq!(fs::read(receipt).unwrap(), b"corrupt receipt");
        assert!(!stage.join("data/nested/a.txt").exists());
    }
    #[test]
    fn partial_replaced_directory_is_not_treated_as_owned() {
        let stage = stage();
        fs::rename(stage.join("data/nested"), stage.join("moved")).unwrap();
        fs::create_dir(stage.join("data/nested")).unwrap();
        fs::rename(stage.join("moved/a.txt"), stage.join("data/nested/a.txt")).unwrap();
        assert!(recover_partial(&stage, &Job::new()).is_err());
        assert!(stage.join("data/nested/a.txt").exists());
    }
    #[test]
    fn partial_recovery_resumes_after_empty_owned_directories_were_removed() {
        let stage = stage();
        let owner = read_json(&stage.join("owner.json")).unwrap();
        let tree = tree_read(&owner["tree"]).unwrap();
        for entry in &tree.files {
            delete_owned(&stage.join("data").join(&entry.path), entry, &Job::new()).unwrap();
            remove_metadata(
                &stage
                    .join("completed")
                    .join(format!("{}.json", hash(entry.path.as_bytes()))),
                &Job::new(),
            )
            .unwrap();
        }
        fs::remove_dir(stage.join("data/nested")).unwrap();
        fs::remove_dir(stage.join("data")).unwrap();
        fs::remove_dir(stage.join("completed")).unwrap();
        recover_partial(&stage, &Job::new()).unwrap();
        assert!(!stage.exists());
    }
}

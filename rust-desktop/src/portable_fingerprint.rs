use codex_manager_core::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn stamp(path: &Path, hash: &mut Sha256) -> Result<(), String> {
    absolute(path)?;
    ensure_plain(path)?;
    let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    hash.update(
        serde_json::to_vec(&json!([
            path,
            identity(path)?,
            meta.len(),
            meta.modified()
                .map_err(|e| e.to_string())?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos(),
            meta.is_dir()
        ]))
        .map_err(|e| e.to_string())?,
    );
    Ok(())
}
/// Metadata only: this is a scheduling hint, never an integrity receipt.
pub fn fingerprint(
    catalog: &Catalog,
    project: &str,
    anchor: &Path,
    job: &Job,
) -> Result<String, String> {
    let group = catalog
        .projects
        .iter()
        .find(|g| g.project["id"] == project)
        .ok_or("자동 보관 프로젝트가 없습니다.")?;
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(&group.project).map_err(|e| e.to_string())?);
    let all = all_threads(catalog);
    let selected = group
        .threads
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let ids = expand_thread_ids(&all, &selected)?;
    for thread in all.iter().filter(|t| {
        t["id"]
            .as_str()
            .is_some_and(|id| ids.iter().any(|s| s == id))
    }) {
        job.check()?;
        hash.update(serde_json::to_vec(thread).map_err(|e| e.to_string())?);
        let source =
            resolve_rollout_path(Path::new(thread["path"].as_str().ok_or("대화 경로 누락")?))?;
        stamp(&source, &mut hash)?;
    }
    for root in roots(&group.project)? {
        absolute(&root)?;
        ensure_plain(&root)?;
        if !root.is_dir() {
            return Err("자동 보관 연결 폴더가 없습니다.".into());
        }
        if clean_path(&root).starts_with(clean_path(anchor)) {
            hash.update(identity(&root)?.as_bytes());
            continue;
        }
        stamp(&root, &mut hash)?;
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            job.check()?;
            let mut entries = fs::read_dir(dir)
                .map_err(|e| e.to_string())?
                .map(|e| e.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            entries.sort();
            for path in entries {
                if path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(".codex-manager"))
                {
                    continue;
                }
                stamp(&path, &mut hash)?;
                if path.is_dir() {
                    stack.push(path);
                }
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_nested_edits_and_source_replacement_change_stamp() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        let anchor = root.join("anchor");
        let external = root.join("outside");
        fs::create_dir_all(&anchor).unwrap();
        fs::create_dir_all(external.join("nested")).unwrap();
        let file = external.join("nested/file");
        fs::write(&file, "a").unwrap();
        let c = Catalog {
            projects: vec![ProjectGroup {
                project: json!({"id":"p","roots":[{"path":anchor},{"path":external}]}),
                threads: vec![],
            }],
            unassigned: vec![],
            unresolved: vec![],
        };
        let a = fingerprint(&c, "p", &anchor, &Job::new()).unwrap();
        assert_eq!(a, fingerprint(&c, "p", &anchor, &Job::new()).unwrap());
        fs::write(file, "changed").unwrap();
        assert_ne!(a, fingerprint(&c, "p", &anchor, &Job::new()).unwrap());
        fs::rename(&anchor, root.join("moved")).unwrap();
        assert!(fingerprint(&c, "p", &anchor, &Job::new()).is_err());
        assert!(!anchor.exists());
    }
}

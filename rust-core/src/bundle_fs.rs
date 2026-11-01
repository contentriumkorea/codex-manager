use crate::{valid_bundle_path, validate_bundle_index, Job};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, Metadata};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

fn linked(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn checked_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if !valid_bundle_path(relative) {
        return Err("invalid relative path".into());
    }
    let mut path = root.to_owned();
    for component in relative.split('/') {
        path.push(component);
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if linked(&meta) {
            return Err("linked path in bundle".into());
        }
    }
    Ok(path)
}

fn hash_file(
    path: &Path,
    expected_size: u64,
    job: &Job,
    done: &mut u64,
    total: u64,
) -> Result<String, String> {
    let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if linked(&before) || !before.is_file() || before.len() != expected_size {
        return Err("bundle file size or type mismatch".into());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut count = 0u64;
    loop {
        job.check()?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count = count.checked_add(n as u64).ok_or("file size overflow")?;
        *done = done.saturating_add(n as u64);
        job.advance((*done).min(total), &path.to_string_lossy(), n as u64);
        hash.update(&buffer[..n]);
    }
    let after = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if count != expected_size
        || linked(&after)
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err("bundle file changed while reading".into());
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn collect_files(root: &Path) -> Result<HashSet<String>, String> {
    let mut files = HashSet::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if linked(&metadata) {
                return Err("linked path in bundle".into());
            }
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
            let text = relative
                .to_str()
                .ok_or("invalid Unicode filename")?
                .replace('\\', "/");
            if !valid_bundle_path(&text) {
                return Err("invalid bundle path".into());
            }
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                if text != "checksums.jsonl" && text != "verification.json" {
                    files.insert(text);
                }
            } else {
                return Err("unsupported bundle entry".into());
            }
        }
    }
    Ok(files)
}

pub fn verify_bundle(root: &Path) -> Result<(), String> {
    load_verified_bundle(root, &Job::new()).map(|_| ())
}
pub struct VerifiedBundle {
    pub manifest: Value,
    pub checksums: Vec<Value>,
}
pub fn load_verified_bundle(root: &Path, job: &Job) -> Result<Value, String> {
    verified_bundle(root, job).map(|v| v.manifest)
}
pub fn verified_bundle(root: &Path, job: &Job) -> Result<VerifiedBundle, String> {
    let root_meta = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if linked(&root_meta) || !root_meta.is_dir() {
        return Err("invalid bundle root".into());
    }
    for parent in root.ancestors().skip(1) {
        if linked(&fs::symlink_metadata(parent).map_err(|e| e.to_string())?) {
            return Err("linked parent of bundle".into());
        }
    }
    let manifest_path = checked_path(root, "manifest.json")?;
    let checksum_path = checked_path(root, "checksums.jsonl")?;
    let manifest_bytes = fs::read(&manifest_path).map_err(|e| e.to_string())?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
    let mut checksums = Vec::new();
    for line in BufReader::new(File::open(checksum_path).map_err(|e| e.to_string())?).lines() {
        checksums.push(
            serde_json::from_str::<Value>(&line.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        );
    }
    validate_bundle_index(&manifest, &checksums)?;
    let manifest_hash = format!("{:x}", Sha256::digest(&manifest_bytes));
    if checksums
        .iter()
        .find(|c| c["path"] == "manifest.json")
        .and_then(|c| c["sha256"].as_str())
        != Some(&manifest_hash)
    {
        return Err("manifest changed before verification".into());
    }
    let total = checksums.iter().try_fold(0u64, |sum, c| {
        sum.checked_add(c["size"].as_u64().ok_or("invalid size")?)
            .ok_or("size overflow")
    })?;
    job.phase("백업 무결성 검증", total);
    let mut done = 0;
    let mut expected = HashSet::new();
    for entry in &checksums {
        let relative = entry["path"].as_str().ok_or("missing path")?;
        let path = checked_path(root, relative)?;
        let size = entry["size"].as_u64().ok_or("missing size")?;
        let hash = entry["sha256"].as_str().ok_or("missing hash")?;
        if hash_file(&path, size, job, &mut done, total)? != hash.to_ascii_lowercase() {
            return Err("bundle hash mismatch".into());
        }
        expected.insert(relative.to_owned());
    }
    if collect_files(root)? != expected {
        return Err("bundle file list differs".into());
    }
    job.check()?;
    Ok(VerifiedBundle {
        manifest,
        checksums,
    })
}

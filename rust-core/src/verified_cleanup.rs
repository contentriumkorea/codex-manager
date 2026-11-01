use crate::{absolute, digest, ensure_plain, identity, paths_overlap, scan_tree, Entry, Job, Tree};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};
fn matches(actual: &Tree, expected: &Tree, identities: bool) -> bool {
    let mut a = actual.directories.clone();
    let mut b = expected.directories.clone();
    a.sort();
    b.sort();
    a == b
        && actual.files.len() == expected.files.len()
        && expected.files.iter().all(|e| {
            actual.files.iter().any(|v| {
                v.path == e.path
                    && v.size == e.size
                    && v.sha256 == e.sha256
                    && (!identities || e.identity.is_none() || e.identity == v.identity)
            })
        })
}
fn delete_file(source: &Path, destination: &Path, entry: &Entry, job: &Job) -> Result<(), String> {
    ensure_plain(source)?;
    ensure_plain(destination)?;
    if digest(destination, job)? != entry.sha256 {
        return Err("대상 파일이 변경되어 원본 정리를 중단했습니다.".into());
    }
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
        // Acquire DELETE access without marking for deletion. Only this exact
        // handle is marked after content and identity validation have succeeded.
        let mut file = fs::OpenOptions::new()
            .read(true)
            .access_mode(0x80010000)
            .share_mode(1)
            .open(source)
            .map_err(|e| e.to_string())?;
        if entry
            .identity
            .as_ref()
            .is_some_and(|id| identity(source).as_ref() != Ok(id))
        {
            return Err("원본 파일이 교체되어 정리를 중단했습니다.".into());
        }
        let mut hash = Sha256::new();
        let mut size = 0;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            job.check()?;
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            size += n as u64;
            hash.update(&buffer[..n]);
        }
        if size != entry.size || format!("{:x}", hash.finalize()) != entry.sha256 {
            return Err("원본이 변경되어 정리를 중단했습니다.".into());
        }
        job.check()?;
        let delete = 1i32;
        if unsafe { SetFileInformationByHandle(file.as_raw_handle(), 4, &delete, 4) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        drop(file);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("검증 후 원본 정리는 Windows에서 지원합니다.".into())
    }
}
pub fn remove_verified_tree(
    source: &Path,
    destination: &Path,
    tree: &Tree,
    job: &Job,
) -> Result<(), String> {
    absolute(source)?;
    absolute(destination)?;
    if paths_overlap(source, destination) {
        return Err("원본과 대상이 겹칩니다.".into());
    }
    if !matches(&scan_tree(source, job)?, tree, true)
        || !matches(&scan_tree(destination, job)?, tree, false)
    {
        return Err("검증 후 폴더가 변경되어 원본을 정리하지 않습니다.".into());
    }
    job.phase_items("검증된 원본 파일 정리", tree.files.len() as u64);
    for (i, entry) in tree.files.iter().enumerate() {
        job.check()?;
        delete_file(
            &source.join(&entry.path),
            &destination.join(&entry.path),
            entry,
            job,
        )?;
        job.advance((i + 1) as u64, &entry.path, 0);
    }
    let mut dirs = tree.directories.clone();
    dirs.sort_by_key(|p| std::cmp::Reverse(p.split('/').count()));
    for dir in dirs {
        let path = source.join(dir);
        ensure_plain(&path)?;
        fs::remove_dir(path)
            .map_err(|e| format!("원본에 새 항목이 있거나 폴더를 정리할 수 없습니다: {e}"))?;
    }
    fs::remove_dir(source).map_err(|e| format!("원본 폴더 정리 확인 필요: {e}"))
}

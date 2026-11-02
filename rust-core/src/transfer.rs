use crate::{absolute, ensure_plain, valid_bundle_path, Job};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Entry {
    pub identity: Option<String>,
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug)]
pub struct Tree {
    pub files: Vec<Entry>,
    pub directories: Vec<String>,
}
pub(crate) fn read_locked(path: &Path) -> Result<File, String> {
    ensure_plain(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    options
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}
pub fn digest(path: &Path, job: &Job) -> Result<String, String> {
    let mut file = read_locked(path)?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        job.check()?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let after = fs::metadata(path).map_err(|e| e.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("읽는 중 원본 파일이 변경됐습니다.".into());
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn scan_tree(root: &Path, job: &Job) -> Result<Tree, String> {
    absolute(root)?;
    if !root.is_dir() {
        return Err(format!("폴더를 찾을 수 없습니다: {}", root.display()));
    }
    job.phase("파일 목록·해시 확인", 0);
    let mut tree = Tree {
        files: Vec::new(),
        directories: Vec::new(),
    };
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        job.check()?;
        ensure_plain(&dir)?;
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            job.check()?;
            let path = entry.map_err(|e| e.to_string())?.path();
            ensure_plain(&path)?;
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("파일 이름을 읽을 수 없습니다.")?
                .replace('\\', "/");
            if !valid_bundle_path(&relative) {
                return Err(format!("지원하지 않는 파일 이름: {relative}"));
            }
            let m = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            job.advance(tree.files.len() as u64, &relative, 0);
            if m.is_dir() {
                tree.directories.push(relative);
                stack.push(path);
            } else if m.is_file() {
                tree.files.push(Entry {
                    identity: Some(crate::identity(&path)?),
                    path: relative,
                    size: m.len(),
                    sha256: digest(&path, job)?,
                });
            } else {
                return Err("일반 파일이 아닌 항목이 있습니다.".into());
            }
        }
    }
    tree.files.sort_by(|a, b| a.path.cmp(&b.path));
    tree.directories.sort();
    Ok(tree)
}
pub fn tree_size(tree: &Tree) -> Result<u64, String> {
    tree.files.iter().try_fold(0u64, |s, e| {
        s.checked_add(e.size)
            .ok_or_else(|| "파일 크기 합계 초과".into())
    })
}
pub fn copy_file(
    source: &Path,
    destination: &Path,
    entry: &Entry,
    job: &Job,
    done: &mut u64,
    total: u64,
) -> Result<(), String> {
    copy_file_owned(source, destination, entry, job, done, total).map(|_| ())
}
pub(crate) fn copy_file_owned(
    source: &Path,
    destination: &Path,
    entry: &Entry,
    job: &Job,
    done: &mut u64,
    total: u64,
) -> Result<String, String> {
    job.check()?;
    ensure_plain(source)?;
    ensure_plain(destination)?;
    let mut src = read_locked(source)?;
    if entry
        .identity
        .as_ref()
        .is_some_and(|id| crate::identity(source).as_ref() != Ok(id))
    {
        return Err("원본 파일이 다른 항목으로 교체됐습니다.".into());
    }
    let before = src.metadata().map_err(|e| e.to_string())?;
    if before.len() != entry.size {
        return Err("원본 크기가 변경됐습니다.".into());
    }
    if let Some(parent) = destination.parent() {
        ensure_plain(parent)?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        ensure_plain(parent)?;
    }
    if destination.exists() {
        return Err("대상 파일이 이미 있습니다.".into());
    }
    let temporary =
        destination.with_file_name(format!(".manager-{}.partial", uuid::Uuid::new_v4()));
    let mut partial = PartialFile {
        path: temporary.clone(),
        file: None,
    };
    let dst = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    partial.file = Some(dst);
    let dst = partial.file.as_mut().unwrap();
    let mut buffer = vec![0; 1024 * 1024];
    let mut hash = Sha256::new();
    let mut count = 0u64;
    loop {
        job.check()?;
        let n = src.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        dst.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        hash.update(&buffer[..n]);
        count += n as u64;
        *done += n as u64;
        job.advance((*done).min(total), &entry.path, n as u64);
    }
    dst.sync_all().map_err(|e| e.to_string())?;
    partial.file = None;
    let after = src.metadata().map_err(|e| e.to_string())?;
    if count != entry.size
        || before.modified().ok() != after.modified().ok()
        || format!("{:x}", hash.finalize()) != entry.sha256
    {
        return Err("복사 중 원본이 변경됐습니다. 원본과 복사본을 보존했습니다.".into());
    }
    let published_identity = crate::identity(&temporary)?;
    if digest(&temporary, job)? != entry.sha256 {
        return Err("복사본 검증에 실패했습니다.".into());
    }
    rename_new(&temporary, destination)?;
    if crate::identity(destination)? != published_identity {
        return Err("복사본이 소유권 기록 전에 교체됐습니다.".into());
    }
    Ok(published_identity)
}
pub(crate) struct PartialFile {
    path: PathBuf,
    file: Option<File>,
}
impl PartialFile {
    pub(crate) fn create(path: &Path) -> Result<Self, String> {
        ensure_plain(path)?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            path: path.to_owned(),
            file: Some(file),
        })
    }
    pub(crate) fn writer(&mut self) -> &mut File {
        self.file.as_mut().unwrap()
    }
    pub(crate) fn publish(mut self, target: &Path) -> Result<(), String> {
        self.writer().sync_all().map_err(|e| e.to_string())?;
        self.file = None;
        rename_new(&self.path, target)
    }
}
impl Drop for PartialFile {
    fn drop(&mut self) {
        self.file = None;
        let _ = fs::remove_file(&self.path);
    }
}
pub fn rename_new(source: &Path, target: &Path) -> Result<(), String> {
    ensure_plain(source)?;
    ensure_plain(target)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileW(existing: *const u16, new: *const u16) -> i32;
        }
        let source = source.canonicalize().map_err(|e| e.to_string())?;
        let target = target
            .parent()
            .ok_or("대상 상위 폴더 누락")?
            .canonicalize()
            .map_err(|e| e.to_string())?
            .join(target.file_name().ok_or("대상 이름 누락")?);
        let a = source
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let b = target
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // MoveFileW fails if the destination exists; never replace a concurrent file.
        if unsafe { MoveFileW(a.as_ptr(), b.as_ptr()) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(source, target).map_err(|e| e.to_string())?;
        fs::remove_file(source).map_err(|e| e.to_string())
    }
}
pub fn resume_copy_tree(
    source: &Path,
    destination: &Path,
    tree: &Tree,
    job: &Job,
) -> Result<(), String> {
    if !destination.exists() {
        return copy_tree(source, destination, tree, job);
    }
    absolute(source)?;
    absolute(destination)?;
    if crate::paths_overlap(source, destination) {
        return Err("원본과 대상 폴더가 겹칩니다.".into());
    }
    let current = scan_tree(destination, job)?;
    for entry in &current.files {
        let expected = tree
            .files
            .iter()
            .find(|e| e.path == entry.path)
            .ok_or("대상에 작업 외 파일이 추가됐습니다.")?;
        if entry.sha256 != expected.sha256 || entry.size != expected.size {
            return Err("대상 파일이 이후 변경됐습니다. 덮어쓰지 않습니다.".into());
        }
    }
    for dir in &current.directories {
        if !tree.directories.contains(dir) {
            return Err("대상에 작업 외 폴더가 추가됐습니다.".into());
        }
    }
    for dir in &tree.directories {
        if !valid_bundle_path(dir) {
            return Err("잘못된 상대 경로".into());
        }
        let path = destination.join(dir);
        ensure_plain(&path)?;
        fs::create_dir_all(path).map_err(|e| e.to_string())?;
    }
    let total = tree_size(tree)?;
    let mut done = 0;
    job.phase("미완료 파일 복사·검증", total);
    for entry in &tree.files {
        if current.files.iter().any(|e| e.path == entry.path) {
            done += entry.size;
            job.advance(done, &entry.path, 0);
            continue;
        }
        copy_file(
            &source.join(&entry.path),
            &destination.join(&entry.path),
            entry,
            job,
            &mut done,
            total,
        )?;
    }
    Ok(())
}
pub fn copy_tree(source: &Path, destination: &Path, tree: &Tree, job: &Job) -> Result<(), String> {
    copy_tree_observed(source, destination, tree, job, &|_, _, _, _| Ok(()))
}
pub(crate) type CopyObserver<'a> =
    dyn Fn(&Path, Option<&Path>, &str, Option<&str>) -> Result<(), String> + 'a;
pub(crate) fn copy_tree_observed(
    source: &Path,
    destination: &Path,
    tree: &Tree,
    job: &Job,
    copied: &CopyObserver<'_>,
) -> Result<(), String> {
    let src = absolute(source)?;
    let dst = absolute(destination)?;
    let key = |p: &Path| {
        p.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    };
    let a = key(&src);
    let b = key(&dst);
    if a == b || b.starts_with(&(a.clone() + "/")) || a.starts_with(&(b.clone() + "/")) {
        return Err("원본과 대상 폴더가 겹칩니다.".into());
    }
    if dst.exists() {
        return Err("대상 폴더가 이미 있습니다. 새 폴더를 선택하세요.".into());
    }
    job.check()?;
    if let Some(parent) = dst.parent() {
        ensure_plain(parent)?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir(&dst).map_err(|e| e.to_string())?;
    copied(&dst, None, &crate::identity(&dst)?, None)?;
    let mut directories = tree.directories.clone();
    directories.sort_by_key(|p| p.split('/').count());
    for dir in &directories {
        job.check()?;
        if !valid_bundle_path(dir) {
            return Err("잘못된 상대 경로".into());
        }
        let path = dst.join(dir);
        ensure_plain(&path)?;
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        copied(&path, None, &crate::identity(&path)?, None)?;
    }
    let total = tree_size(tree)?;
    job.phase("파일 복사·검증", total);
    let mut done = 0;
    for entry in &tree.files {
        if !valid_bundle_path(&entry.path) {
            return Err("잘못된 상대 경로".into());
        }
        let owned_identity = copy_file_owned(
            &src.join(&entry.path),
            &dst.join(&entry.path),
            entry,
            job,
            &mut done,
            total,
        )?;
        copied(
            &dst.join(&entry.path),
            Some(&src.join(&entry.path)),
            &owned_identity,
            Some(&entry.sha256),
        )?;
    }
    job.phase("원본 변경 확인", total);
    let current = scan_tree(&src, job)?;
    if current.directories != tree.directories
        || current.files.len() != tree.files.len()
        || current
            .files
            .iter()
            .zip(&tree.files)
            .any(|(a, b)| a.path != b.path || a.size != b.size || a.sha256 != b.sha256)
    {
        return Err("작업 중 원본 폴더가 변경됐습니다. 원본을 유지했습니다.".into());
    }
    Ok(())
}
pub fn write_json_new(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    ensure_plain(path)?;
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut f, value).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}
pub fn safe_destination(parent: &Path, name: &str) -> Result<PathBuf, String> {
    absolute(parent)?;
    if !valid_bundle_path(name) || name.contains('/') {
        return Err("폴더 이름을 확인하세요.".into());
    }
    Ok(parent.join(name))
}

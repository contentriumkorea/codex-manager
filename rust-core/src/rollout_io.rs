//! Validated streaming rollout reads. Raw file proofs never mean decoded-message proofs.
use crate::{digest, ensure_plain, file_identity, identity, Job};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// Resolve only Codex's exact `.jsonl` -> `.jsonl.zst` sibling convention.
/// Existing plain files take precedence; permission errors and links never fall back.
pub fn resolve_rollout_path(path: &Path) -> Result<PathBuf, String> {
    crate::absolute(path)?;
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => return Ok(path.to_owned()),
        Ok(_) => return Err("Rollout is not a regular file".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    if path.extension().is_some_and(|v| v == "jsonl") {
        let candidate = path.with_extension("jsonl.zst");
        ensure_plain(&candidate)?;
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|e| format!("{}: {e}", candidate.display()))?;
        if metadata.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Rollout is missing or not a regular file: {}",
        path.display()
    ))
}

pub struct RolloutRecord {
    /// Original decoded bytes, including the line ending, for verbatim historical preservation.
    pub line: String,
    pub value: Value,
}
/// Consume through `next_record() == Ok(None)` before treating a rollout as verified.
/// A partial consumer has not validated the trailing JSON/frame or final source proof.
pub struct RolloutReader {
    live: bool,
    requested: PathBuf,
    path: PathBuf,
    raw_sha256: String,
    raw_identity: String,
    raw_size: u64,
    modified: Option<std::time::SystemTime>,
    guard: File,
    reader: Box<dyn BufRead>,
    job: Job,
    expected_id: Option<String>,
    count: usize,
    complete: bool,
    failure: Option<String>,
}
impl RolloutReader {
    pub fn open(path: &Path, expected_id: Option<&str>, job: &Job) -> Result<Self, String> {
        Self::open_mode(path, expected_id, job, false)
    }
    /// Read while Codex holds a writable handle; a changing source fails at EOF.
    /// This mode does not relax locks in any raw-copy or mutation operation.
    pub fn open_live(path: &Path, expected_id: Option<&str>, job: &Job) -> Result<Self, String> {
        Self::open_mode(path, expected_id, job, true)
    }
    fn open_mode(
        path: &Path,
        expected_id: Option<&str>,
        job: &Job,
        live: bool,
    ) -> Result<Self, String> {
        job.check()?;
        let resolved = resolve_rollout_path(path)?;
        let mut file = open_raw(&resolved, live)?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        let raw_identity = file_identity(&file)?;
        if identity(&resolved)? != raw_identity {
            return Err("Rollout replaced while opening".into());
        }
        let raw_sha256 = raw_digest(&resolved, job, live)?;
        let mut magic = [0u8; 4];
        let n = file.read(&mut magic).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let compressed = resolved.extension().is_some_and(|v| v == "zst")
            || (n == 4 && magic == [0x28, 0xb5, 0x2f, 0xfd]);
        let guard = file.try_clone().map_err(|e| e.to_string())?;
        let decoded: Box<dyn Read> = if compressed {
            // Default decoder traverses all concatenated frames and rejects an incomplete EOF.
            Box::new(zstd::stream::read::Decoder::new(file).map_err(|e| e.to_string())?)
        } else {
            Box::new(file)
        };
        let reader: Box<dyn BufRead> = Box::new(BufReader::new(CancellableRead {
            inner: decoded,
            job: job.clone(),
        }));
        let result = Self {
            live,
            requested: path.to_owned(),
            path: resolved,
            raw_sha256,
            raw_identity,
            raw_size: metadata.len(),
            modified: metadata.modified().ok(),
            guard,
            reader,
            job: job.clone(),
            expected_id: expected_id.map(str::to_owned),
            count: 0,
            complete: false,
            failure: None,
        };
        result.check_source()?;
        Ok(result)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn raw_sha256(&self) -> &str {
        &self.raw_sha256
    }
    pub fn raw_identity(&self) -> &str {
        &self.raw_identity
    }
    pub fn raw_size(&self) -> u64 {
        self.raw_size
    }
    fn check_source(&self) -> Result<(), String> {
        self.job.check()?;
        ensure_plain(&self.path)?;
        let m = self.guard.metadata().map_err(|e| e.to_string())?;
        if resolve_rollout_path(&self.requested)? != self.path
            || identity(&self.path)? != self.raw_identity
            || m.len() != self.raw_size
            || m.modified().ok() != self.modified
        {
            return Err("Rollout changed or was replaced during read".into());
        }
        Ok(())
    }
    pub fn next_record(&mut self) -> Result<Option<RolloutRecord>, String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let result = self.read_record();
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }
    fn read_record(&mut self) -> Result<Option<RolloutRecord>, String> {
        self.job.check()?;
        if self.complete {
            return Ok(None);
        }
        let mut line = String::new();
        if self
            .reader
            .read_line(&mut line)
            .map_err(|e| format!("Invalid rollout stream: {e}"))?
            == 0
        {
            if self.count == 0 {
                return Err("Empty rollout".into());
            }
            self.check_source()?;
            if raw_digest(&self.path, &self.job, self.live)? != self.raw_sha256 {
                return Err("Rollout bytes changed during read".into());
            }
            self.check_source()?;
            self.complete = true;
            return Ok(None);
        }
        let value: Value = serde_json::from_str(&line).map_err(|e| {
            format!(
                "{}: invalid rollout record {}: {e}",
                self.path.display(),
                self.count + 1
            )
        })?;
        if self.count == 0
            && (value["type"] != "session_meta"
                || value["payload"]["id"].as_str().is_none_or(str::is_empty)
                || self
                    .expected_id
                    .as_ref()
                    .is_some_and(|id| value["payload"]["id"] != *id))
        {
            return Err("Rollout session ID or first record is invalid".into());
        }
        self.job.check()?;
        self.count += 1;
        Ok(Some(RolloutRecord { line, value }))
    }
}

struct CancellableRead {
    inner: Box<dyn Read>,
    job: Job,
}
impl Read for CancellableRead {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.job.check().map_err(std::io::Error::other)?;
        self.inner.read(buffer)
    }
}

fn open_raw(path: &Path, live: bool) -> Result<File, String> {
    if !live {
        return crate::transfer::read_locked(path);
    }
    ensure_plain(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(7);
    }
    options.open(path).map_err(|e| e.to_string())
}
fn raw_digest(path: &Path, job: &Job, live: bool) -> Result<String, String> {
    if !live {
        return digest(path, job);
    }
    let mut file = open_raw(path, true)?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    let id = file_identity(&file)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        job.check()?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    if identity(path)? != id
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err("Rollout changed while hashing".into());
    }
    Ok(format!("{:x}", hash.finalize()))
}
/// Proof of every decoded byte (including runtime metadata), not merely messages.
pub fn decoded_rollout_digest(path: &Path, job: &Job) -> Result<String, String> {
    let mut reader = RolloutReader::open(path, None, job)?;
    let mut hash = Sha256::new();
    while let Some(record) = reader.next_record()? {
        hash.update(record.line.as_bytes());
    }
    Ok(format!("{:x}", hash.finalize()))
}

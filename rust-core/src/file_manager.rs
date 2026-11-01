use crate::{
    absolute, copy_file, copy_tree, digest, ensure_plain, paths_overlap, rename_new, roots,
    scan_tree, valid_bundle_path, Adapter, Entry, Job,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
pub const FILE_RECOVERY: &str = ".codex-manager-file-recovery";
pub fn signature(path: &Path, job: &Job) -> Result<Value, String> {
    absolute(path)?;
    if path.is_file() {
        return Ok(json!({"":digest(path,job)?}));
    }
    let tree = scan_tree(path, job)?;
    let mut map = serde_json::Map::new();
    map.insert("".into(), Value::Null);
    for dir in tree.directories {
        map.insert(dir, Value::Null);
    }
    for file in tree.files {
        map.insert(file.path, json!(file.sha256));
    }
    Ok(Value::Object(map))
}
pub fn identity(path: &Path) -> Result<String, String> {
    ensure_plain(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(7)
            .custom_flags(0x02000000)
            .open(path)
            .map_err(|e| e.to_string())?;
        file_identity(&file)
    }
    #[cfg(not(windows))]
    {
        file_identity(&fs::File::open(path).map_err(|e| e.to_string())?)
    }
}
pub fn file_identity(file: &fs::File) -> Result<String, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut u32) -> i32;
        }
        let mut info = [0u32; 13];
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(format!("{}:{}:{}", info[7], info[11], info[12]))
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        Ok(format!("{}:{}", meta.dev(), meta.ino()))
    }
}
fn within(path: &Path, root: &Path) -> bool {
    let path = crate::clean_path(path)
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    let root = crate::clean_path(root)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase();
    path == root || path.starts_with(&(root + "/"))
}
pub fn list_files(directory: &Path, allowed: &[PathBuf]) -> Result<Vec<Value>, String> {
    absolute(directory)?;
    if !allowed.iter().any(|root| within(directory, root)) {
        return Err("프로젝트 폴더 안에서 탐색하세요.".into());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == FILE_RECOVERY {
            continue;
        }
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        let linked = ensure_plain(&path).is_err();
        files.push(json!({"id":path,"path":path,"name":name,"directory":meta.is_dir(),"linked":linked,"size":if meta.is_file(){Some(meta.len())}else{None},"updatedAt":meta.modified().ok().and_then(|t|t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d|d.as_secs())}));
    }
    Ok(files)
}
fn copy_item(source: &Path, target: &Path, stamp: &Value, job: &Job) -> Result<(), String> {
    if stamp[""].is_null() {
        copy_tree(source, target, &scan_tree(source, job)?, job)?;
    } else {
        let entry = Entry {
            identity: Some(identity(source)?),
            path: source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            size: fs::metadata(source).map_err(|e| e.to_string())?.len(),
            sha256: stamp[""].as_str().ok_or("파일 해시 누락")?.into(),
        };
        job.phase("파일 복사·검증", entry.size);
        copy_file(source, target, &entry, job, &mut 0, entry.size)?;
    }
    if signature(source, job)? != *stamp || signature(target, job)? != *stamp {
        return Err("복사 중 파일이 변경됐습니다. 원본은 유지됩니다.".into());
    }
    Ok(())
}
fn unused(path: PathBuf, reserved: &mut HashSet<String>) -> PathBuf {
    let mut result = path.clone();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    let mut n = 1;
    while result.exists() || reserved.contains(&result.to_string_lossy().to_lowercase()) {
        result = path.with_file_name(format!("{stem} - 복사본 ({n}){ext}"));
        n += 1;
    }
    reserved.insert(result.to_string_lossy().to_lowercase());
    result
}
impl Adapter {
    pub fn mkdir(&self, parent: &Path, name: &str, job: &Job) -> Result<String, String> {
        job.check()?;
        self.ensure_mutation_lock()?;
        absolute(parent)?;
        if !parent.is_dir()
            || !valid_bundle_path(name)
            || name.contains('/')
            || name.contains(FILE_RECOVERY)
        {
            return Err("폴더 위치와 이름을 확인하세요.".into());
        }
        let catalog = self.snapshot()?;
        let allowed = catalog
            .projects
            .iter()
            .map(|g| roots(&g.project))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let root = allowed
            .iter()
            .filter(|root| within(parent, root))
            .max_by_key(|root| root.components().count())
            .ok_or("프로젝트 폴더 안에 만드세요.")?;
        if self
            .protected_storage()
            .iter()
            .any(|p| paths_overlap(parent, p))
            || parent.components().any(|c| c.as_os_str() == FILE_RECOVERY)
        {
            return Err("보호된 폴더에는 만들 수 없습니다.".into());
        }
        let target = parent.join(name);
        ensure_plain(&target)?;
        if target.exists() {
            return Err("같은 이름의 항목이 있습니다.".into());
        }
        let recovery = root
            .join(FILE_RECOVERY)
            .join(uuid::Uuid::new_v4().to_string());
        let stage = recovery.join("mkdir-stage");
        let plan = json!({"source":target,"target":target,"stage":stage,"recovery":recovery,"mode":"copy","sourceId":"","signature":{"":null},"hold":null});
        let op=self.begin(json!({"kind":"rust-files","action":"mkdir","plans":[plan],"resources":[format!("file:{}",target.display())]}))?;
        let result = (|| {
            job.check()?;
            ensure_plain(&recovery)?;
            fs::create_dir_all(&recovery).map_err(|e| e.to_string())?;
            fs::create_dir(&stage).map_err(|e| e.to_string())?;
            self.journal.record(
                &op,
                "publish-intent",
                json!({"index":0,"identity":identity(&stage)?}),
            )?;
            rename_new(&stage, &target)?;
            Ok(())
        })();
        self.finish(&op, result)
    }
    pub fn edit_files(
        &self,
        kind: &str,
        sources: &[PathBuf],
        destination: &Path,
        name: Option<&str>,
        job: &Job,
    ) -> Result<String, String> {
        job.check()?;
        self.ensure_mutation_lock()?;
        if !["copy", "move", "rename", "delete"].contains(&kind)
            || sources.is_empty()
            || (kind == "rename" && sources.len() != 1)
        {
            return Err("파일과 작업을 다시 선택하세요.".into());
        }
        absolute(destination)?;
        let catalog = self.snapshot()?;
        let allowed = catalog
            .projects
            .iter()
            .map(|g| roots(&g.project))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let protected = crate::protected_workspaces(&catalog, job)?;
        if kind != "copy" && sources.iter().any(|source| source.is_dir()) {
            self.require_closed()?;
        }
        let mut distinct = HashSet::new();
        for source in sources {
            absolute(source)?;
            if !distinct.insert(source.to_string_lossy().to_lowercase()) {
                return Err("중복 파일을 선택했습니다.".into());
            }
            if source.components().any(|c| c.as_os_str() == FILE_RECOVERY)
                || self
                    .protected_storage()
                    .iter()
                    .any(|p| paths_overlap(source, p))
                || !allowed.iter().any(|r| within(source, r))
            {
                return Err("프로젝트 파일만 관리할 수 있습니다.".into());
            }
            if kind != "copy" && protected.iter().any(|p| within(p, source)) {
                return Err("프로젝트나 대화가 사용하는 폴더는 프로젝트 관리에서 옮기세요.".into());
            }
            if sources
                .iter()
                .any(|other| source != other && within(source, other))
            {
                return Err("상위 폴더와 하위 항목을 동시에 선택할 수 없습니다.".into());
            }
        }
        if ["copy", "move"].contains(&kind)
            && (!destination.is_dir()
                || self
                    .protected_storage()
                    .iter()
                    .any(|p| paths_overlap(destination, p))
                || !allowed.iter().any(|r| within(destination, r))
                || destination
                    .components()
                    .any(|c| c.as_os_str() == FILE_RECOVERY)
                || sources.iter().any(|p| within(destination, p)))
        {
            return Err("프로젝트 안의 다른 대상 폴더를 선택하세요.".into());
        }
        if kind == "rename"
            && name.is_none_or(|name| {
                !valid_bundle_path(name) || name.contains('/') || name.contains(FILE_RECOVERY)
            })
        {
            return Err("사용할 수 없는 파일 이름입니다.".into());
        }
        let key = uuid::Uuid::new_v4().to_string();
        let mut plans = Vec::new();
        let mut reserved = HashSet::new();
        for (index, source) in sources.iter().enumerate() {
            job.check()?;
            let root = allowed
                .iter()
                .filter(|root| within(source, root))
                .max_by_key(|root| root.components().count())
                .ok_or("원본 프로젝트 루트 누락")?;
            let recovery = root.join(FILE_RECOVERY).join(&key);
            let target = match kind {
                "delete" => recovery.join(format!(
                    "{index}-{}",
                    source.file_name().unwrap_or_default().to_string_lossy()
                )),
                "rename" => source.with_file_name(name.unwrap()),
                _ => {
                    if source.parent() == Some(destination) && kind == "move" {
                        continue;
                    }
                    unused(
                        destination.join(source.file_name().ok_or("파일 이름 오류")?),
                        &mut reserved,
                    )
                }
            };
            if target == *source {
                continue;
            }
            absolute(&target)?;
            if target.exists() {
                return Err("대상 항목이 이미 있습니다.".into());
            }
            let source_id = identity(source)?;
            let mode = if kind == "copy" {
                "copy"
            } else if kind == "move"
                && source_id.split(':').next() != identity(destination)?.split(':').next()
            {
                "cross_move"
            } else {
                "rename"
            };
            let target_root = allowed
                .iter()
                .filter(|root| within(&target, root))
                .max_by_key(|root| root.components().count())
                .unwrap_or(root);
            plans.push(json!({"source":source,"sourceId":source_id,"target":target,"mode":mode,"signature":signature(source,job)?,"stage":target_root.join(FILE_RECOVERY).join(&key).join(format!("{index}-stage")),"hold":if mode=="cross_move"{Some(recovery.join(format!("{index}-hold")))}else{None},"recovery":target_root.join(FILE_RECOVERY).join(&key)}));
        }
        if plans.is_empty() {
            return Err("변경할 항목이 없습니다.".into());
        }
        let op=self.begin(json!({"kind":"rust-files","action":kind,"plans":plans,"resources":sources.iter().map(|p|format!("file:{}",p.display())).collect::<Vec<_>>()}))?;
        let result = (|| {
            for (index, plan) in plans.iter().enumerate() {
                job.check()?;
                let source = Path::new(plan["source"].as_str().unwrap());
                let target = Path::new(plan["target"].as_str().unwrap());
                let stage = Path::new(plan["stage"].as_str().unwrap());
                if identity(source)? != plan["sourceId"].as_str().unwrap()
                    || signature(source, job)? != plan["signature"]
                {
                    return Err("원본이 변경됐습니다.".into());
                }
                ensure_plain(target)?;
                fs::create_dir_all(target.parent().ok_or("대상 폴더 누락")?)
                    .map_err(|e| e.to_string())?;
                if plan["mode"] == "rename" {
                    self.journal.record(
                        &op,
                        "publish-intent",
                        json!({"index":index,"identity":identity(source)?}),
                    )?;
                    job.phase("파일 경로 변경", plans.len() as u64);
                    rename_new(source, target)?;
                } else {
                    fs::create_dir_all(stage.parent().ok_or("임시 폴더 누락")?)
                        .map_err(|e| e.to_string())?;
                    copy_item(source, stage, &plan["signature"], job)?;
                    self.journal.record(
                        &op,
                        "publish-intent",
                        json!({"index":index,"identity":identity(stage)?}),
                    )?;
                    rename_new(stage, target)?;
                    if let Some(hold) = plan["hold"].as_str() {
                        if signature(source, job)? != plan["signature"]
                            || identity(source)? != plan["sourceId"].as_str().unwrap()
                        {
                            return Err("이동 전에 원본이 변경됐습니다.".into());
                        }
                        let hold = Path::new(hold);
                        ensure_plain(hold)?;
                        fs::create_dir_all(hold.parent().unwrap()).map_err(|e| e.to_string())?;
                        rename_new(source, hold)?;
                    }
                }
                self.journal
                    .record(&op, "editing", json!({"index":index}))?;
            }
            Ok(())
        })();
        self.finish(&op, result)
    }
    pub(crate) fn undo_files(&self, id: &str, payload: &Value, job: &Job) -> Result<(), String> {
        job.check()?;
        let protected = crate::protected_workspaces(&self.snapshot()?, job)?;
        let plans = payload["plans"].as_array().ok_or("파일 복구 정보 누락")?;
        let events = self.journal.events(id)?;
        let intents: HashMap<_, _> = events
            .iter()
            .filter(|e| e["phase"] == "publish-intent")
            .filter_map(|e| {
                Some((
                    e["payload"]["index"].as_u64()?,
                    e["payload"]["identity"].as_str()?.to_owned(),
                ))
            })
            .collect();
        let mut owned = Vec::new();
        for (index, plan) in plans.iter().enumerate() {
            let source = Path::new(plan["source"].as_str().ok_or("원본 경로 누락")?);
            let target = Path::new(plan["target"].as_str().ok_or("대상 경로 누락")?);
            absolute(source)?;
            absolute(target)?;
            let stage = Path::new(plan["stage"].as_str().ok_or("임시 경로 누락")?);
            absolute(stage)?;
            let mut target_owned = target.exists()
                && intents
                    .get(&(index as u64))
                    .is_some_and(|expected| identity(target).is_ok_and(|id| id == *expected));
            if (plan["mode"] == "rename" && source.exists())
                || (plan["mode"] != "rename" && stage.exists())
            {
                target_owned = false;
            }
            if target_owned && signature(target, job)? != plan["signature"] {
                return Err("대상이 이후 변경됐습니다. 덮어쓰지 않습니다.".into());
            }
            if target_owned && target.is_dir() {
                self.require_closed()?;
                if protected.iter().any(|p| within(p, target)) {
                    return Err(
                        "다른 프로젝트나 대화가 사용하는 폴더여서 되돌릴 수 없습니다.".into(),
                    );
                }
            }
            if plan["mode"] == "rename" {
                if source.exists() {
                    if identity(source)? != plan["sourceId"].as_str().ok_or("원본 ID 누락")?
                        || signature(source, job)? != plan["signature"]
                    {
                        return Err("원래 위치에 새 항목이 있습니다.".into());
                    }
                } else if !target_owned {
                    return Err("복구할 원본을 확인할 수 없습니다.".into());
                }
            }
            if let Some(hold) = plan["hold"].as_str() {
                let hold = Path::new(hold);
                absolute(hold)?;
                let location = if hold.exists() {
                    if source.exists() {
                        return Err("원래 위치에 새 항목이 있습니다.".into());
                    }
                    hold
                } else {
                    source
                };
                if !location.exists()
                    || identity(location)? != plan["sourceId"].as_str().unwrap_or("")
                    || signature(location, job)? != plan["signature"]
                {
                    return Err("복구 보관소가 변경됐습니다.".into());
                }
            }
            owned.push(target_owned);
        }
        self.mark_undo_mutation()?;
        for (index, plan) in plans.iter().enumerate().rev() {
            job.check()?;
            let source = Path::new(plan["source"].as_str().unwrap());
            let target = Path::new(plan["target"].as_str().unwrap());
            if plan["mode"] == "rename" {
                if owned[index] {
                    fs::create_dir_all(source.parent().unwrap()).map_err(|e| e.to_string())?;
                    rename_new(target, source)?;
                }
            } else {
                if let Some(hold) = plan["hold"].as_str() {
                    let hold = Path::new(hold);
                    if hold.exists() {
                        fs::create_dir_all(source.parent().unwrap()).map_err(|e| e.to_string())?;
                        rename_new(hold, source)?;
                    }
                }
                if owned[index] {
                    let kept = Path::new(plan["recovery"].as_str().ok_or("보관 위치 누락")?)
                        .join(format!("{index}-undone"));
                    ensure_plain(&kept)?;
                    fs::create_dir_all(kept.parent().unwrap()).map_err(|e| e.to_string())?;
                    rename_new(target, &kept)?;
                }
            }
            self.journal
                .record(id, "restoring-file", json!({"source":source}))?;
        }
        Ok(())
    }
}

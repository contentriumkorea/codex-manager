//! Cross-store operations retain immutable intent in the shared journal. Child
//! journals use the same Adapter (and filesystem lock) for their entire lifetime.
use crate::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
fn s<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v[k].as_str().ok_or_else(|| format!("작업 정보 누락: {k}"))
}
fn arr<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>, String> {
    v[k].as_array()
        .ok_or_else(|| format!("작업 목록 누락: {k}"))
}
fn ids(v: &Value, k: &str) -> Result<Vec<String>, String> {
    arr(v, k)?
        .iter()
        .map(|x| x.as_str().map(str::to_owned).ok_or("선택 값 오류".into()))
        .collect()
}
fn key(p: &Path) -> String {
    // Canonicalize the existing ancestor too, so short-name aliases and casing
    // cannot disguise overlap when the final destination does not exist yet.
    let canonical = p
        .ancestors()
        .find_map(|a| {
            a.canonicalize()
                .ok()
                .and_then(|a_real| p.strip_prefix(a).ok().map(|tail| a_real.join(tail)))
        })
        .unwrap_or_else(|| p.to_owned());
    clean_path(&canonical)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}
fn overlap(a: &Path, b: &Path) -> bool {
    let a = key(a);
    let b = key(b);
    a == b || a.starts_with(&(b.clone() + "/")) || b.starts_with(&(a + "/"))
}
fn within(p: &Path, r: &Path) -> bool {
    let p = key(p);
    let r = key(r);
    p == r || p.starts_with(&(r + "/"))
}
fn revision(v: &Value) -> Value {
    json!(format!("{:x}", Sha256::digest(v.to_string().as_bytes())))
}
fn stores(source: &Adapter, target: &Adapter) -> Result<(), String> {
    for a in [&source.store.home, &source.store.sqlite_home] {
        absolute(a)?;
        for b in [&target.store.home, &target.store.sqlite_home] {
            absolute(b)?;
            if overlap(a, b) || (a.exists() && b.exists() && identity(a)? == identity(b)?) {
                return Err("같거나 겹치는 Codex 저장소 간에는 전송할 수 없습니다.".into());
            }
        }
    }
    if key(&source.journal.path) != key(&target.journal.path) {
        return Err("두 저장소는 같은 Manager 작업 내역을 사용해야 합니다.".into());
    }
    Ok(())
}
fn protected(source: &Adapter, target: &Adapter, p: &Path) -> Result<(), String> {
    absolute(p)?;
    if p.components()
        .any(|c| c.as_os_str() == FILE_RECOVERY || c.as_os_str() == ".codex-manager-transfer")
        || source
            .protected_storage()
            .iter()
            .chain(target.protected_storage().iter())
            .any(|r| overlap(p, r))
    {
        return Err("Codex·Manager 보호 경로는 전송할 수 없습니다.".into());
    }
    Ok(())
}
fn safe_name(name: &str) -> String {
    let name = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect::<String>();
    let name = name.trim_matches(['.', ' ']);
    if name.is_empty() {
        "project".into()
    } else {
        name.chars().take(100).collect()
    }
}
fn identities(path: &Path, job: &Job) -> Result<Value, String> {
    let mut result = json!({"":identity(path)?});
    if path.is_dir() {
        for entry in scan_tree(path, job)?.files {
            result[&entry.path] = json!(entry.identity);
        }
    }
    Ok(result)
}
fn thread_stamp(t: &Value, job: &Job) -> Result<Value, String> {
    let path = Path::new(s(t, "path")?);
    Ok(
        json!({"thread":t,"identity":identity(path)?,"sha256":digest(path,job)?,"messages":crate::workflow::messages_digest(path,job)?,"runtime":runtime_roots_with_job(path,job)?}),
    )
}
fn dependencies(t: &Value, paths: &[PathBuf], job: &Job) -> Result<(), String> {
    let map = paths
        .iter()
        .map(|p| (p.clone(), p.clone()))
        .collect::<Vec<_>>();
    map_workspace(Path::new(s(t, "cwd")?), &map)?;
    let path = Path::new(s(t, "path")?);
    for p in runtime_roots_with_job(path, job)? {
        map_workspace(&p, &map)?;
    }
    if t["attachmentCount"].as_u64().unwrap_or(0) > 0 || !t["attachedThreadParentId"].is_null() {
        return Err("외부 첨부 의존성이 있는 대화입니다. 백업에서 의존성을 확인하세요.".into());
    }
    for line in BufReader::new(fs::File::open(path).map_err(|e| e.to_string())?).lines() {
        job.check()?;
        let v: Value =
            serde_json::from_str(&line.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if v["type"] == "response_item" {
            for part in v["payload"]["content"].as_array().into_iter().flatten() {
                if let Some(p) = part["path"].as_str().or(part["image_path"].as_str()) {
                    if Path::new(p).is_absolute() {
                        map_workspace(Path::new(p), &map).map_err(|_| {
                            "외부 첨부 파일이 연결돼 있습니다. 의존성을 먼저 확인하세요.".to_owned()
                        })?;
                    }
                }
            }
        }
    }
    Ok(())
}
/// Read-only preview. The source revision covers file bytes and rollout identity
/// as well as catalog metadata; a catalog-only revision cannot protect file drags.
pub fn cross_store_plan(
    source: &Adapter,
    target: &Adapter,
    request: &Value,
    job: &Job,
) -> Result<Value, String> {
    job.check()?;
    stores(source, target)?;
    let kind = s(request, "kind")?;
    let selected = ids(request, "ids")?;
    if selected.is_empty() || selected.iter().collect::<HashSet<_>>().len() != selected.len() {
        return Err("중복 없이 전송할 항목을 선택하세요.".into());
    }
    let destination = absolute(Path::new(s(request, "destination")?))?;
    protected(source, target, &destination)?;
    let a = source.snapshot()?;
    let b = target.snapshot()?;
    let target_project = request["targetProject"]
        .as_str()
        .filter(|id| !id.is_empty());
    if let Some(id) = target_project {
        if !b.projects.iter().any(|g| g.project["id"] == id) {
            return Err("대상 프로젝트가 바뀌었습니다.".into());
        }
    }
    let all = all_threads(&a);
    // Separate homes may still share actual workspace paths. Every target
    // project, cwd and secondary runtime root remains an external reference.
    let target_workspaces = protected_workspaces(&b, job)?;
    let mut groups = Vec::<Value>::new();
    let mut selected_threads = Vec::new();
    let source_roots = a
        .projects
        .iter()
        .map(|g| roots(&g.project))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let mut mappings = Vec::new();
    let mut warnings = Vec::new();
    if kind == "files" {
        let allowed = b
            .projects
            .iter()
            .filter(|g| target_project.is_none_or(|id| g.project["id"] == id))
            .map(|g| roots(&g.project))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if !destination.is_dir() || !allowed.iter().any(|r| within(&destination, r)) {
            return Err("대상 저장소 프로젝트 안의 폴더를 선택하세요.".into());
        }
        let paths = selected
            .iter()
            .map(|p| absolute(Path::new(p)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut runtime = protected_workspaces(&a, job)?;
        runtime.extend(target_workspaces.iter().cloned());
        for (i, path) in paths.iter().enumerate() {
            protected(source, target, path)?;
            if !source_roots.iter().any(|r| within(path, r))
                || paths
                    .iter()
                    .enumerate()
                    .any(|(j, p)| i != j && overlap(path, p))
            {
                return Err("프로젝트 파일만 선택하고 상위·하위 중복 선택을 해제하세요.".into());
            }
            if request["copy"] != true && runtime.iter().any(|p| within(p, path)) {
                return Err("대화가 사용하는 작업 폴더는 프로젝트·대화 전송으로 옮기세요.".into());
            }
            mappings.push(json!({"source":path,"destination":destination.join(path.file_name().ok_or("파일 이름 누락")?),"retainSource":request["copy"]==true,"identity":identity(path)?,"signature":signature(path,job)?,"identities":identities(path,job)?}));
        }
    } else {
        let initial = match kind {
            "projects" => {
                for id in &selected {
                    if !a.projects.iter().any(|g| g.project["id"] == *id) {
                        return Err("선택한 프로젝트가 바뀌었습니다.".into());
                    }
                }
                a.projects
                    .iter()
                    .filter(|g| selected.iter().any(|id| g.project["id"] == *id))
                    .flat_map(|g| g.threads.iter())
                    .map(|t| s(t, "id").map(str::to_owned))
                    .collect::<Result<Vec<_>, _>>()?
            }
            "threads" => selected.clone(),
            _ => return Err("지원하지 않는 전송 종류입니다.".into()),
        };
        selected_threads = expand_thread_ids(&all, &initial)?;
        let existing = all_threads(&b);
        if selected_threads
            .iter()
            .any(|id| existing.iter().any(|t| t["id"] == *id))
        {
            return Err("대상 저장소에 같은 ID의 대화가 있습니다. 복사·이동을 중단합니다.".into());
        }
        for g in &a.projects {
            let threads = g
                .threads
                .iter()
                .filter(|t| selected_threads.iter().any(|id| t["id"] == *id))
                .cloned()
                .collect::<Vec<_>>();
            if !threads.is_empty()
                || (kind == "projects" && selected.iter().any(|id| g.project["id"] == *id))
            {
                groups.push(json!({"project":g.project,"threads":threads}));
            }
        }
        let extra = all
            .iter()
            .filter(|t| {
                selected_threads.iter().any(|id| t["id"] == *id)
                    && !groups.iter().any(|g| {
                        g["threads"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|x| x["id"] == t["id"])
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        for t in extra {
            let mut paths = runtime_roots_with_job(Path::new(s(&t, "path")?), job)?;
            paths.push(PathBuf::from(s(&t, "cwd")?));
            paths.sort();
            paths.dedup();
            let keep = paths
                .iter()
                .filter(|p| !paths.iter().any(|other| *p != other && within(p, other)))
                .cloned()
                .collect::<Vec<_>>();
            groups.push(json!({"project":{"id":format!("unassigned-{}",s(&t,"id")?),"name":t["name"].as_str().unwrap_or("대화"),"roots":keep.iter().map(|p|json!({"path":p})).collect::<Vec<_>>()},"threads":[t],"synthetic":true}));
        }
        for (gi, g) in groups.iter().enumerate() {
            let paths = roots(&g["project"])?;
            for t in arr(g, "threads")? {
                dependencies(t, &paths, job)?;
            }
            for (ri, path) in paths.iter().enumerate() {
                protected(source, target, path)?;
                if !path.is_dir()
                    || path.join(".git").is_file()
                    || paths
                        .iter()
                        .enumerate()
                        .any(|(j, p)| j != ri && overlap(path, p))
                {
                    return Err("연결 작업트리·겹친 폴더는 의존성을 먼저 확인하세요.".into());
                }
                let other_project = a.projects.iter().any(|other| {
                    other.project["id"] != g["project"]["id"]
                        && roots(&other.project).is_ok_and(|rs| rs.iter().any(|r| overlap(path, r)))
                });
                let remaining = all
                    .iter()
                    .filter(|t| !selected_threads.iter().any(|id| t["id"] == *id))
                    .any(|t| {
                        Path::new(t["cwd"].as_str().unwrap_or("")).is_absolute()
                            && overlap(path, Path::new(t["cwd"].as_str().unwrap_or("")))
                    });
                let mut retain = request["copy"] == true
                    || kind == "threads"
                    || other_project
                    || remaining
                    || target_workspaces.iter().any(|p| overlap(path, p));
                for t in all
                    .iter()
                    .filter(|t| !selected_threads.iter().any(|id| t["id"] == *id))
                {
                    if let Some(p) = t["path"].as_str() {
                        if runtime_roots_with_job(Path::new(p), job)?
                            .iter()
                            .any(|r| overlap(path, r))
                        {
                            retain = true;
                        }
                    }
                }
                if retain && request["copy"] != true {
                    warnings.push(json!(
                        "다른 프로젝트·대화가 사용하는 원본 폴더는 유지합니다."
                    ));
                }
                let target = destination
                    .join(format!(
                        "{}-{}",
                        gi + 1,
                        safe_name(g["project"]["name"].as_str().unwrap_or("project"))
                    ))
                    .join(format!(
                        "{}-{}",
                        ri + 1,
                        safe_name(&path.file_name().unwrap_or_default().to_string_lossy())
                    ));
                mappings.push(json!({"group":gi,"source":path,"destination":target,"retainSource":retain,"identity":identity(path)?,"signature":signature(path,job)?,"identities":identities(path,job)?}));
            }
        }
    }
    for (i, m) in mappings.iter().enumerate() {
        let p = Path::new(s(m, "destination")?);
        protected(source, target, p)?;
        if p.exists()
            || mappings
                .iter()
                .any(|r| overlap(p, Path::new(r["source"].as_str().unwrap())))
            || mappings
                .iter()
                .take(i)
                .any(|r| overlap(p, Path::new(r["destination"].as_str().unwrap())))
        {
            return Err(
                "대상 파일이 이미 있거나 원본·대상 경로가 겹칩니다. 빈 위치를 선택하세요.".into(),
            );
        }
    }
    if source_roots.iter().any(|r| within(&destination, r)) {
        return Err("원본 프로젝트 안에는 전송 결과를 만들 수 없습니다.".into());
    }
    let stamps = all
        .iter()
        .filter(|t| selected_threads.iter().any(|id| t["id"] == *id))
        .map(|t| thread_stamp(t, job))
        .collect::<Result<Vec<_>, _>>()?;
    // Stage on the selected volume but outside existing target project roots.
    let target_roots = b
        .projects
        .iter()
        .map(|g| roots(&g.project))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let mut staging_base = destination.clone();
    while let Some(root) = target_roots.iter().find(|r| within(&staging_base, r)) {
        staging_base = root
            .parent()
            .ok_or("대상 볼륨에 독립된 임시 폴더를 만들 수 없습니다.")?
            .to_owned();
    }
    staging_base = staging_base.join(".codex-manager-transfer");
    absolute(&staging_base)?;
    if source
        .protected_storage()
        .iter()
        .chain(target.protected_storage().iter())
        .any(|p| overlap(&staging_base, p))
        || source_roots.iter().any(|p| overlap(&staging_base, p))
    {
        return Err("대상 볼륨에 보호 경로와 겹치지 않는 임시 위치가 필요합니다.".into());
    }
    let a_json = catalog_json(a);
    let b_json = catalog_json(b);
    let source_revision =
        revision(&json!({"catalog":a_json["revision"],"roots":mappings,"threads":stamps}));
    Ok(
        json!({"request":request,"sourceHome":source.store.home,"targetHome":target.store.home,"sourceSqliteHome":source.store.sqlite_home,"targetSqliteHome":target.store.sqlite_home,"projects":groups,"threads":selected_threads,"threadStamps":stamps,"stagingBase":staging_base,"roots":mappings,"warnings":warnings,"sourceRevision":source_revision,"targetRevision":b_json["revision"]}),
    )
}
fn lock_both(source: &mut Adapter, target: &mut Adapter) -> Result<(), String> {
    source.release_server();
    target.release_server();
    if key(&source.store.home) < key(&target.store.home) {
        source.guard()?;
        target.guard()?;
    } else {
        target.guard()?;
        source.guard()?;
    }
    Ok(())
}
pub fn cross_store_transfer(
    source: &mut Adapter,
    target: &mut Adapter,
    request: &Value,
    job: &Job,
) -> Result<Value, String> {
    if let Some(id) = request["resume"].as_str() {
        return cross_store_resume(source, target, id, job);
    }
    stores(source, target)?;
    lock_both(source, target)?;
    let plan = cross_store_plan(source, target, request, job)?;
    if request["expectedRevision"] != plan["sourceRevision"]
        || request["targetRevision"] != plan["targetRevision"]
    {
        return Err("미리보기 이후 원본·대상이 변경됐습니다. 미리보기를 다시 확인하세요.".into());
    }
    for record in source.journal.history()? {
        if !["completed", "cancelled"].contains(&record["state"].as_str().unwrap_or(""))
            && ["home", "sourceHome", "targetHome"]
                .iter()
                .filter_map(|k| record["payload"][*k].as_str())
                .any(|h| {
                    overlap(Path::new(h), &source.store.home)
                        || overlap(Path::new(h), &target.store.home)
                })
        {
            return Err("같은 저장소의 미완료 작업을 먼저 복구하세요.".into());
        }
    }
    let op=source.journal.begin(&json!({"kind":"rust-cross-store","home":source.store.home,"sourceHome":source.store.home,"targetHome":target.store.home,"request":request,"plan":plan,"resources":[format!("home:{}",source.store.home.display()),format!("home:{}",target.store.home.display())],"undoSupported":false}))?;
    cross_store_resume(source, target, &op, job)
}
pub fn cross_store_resume(
    source: &mut Adapter,
    target: &mut Adapter,
    id: &str,
    job: &Job,
) -> Result<Value, String> {
    stores(source, target)?;
    uuid::Uuid::parse_str(id).map_err(|_| "작업 ID 오류")?;
    let central = source.journal.clone();
    let record = central.operation(id)?.ok_or("전송 기록이 없습니다.")?;
    let p = &record["payload"];
    if p["kind"] != "rust-cross-store"
        || p["sourceHome"] != json!(source.store.home)
        || p["targetHome"] != json!(target.store.home)
    {
        return Err("전송 기록의 두 저장소를 연결하세요.".into());
    }
    if central
        .events(id)?
        .iter()
        .any(|e| e["phase"] == "concluding")
    {
        return Err("안전 종료를 시작한 전송입니다. 안전 종료 미리보기를 다시 확인하세요.".into());
    }
    if record["state"] == "cancelled" {
        return Err("안전 종료한 전송은 재개할 수 없습니다. 새 전송을 미리보기하세요.".into());
    }
    if record["state"] == "completed" {
        return Ok(
            json!({"operationId":id,"sourceHome":source.store.home,"targetHome":target.store.home,"completed":true}),
        );
    }
    if p["plan"]["sourceSqliteHome"] != json!(source.store.sqlite_home)
        || p["plan"]["targetSqliteHome"] != json!(target.store.sqlite_home)
    {
        return Err("전송 이후 데이터베이스 저장 위치가 변경됐습니다.".into());
    }
    for r in arr(&p["plan"], "roots")? {
        protected(source, target, Path::new(s(r, "source")?))?;
        protected(source, target, Path::new(s(r, "destination")?))?;
    }
    absolute(Path::new(s(&p["plan"], "stagingBase")?))?;
    lock_both(source, target)?;
    central.claim(id, &ids(p, "resources")?)?;
    let parent = central.path.parent().ok_or("Manager 경로 누락")?;
    let sj = Journal::open(&parent.join(format!("cross-{id}-source.sqlite")))?;
    let tj = Journal::open(&parent.join(format!("cross-{id}-target.sqlite")))?;
    source.journal = sj;
    target.journal = tj;
    let recovery = Path::new(s(&p["plan"], "stagingBase")?)
        .join(id)
        .join("source-recovery");
    let prior_recovery = source.recovery_override.replace(recovery.clone());
    let result = (|| {
        central.record(id,"child-journals",json!({"source":source.journal.path,"target":target.journal.path,"recoveryRoot":recovery}))?;
        execute(source, target, &central, id, &p["plan"], job)
    })();
    source.recovery_override = prior_recovery;
    source.release_server();
    target.release_server();
    source.journal = central.clone();
    target.journal = central.clone();
    match result {
        Ok(()) => {
            let retained = if recovery.exists() {
                json!({"path":recovery,"bytes":tree_size(&scan_tree(&recovery,job)?)?,"purpose":"source-conversation-recovery"})
            } else {
                Value::Null
            };
            central.record(id, "completed", json!({"retainedRecovery":retained}))?;
            Ok(
                json!({"operationId":id,"sourceHome":source.store.home,"targetHome":target.store.home,"roots":p["plan"]["roots"],"undoSupported":false,"retainedRecovery":retained}),
            )
        }
        Err(e) => {
            central.record(id, "needs_recovery", json!({"error":e}))?;
            Err(format!(
                "{e}\n전송을 작업 내역에서 계속할 수 있습니다. ({id})"
            ))
        }
    }
}
fn conclusion_record(source: &Adapter, target: &Adapter, id: &str) -> Result<Value, String> {
    stores(source, target)?;
    uuid::Uuid::parse_str(id).map_err(|_| "작업 ID 오류")?;
    let record = source
        .journal
        .operation(id)?
        .ok_or("전송 기록이 없습니다.")?;
    let p = &record["payload"];
    if p["kind"] != "rust-cross-store"
        || p["sourceHome"] != json!(source.store.home)
        || p["targetHome"] != json!(target.store.home)
        || p["plan"]["sourceSqliteHome"] != json!(source.store.sqlite_home)
        || p["plan"]["targetSqliteHome"] != json!(target.store.sqlite_home)
    {
        return Err("전송 기록의 두 저장소와 데이터베이스 위치를 연결하세요.".into());
    }
    Ok(record)
}
fn child_journals(central: &Journal, id: &str) -> Result<[Journal; 2], String> {
    let parent = central.path.parent().ok_or("Manager 경로 누락")?;
    Ok([
        Journal {
            path: parent.join(format!("cross-{id}-source.sqlite")),
        },
        Journal {
            path: parent.join(format!("cross-{id}-target.sqlite")),
        },
    ])
}
fn retained_path(path: &Path, job: &Job) -> Value {
    // Unknown/replaced/reparse artifacts must still be reportable and retained.
    match fs::symlink_metadata(path) {
        Ok(_) => {
            json!({"path":path,"exists":true,"identity":identity(path).ok(),"signature":signature(path,job).ok()})
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"path":path,"exists":false}),
        Err(e) => json!({"path":path,"exists":true,"inspectionError":e.to_string()}),
    }
}
/// Read-only: no locks, journal creation, SDK mutation or restoration.
pub fn cross_store_conclude_plan(
    source: &Adapter,
    target: &Adapter,
    id: &str,
    job: &Job,
) -> Result<Value, String> {
    let record = conclusion_record(source, target, id)?;
    let plan = &record["payload"]["plan"];
    let central = &source.journal;
    let source_snapshot = source.snapshot()?;
    let target_snapshot = target.snapshot()?;
    let references = protected_workspaces(&source_snapshot, job).and_then(|mut paths| {
        paths.extend(protected_workspaces(&target_snapshot, job)?);
        Ok(paths)
    });
    let mut retained = Vec::new();
    for (i, r) in arr(plan, "roots")?.iter().enumerate() {
        job.check()?;
        let src = Path::new(s(r, "source")?);
        let dst = Path::new(s(r, "destination")?);
        let hold = hold_path(src, id, i)?;
        let original = retained_path(src, job);
        let held = retained_path(&hold, job);
        let restore = original["exists"] == false
            && held["exists"] == true
            && references
                .as_ref()
                .is_ok_and(|paths| !paths.iter().any(|p| overlap(p, &hold)))
            && event(central, id, "hold-intent", i)?.is_some()
            && held["identity"] == r["identity"]
            && held["signature"] == r["signature"]
            && identities(&hold, job).ok().as_ref() == Some(&r["identities"]);
        retained.push(json!({"source":original,"target":retained_path(dst,job),"hold":held,"action":if restore{"restore-complete-hold"}else{"retain-current"},"sourceUnavailable":original["exists"]==false && !restore}));
    }
    let staging = Path::new(s(plan, "stagingBase")?).join(id);
    let children = child_journals(central, id)?
        .iter()
        .map(|j| {
            Ok(json!({"path":j.path,"operations":if j.path.exists(){j.history()?}else{vec![]}}))
        })
        .collect::<Result<Vec<Value>, String>>()?;
    let source_threads = all_threads(&source_snapshot);
    let target_threads = all_threads(&target_snapshot);
    let conversations=ids(plan,"threads")?.iter().map(|tid| {
        let describe=|threads:&[Value]| threads.iter().find(|t|t["id"]==*tid).map(|t|json!({"thread":t,"rollout":t["path"].as_str().map(|p|retained_path(Path::new(p),job))}));
        let source=describe(&source_threads);
        let target=describe(&target_threads);
        json!({"id":tid,"sourcePresent":source.is_some(),"targetPresent":target.is_some(),"source":source,"target":target})
    }).collect::<Vec<_>>();
    job.check()?;
    let mut result = json!({"operationId":id,"state":record["state"],"sourceHome":source.store.home,"targetHome":target.store.home,"roots":retained,"conversations":conversations,"retainedStaging":retained_path(&staging,job),"recoveryPath":staging.join("source-recovery"),"childJournals":children,"eventRevision":revision(&json!(central.events(id)?)),"warning":"안전 종료는 전송 완료나 원상복구가 아닙니다. 이미 처리된 대화는 되돌리지 않습니다. 남은 파일과 변경·미확인 항목, 복구 사본을 보존하고 두 저장소의 작업 잠금을 해제합니다."});
    result["revision"] = revision(&result);
    Ok(result)
}
/// Stop further transfer/deletion, reconcile actual child outcomes and retain all
/// evidence. Only a complete, identity-checked hold may return to an absent path.
pub fn cross_store_conclude(
    source: &mut Adapter,
    target: &mut Adapter,
    id: &str,
    expected: &Value,
    job: &Job,
) -> Result<Value, String> {
    let record = conclusion_record(source, target, id)?;
    if record["state"] == "cancelled" {
        return source
            .journal
            .events(id)?
            .into_iter()
            .rev()
            .find(|e| e["phase"] == "cancelled")
            .map(|e| e["payload"].clone())
            .ok_or("종료 기록 누락".into());
    }
    if record["state"] == "completed" {
        return Err("이미 완료된 전송입니다.".into());
    }
    lock_both(source, target)?;
    let central = source.journal.clone();
    central.claim(id, &ids(&record["payload"], "resources")?)?;
    let preview = cross_store_conclude_plan(source, target, id, job)?;
    if preview["revision"] != *expected {
        return Err("안전 종료 미리보기 이후 상태가 변경됐습니다. 다시 확인하세요.".into());
    }
    central.record(id, "concluding", json!({"preview":preview}))?;
    // A restart can retry this action after any step; resumed transfer is forbidden
    // once safe conclusion began (its cleanup assumptions are no longer valid).
    for (side, (adapter, journal)) in [&mut *source, &mut *target]
        .into_iter()
        .zip(child_journals(&central, id)?)
        .enumerate()
    {
        if !journal.path.exists() {
            continue;
        }
        adapter.journal = journal.clone();
        let recovery = Path::new(s(&record["payload"]["plan"], "stagingBase")?)
            .join(id)
            .join("conclusion-recovery")
            .join(if side == 0 { "source" } else { "target" });
        let prior_recovery = adapter.recovery_override.replace(recovery);
        let result = (|| {
            for r in journal.history()? {
                if ["completed", "cancelled"].contains(&r["state"].as_str().unwrap_or("")) {
                    continue;
                }
                let child = s(&r, "id")?;
                if r["payload"]["kind"] == "rust-delete" {
                    adapter.finalize_delete(child, job)?;
                } else {
                    // Read actual native state, including an RPC committed before
                    // its outcome was recorded. Sync only what exists now; do not
                    // replay imports, legacy creation, reassignment or deletions.
                    let store = resolve_store(&adapter.store.home)?;
                    let native = if let Some(db) = store.database {
                        read_database_catalog(&db)?
                    } else {
                        adapter.snapshot()?
                    };
                    let child_events = journal.events(child)?;
                    let mut intended = child_events
                        .iter()
                        .filter_map(|e| e["payload"]["threadId"].as_str().map(str::to_owned))
                        .collect::<HashSet<_>>();
                    if r["payload"]["kind"] == "rust-legacy" {
                        intended.extend(
                            r["payload"]["threads"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|t| t["id"].as_str().map(str::to_owned)),
                        );
                    }
                    let assignments = all_threads(&native)
                        .iter()
                        .filter(|t| t["id"].as_str().is_some_and(|id| intended.contains(id)))
                        .filter(|t| {
                            r["payload"]["kind"] != "rust-legacy" || t["projectId"].is_string()
                        })
                        .filter_map(|t| {
                            Some((
                                t["id"].as_str()?.to_owned(),
                                t["projectId"].as_str().map(str::to_owned),
                            ))
                        })
                        .collect::<HashMap<_, _>>();
                    let mut project_ids = child_events
                        .iter()
                        .filter_map(|e| e["payload"]["projectId"].as_str().map(str::to_owned))
                        .collect::<HashSet<_>>();
                    project_ids.extend(assignments.values().flatten().cloned());
                    let projects = native
                        .projects
                        .iter()
                        .filter(|g| {
                            g.project["id"]
                                .as_str()
                                .is_some_and(|id| project_ids.contains(id))
                        })
                        .map(|g| g.project.clone())
                        .collect::<Vec<_>>();
                    adapter.sync(&projects, &assignments, &[], &[])?;
                    journal.record(child,"cancelled",json!({"safeConclusion":id,"retainedCurrent":true,"actualProjects":projects,"actualAssignments":assignments}))?;
                }
            }
            Ok::<_, String>(())
        })();
        adapter.release_server();
        adapter.recovery_override = prior_recovery;
        adapter.journal = central.clone();
        if let Err(e) = result {
            central.record(id, "conclusion-needs-recovery", json!({"error":e}))?;
            return Err(e);
        }
    }
    for (i, r) in arr(&record["payload"]["plan"], "roots")?.iter().enumerate() {
        if preview["roots"][i]["action"] != "restore-complete-hold" {
            continue;
        }
        let src = Path::new(s(r, "source")?);
        let hold = hold_path(src, id, i)?;
        protected(source, target, src)?;
        if !src.exists()
            && identity(&hold).ok().as_ref()
                == r["identity"].as_str().map(|v| v.to_owned()).as_ref()
            && signature(&hold, job).ok().as_ref() == Some(&r["signature"])
            && identities(&hold, job).ok().as_ref() == Some(&r["identities"])
        {
            rename_new(&hold, src)?;
            central.record(
                id,
                "conclusion-root-restored",
                json!({"index":i,"source":src,"hold":hold}),
            )?;
        }
    }
    let outcome = cross_store_conclude_plan(source, target, id, job)?;
    let result = json!({"safeConclusion":true,"transferCompleted":false,"retained":outcome});
    central.record(id, "cancelled", result.clone())?;
    Ok(result)
}

fn event(central: &Journal, id: &str, phase: &str, index: usize) -> Result<Option<Value>, String> {
    Ok(central
        .events(id)?
        .into_iter()
        .rev()
        .find(|e| e["phase"] == phase && e["payload"]["index"] == index)
        .map(|e| e["payload"].clone()))
}
fn aliases(
    adapter: &mut Adapter,
    selected: &[String],
    job: &Job,
) -> Result<HashMap<String, String>, String> {
    adapter.resume_legacy(job)?;
    adapter.materialize_legacy(selected, job)?;
    let mut out = HashMap::new();
    for r in adapter.journal.history()? {
        if r["payload"]["kind"] == "rust-legacy" && r["state"] == "completed" {
            for e in adapter.journal.events(s(&r, "id")?)? {
                if let Some(pid) = e["payload"]["projectId"].as_str() {
                    out.insert(s(&r["payload"]["project"], "id")?.into(), pid.into());
                }
            }
        }
    }
    Ok(out)
}
fn check_source(
    source: &Adapter,
    plan: &Value,
    alias: &HashMap<String, String>,
    job: &Job,
) -> Result<(), String> {
    let snapshot = source.snapshot()?;
    let current = all_threads(&snapshot);
    let history = source.journal.history()?;
    for g in arr(plan, "projects")? {
        if g["synthetic"] == true {
            continue;
        }
        let before = &g["project"];
        let old = s(before, "id")?;
        let pid = alias.get(old).map(String::as_str).unwrap_or(old);
        if let Some(actual) = snapshot.projects.iter().find(|g| g.project["id"] == pid) {
            if actual.project["name"] != before["name"] || roots(&actual.project)? != roots(before)?
            {
                return Err("원본 프로젝트의 이름·폴더가 이후 변경됐습니다.".into());
            }
        } else if !history.iter().any(|r| {
            r["payload"]["kind"] == "rust-delete"
                && r["payload"]["projects"]
                    .as_array()
                    .is_some_and(|ps| ps.iter().any(|p| p["id"] == pid))
        }) {
            return Err("원본 프로젝트가 다른 곳에서 제거됐습니다.".into());
        }
    }
    for stamp in arr(plan, "threadStamps")? {
        let before = &stamp["thread"];
        let tid = s(before, "id")?;
        if let Some(t) = current.iter().find(|t| t["id"] == tid) {
            let path = Path::new(s(t, "path")?);
            let project = before["projectId"]
                .as_str()
                .and_then(|p| alias.get(p))
                .map(|p| json!(p))
                .unwrap_or_else(|| before["projectId"].clone());
            if t["name"] != before["name"]
                || t["projectId"] != project
                || clean_path(Path::new(s(t, "cwd")?)) != clean_path(Path::new(s(before, "cwd")?))
                || identity(path)? != stamp["identity"]
                || digest(path, job)? != stamp["sha256"]
                || crate::workflow::messages_digest(path, job)? != stamp["messages"]
                || json!(runtime_roots_with_job(path, job)?) != stamp["runtime"]
            {
                return Err("원본 대화가 이후 변경됐습니다. 삭제하지 않습니다.".into());
            }
        } else if !history.iter().any(|r| {
            r["payload"]["kind"] == "rust-delete"
                && r["payload"]["threads"]
                    .as_array()
                    .is_some_and(|ts| ts.iter().any(|t| t["id"] == tid))
        }) {
            return Err("원본 대화가 다른 곳에서 삭제됐습니다.".into());
        }
    }
    Ok(())
}
fn check_roots(plan: &Value, id: &str, job: &Job) -> Result<(), String> {
    for (i, r) in arr(plan, "roots")?.iter().enumerate() {
        let source = Path::new(s(r, "source")?);
        let hold = hold_path(source, id, i)?;
        let path = if source.exists() {
            source
        } else if hold.exists() {
            &hold
        } else {
            continue;
        };
        if identity(path)? != r["identity"] {
            return Err("원본 파일이 교체됐습니다.".into());
        }
        // A partially cleaned hold is verified against a subset during cleanup.
        if path == source
            && (signature(path, job)? != r["signature"]
                || identities(path, job)? != r["identities"])
        {
            return Err("원본 파일 내용이 변경됐습니다.".into());
        }
    }
    Ok(())
}
fn hold_path(source: &Path, id: &str, i: usize) -> Result<PathBuf, String> {
    Ok(source
        .parent()
        .ok_or("원본 상위 폴더 누락")?
        .join(format!(".codex-manager-transfer-{id}-{i}-hold")))
}
fn checkpoint(
    source: &Adapter,
    central: &Journal,
    id: &str,
    request: &Value,
    label: &str,
) -> Result<(), String> {
    let phase = format!("isolated-test-{label}");
    if source.isolated_checkpoint(request, label)
        && !central.events(id)?.iter().any(|e| e["phase"] == phase)
    {
        central.record(id, &phase, json!({}))?;
        return Err(format!("Isolated fixture interruption: {label}"));
    }
    Ok(())
}
fn execute(
    source: &mut Adapter,
    target: &mut Adapter,
    central: &Journal,
    id: &str,
    plan: &Value,
    job: &Job,
) -> Result<(), String> {
    let request = &plan["request"];
    job.check()?;
    let selected = ids(request, "ids")?;
    let source_alias = if request["kind"] != "files" && request["copy"] != true {
        aliases(source, &selected, job)?
    } else {
        HashMap::new()
    };
    source.release_server();
    let target_alias = if let Some(t) = request["targetProject"].as_str().filter(|t| !t.is_empty())
    {
        aliases(target, &[t.into()], job)?
    } else {
        HashMap::new()
    };
    target.release_server();
    check_source(source, plan, &source_alias, job)?;
    check_roots(plan, id, job)?;
    central.record(
        id,
        "aliases",
        json!({"source":source_alias,"target":target_alias}),
    )?;
    if request["kind"] == "files" {
        let allowed_source = source
            .snapshot()?
            .projects
            .iter()
            .map(|g| roots(&g.project))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let wanted = request["targetProject"].as_str().filter(|s| !s.is_empty());
        let allowed_target = target
            .snapshot()?
            .projects
            .iter()
            .filter(|g| wanted.is_none_or(|id| g.project["id"] == id))
            .map(|g| roots(&g.project))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        for r in arr(plan, "roots")? {
            if !allowed_source
                .iter()
                .any(|p| within(Path::new(r["source"].as_str().unwrap()), p))
                || !allowed_target
                    .iter()
                    .any(|p| within(Path::new(r["destination"].as_str().unwrap()), p))
            {
                return Err("전송 파일의 프로젝트 소속이 변경됐습니다.".into());
            }
        }
        publish_files(central, id, plan, job)?;
    } else {
        for (i, g) in arr(plan, "projects")?.iter().enumerate() {
            job.check()?;
            let bundle = if let Some(e) = event(central, id, "bundle-ready", i)? {
                PathBuf::from(s(&e, "path")?)
            } else {
                retire_incomplete_bundles(central, id, i, job)?;
                let bundle = Path::new(s(plan, "stagingBase")?)
                    .join(format!("{id}/{i}-{}", uuid::Uuid::new_v4()));
                central.record(id, "bundle-intent", json!({"index":i,"path":bundle}))?;
                let pid = s(&g["project"], "id")?;
                let mut threads = arr(g, "threads")?.clone();
                for t in &mut threads {
                    t["projectId"] = json!(pid);
                }
                let catalog = Catalog {
                    projects: vec![ProjectGroup {
                        project: g["project"].clone(),
                        threads,
                    }],
                    unassigned: vec![],
                    unresolved: vec![],
                };
                let manifest = crate::backup::backup_project_observed(
                    &catalog,
                    pid,
                    &bundle,
                    job,
                    &|path| {
                        central.record(
                            id,
                            "bundle-created",
                            json!({"index":i,"path":path,"identity":identity(path)?}),
                        )?;
                        checkpoint(source, central, id, request, "bundle-created")
                    },
                    &|path, reference, owned_identity, copied_sha256| {
                        if identity(path)? != owned_identity {
                            return Err(
                                "복사본이 소유권 기록 전에 교체됐습니다. 그대로 보관합니다.".into(),
                            );
                        }
                        let file = path.is_file();
                        let cancel_export = file
                            && path.starts_with(bundle.join("files"))
                            && source.isolated_checkpoint(request, "export-file-cancel")
                            && central
                                .events(id)?
                                .iter()
                                .filter(|e| e["phase"] == "bundle-intent")
                                .count()
                                <= request["__testCancelAttempts"].as_u64().unwrap_or(1) as usize;
                        if cancel_export {
                            job.cancel();
                        }

                        // Receipt follows durable publication. A crash in this gap
                        // leaves an unknown artifact: never adopt or delete it.
                        // A handled cancellation after publication must not skip
                        // this receipt. Workspace bytes were just verified by copy;
                        // only small generated control files need a fresh digest.
                        let checksum = if file {
                            Some(if let Some(hash) = copied_sha256 {
                                hash.to_owned()
                            } else {
                                digest(path, &Job::new())?
                            })
                        } else {
                            None
                        };
                        central.record(id, "bundle-owned-entry", json!({"bundle":bundle,"path":path,"identity":owned_identity,"file":file,"sha256":checksum,"reference":reference,"referenceIdentity":reference.map(identity).transpose()?}))?;
                        if cancel_export {
                            job.check()?;
                        }
                        Ok(())
                    },
                )?;
                if !arr(&manifest, "dependencies")?.is_empty() {
                    return Err("외부 의존성이 있는 대화는 전송할 수 없습니다.".into());
                }
                central.record(id, "bundle-ready", json!({"index":i,"path":bundle}))?;
                bundle
            };
            let destinations = arr(plan, "roots")?
                .iter()
                .filter(|r| r["group"] == i)
                .map(|r| s(r, "destination").map(PathBuf::from))
                .collect::<Result<Vec<_>, _>>()?;
            let previous = target.journal.history()?.into_iter().find(|r| {
                r["payload"]["kind"] == "rust-import" && r["payload"]["bundle"] == json!(bundle)
            });
            let target_id = request["targetProject"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| target_alias.get(s).map(String::as_str).unwrap_or(s));
            source.release_server();
            let child = if let Some(r) = &previous {
                if r["state"] == "completed" {
                    s(r, "id")?.to_owned()
                } else {
                    target.import_backup(
                        &bundle,
                        &destinations,
                        target_id,
                        false,
                        Some(s(r, "id")?),
                        job,
                    )?
                }
            } else {
                target.import_backup(&bundle, &destinations, target_id, false, None, job)?
            };
            target.release_server();
            checkpoint(source, central, id, request, "imported")?;
            central.record(
                id,
                "imported",
                json!({"index":i,"childId":child,"bundle":bundle}),
            )?;
        }
        verify_target(target, central, id, plan, job)?;
    }
    check_source(source, plan, &source_alias, job)?;
    check_roots(plan, id, job)?;
    central.record(id, "target-verified", json!({}))?;
    checkpoint(source, central, id, request, "target-verified")?;
    job.check()?;
    if request["copy"] == true {
        return cleanup_bundles(source, central, id, plan, job);
    }
    target.release_server();
    if request["kind"] != "files" {
        // Finalize only deletions from this private child journal, then retry the
        // still-present verified selection. Recovery copies remain in Manager state.
        for r in source.journal.history()? {
            if r["payload"]["kind"] == "rust-delete"
                && !["completed", "cancelled"].contains(&r["state"].as_str().unwrap_or(""))
            {
                source.finalize_delete(s(&r, "id")?, job)?;
            }
        }
        for tid in ids(plan, "threads")? {
            job.check()?;
            verify_target(target, central, id, plan, job)?;
            target.release_server();
            check_source(source, plan, &source_alias, job)?;
            if all_threads(&source.snapshot()?)
                .iter()
                .any(|t| t["id"] == tid)
            {
                source.delete(&[], &[tid], false, job)?;
                source.release_server();
            }
        }
        if request["kind"] == "projects" {
            for old in &selected {
                let pid = source_alias.get(old).unwrap_or(old);
                if let Some(g) = source
                    .snapshot()?
                    .projects
                    .iter()
                    .find(|g| g.project["id"] == *pid)
                {
                    if !g.threads.is_empty() {
                        return Err(
                            "원본 프로젝트에 새 대화가 생겼습니다. 프로젝트를 유지합니다.".into(),
                        );
                    }
                    source.delete(std::slice::from_ref(pid), &[], false, job)?;
                    source.release_server();
                }
            }
        }
    }
    for (i, r) in arr(plan, "roots")?.iter().enumerate() {
        if r["retainSource"] == true {
            continue;
        }
        job.check()?;
        let src = Path::new(s(r, "source")?);
        let dst = Path::new(s(r, "destination")?);
        let hold = hold_path(src, id, i)?;
        if signature(dst, job)? != r["signature"] {
            return Err("대상 파일이 변경됐습니다. 원본 정리를 중단합니다.".into());
        }
        // Re-read BOTH stores immediately before destructive cleanup, also
        // when resuming a hold created before interruption. Imported destination
        // roots do not overlap the original path and are not mistaken for users
        // of the source. Project transfers preserve ancestor references too;
        // file drags retain the existing root/cwd-within-selection policy.
        let mut references = protected_workspaces(&source.snapshot()?, job)?;
        references.extend(protected_workspaces(&target.snapshot()?, job)?);
        let used = |path: &Path| {
            references.iter().any(|p| {
                if request["kind"] == "files" {
                    within(p, path)
                } else {
                    overlap(p, path)
                }
            })
        };
        if used(src) || (hold.exists() && used(&hold)) {
            if !src.exists() && hold.exists() && used(src) {
                // A new reference can arrive after the original was held. Only
                // restore a complete, unchanged owned hold; partial holds stay
                // recoverable without deleting any further bytes.
                if event(central, id, "hold-intent", i)?.is_some()
                    && identity(&hold)? == r["identity"]
                    && signature(&hold, job)? == r["signature"]
                    && identities(&hold, job)? == r["identities"]
                {
                    rename_new(&hold, src)?;
                    central.record(id, "root-restored-for-reference", json!({"index":i}))?;
                }
            }
            return Err("두 저장소의 프로젝트·대화가 원본 폴더를 사용합니다. 원본 정리를 중단하고 파일을 유지합니다.".into());
        }
        if src.exists() {
            if event(central, id, "hold-intent", i)?.is_some() && hold.exists() {
                return Err("원래 위치에 새 파일이 생겼습니다.".into());
            }
            if identity(src)? != r["identity"] || signature(src, job)? != r["signature"] {
                return Err("원본이 변경됐습니다.".into());
            }
            central.record(
                id,
                "hold-intent",
                json!({"index":i,"path":hold,"identity":r["identity"]}),
            )?;
            rename_new(src, &hold)?;
            checkpoint(source, central, id, request, "root-held")?;
        }
        if hold.exists() {
            if event(central, id, "hold-intent", i)?.is_none() || identity(&hold)? != r["identity"]
            {
                return Err("원본 정리 보관소의 소유권을 확인할 수 없습니다.".into());
            }
            cleanup_hold(&hold, dst, &r["signature"], Some(&r["identities"]), job)?;
        } else if event(central, id, "hold-intent", i)?.is_none() {
            return Err("원본이 다른 곳에서 제거됐습니다.".into());
        }
        central.record(id, "root-cleaned", json!({"index":i}))?;
    }
    cleanup_bundles(source, central, id, plan, job)
}
fn verify_target(
    target: &Adapter,
    central: &Journal,
    id: &str,
    plan: &Value,
    job: &Job,
) -> Result<(), String> {
    let snapshot = target.snapshot()?;
    let current = all_threads(&snapshot);
    for (i, g) in arr(plan, "projects")?.iter().enumerate() {
        let e = event(central, id, "imported", i)?.ok_or("가져오기 확인 기록 누락")?;
        let child = s(&e, "childId")?;
        let pid = target
            .journal
            .events(child)?
            .into_iter()
            .rev()
            .find_map(|e| e["payload"]["projectId"].as_str().map(str::to_owned))
            .ok_or("대상 프로젝트 연결 기록 누락")?;
        let mappings = arr(plan, "roots")?
            .iter()
            .filter(|r| r["group"] == i)
            .map(|r| {
                Ok((
                    PathBuf::from(s(r, "source")?),
                    PathBuf::from(s(r, "destination")?),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let project = &snapshot
            .projects
            .iter()
            .find(|g| g.project["id"] == pid)
            .ok_or("대상 프로젝트가 제거됐습니다.")?
            .project;
        let merge = plan["request"]["targetProject"]
            .as_str()
            .is_some_and(|s| !s.is_empty());
        let (expected_name, mut expected_roots) = if merge {
            let first = event(central, id, "imported", 0)?.ok_or("대상 프로젝트 원본 기록 누락")?;
            let record = target
                .journal
                .operation(s(&first, "childId")?)?
                .ok_or("가져오기 기록 누락")?;
            let before = &record["payload"]["targetBefore"];
            (before["name"].clone(), roots(before)?)
        } else {
            (g["project"]["name"].clone(), Vec::new())
        };
        for r in arr(plan, "roots")?
            .iter()
            .filter(|r| merge || r["group"] == i)
        {
            let path = PathBuf::from(s(r, "destination")?);
            if !expected_roots.iter().any(|root| within(&path, root)) {
                expected_roots.push(path);
            }
        }
        let mut expected = expected_roots.iter().map(|p| key(p)).collect::<Vec<_>>();
        expected.sort();
        let mut actual = roots(project)?.iter().map(|p| key(p)).collect::<Vec<_>>();
        actual.sort();
        if project["name"] != expected_name || actual != expected {
            return Err("대상 프로젝트 이름·폴더가 변경됐습니다.".into());
        }
        for t in arr(g, "threads")? {
            let tid = s(t, "id")?;
            let actual = current
                .iter()
                .find(|v| v["id"] == tid)
                .ok_or("가져온 대화가 없습니다.")?;
            let stamp = arr(plan, "threadStamps")?
                .iter()
                .find(|v| v["thread"]["id"] == tid)
                .ok_or("대화 검증 기록 누락")?;
            let path = Path::new(s(actual, "path")?);
            let mut expected_runtime = arr(stamp, "runtime")?
                .iter()
                .map(|v| map_workspace(Path::new(v.as_str().ok_or("작업 경로 오류")?), &mappings))
                .collect::<Result<Vec<_>, _>>()?;
            if expected_runtime.is_empty() {
                expected_runtime.push(map_workspace(Path::new(s(t, "cwd")?), &mappings)?);
            }
            if !target.owns_rollout(child, actual, job)? {
                return Err("대상 대화의 가져오기 소유권을 확인할 수 없습니다.".into());
            }
            if actual["name"] != t["name"].as_str().unwrap_or("제목 없는 대화")
                || actual["projectId"] != pid
                || clean_path(Path::new(s(actual, "cwd")?))
                    != map_workspace(Path::new(s(t, "cwd")?), &mappings)?
                || runtime_roots_with_job(path, job)? != expected_runtime
                || crate::workflow::messages_digest(path, job)? != stamp["messages"]
            {
                return Err("대상 대화의 제목·내용·작업 폴더·소속이 변경됐습니다.".into());
            }
        }
    }
    for r in arr(plan, "roots")? {
        if signature(Path::new(s(r, "destination")?), job)? != r["signature"] {
            return Err("대상 파일 검증이 일치하지 않습니다.".into());
        }
    }
    Ok(())
}
fn publish_files(central: &Journal, id: &str, plan: &Value, job: &Job) -> Result<(), String> {
    for (i, r) in arr(plan, "roots")?.iter().enumerate() {
        job.check()?;
        let src = Path::new(s(r, "source")?);
        let dst = Path::new(s(r, "destination")?);
        let stage = dst
            .parent()
            .ok_or("대상 상위 경로 누락")?
            .join(format!(".codex-manager-transfer-{id}-{i}-stage"));
        if dst.exists() {
            let intent = event(central, id, "file-publish-intent", i)?
                .ok_or("대상에 다른 파일이 있습니다.")?;
            if identity(dst)? != intent["identity"] || signature(dst, job)? != r["signature"] {
                return Err("대상 파일이 변경됐습니다.".into());
            }
            continue;
        }
        if identity(src)? != r["identity"] || signature(src, job)? != r["signature"] {
            return Err("원본 파일이 변경됐습니다.".into());
        }
        central.record(id, "file-stage-intent", json!({"index":i,"path":stage}))?;
        if src.is_dir() {
            resume_copy_tree(src, &stage, &scan_tree(src, job)?, job)?;
        } else if stage.exists() {
            if signature(&stage, job)? != r["signature"] {
                return Err("중단된 임시 파일을 확인하세요.".into());
            }
        } else {
            let entry = Entry {
                path: String::new(),
                identity: Some(identity(src)?),
                size: fs::metadata(src).map_err(|e| e.to_string())?.len(),
                sha256: s(&r["signature"], "")?.into(),
            };
            copy_file(src, &stage, &entry, job, &mut 0, entry.size)?;
        }
        if signature(&stage, job)? != r["signature"] {
            return Err("임시 복사 검증 실패".into());
        }
        central.record(
            id,
            "file-publish-intent",
            json!({"index":i,"identity":identity(&stage)?}),
        )?;
        rename_new(&stage, dst)?;
    }
    Ok(())
}
fn cleanup_hold(
    hold: &Path,
    destination: &Path,
    expected: &Value,
    expected_ids: Option<&Value>,
    job: &Job,
) -> Result<(), String> {
    if hold.is_file() {
        if signature(hold, job)? != *expected {
            return Err("정리할 원본 파일이 변경됐습니다.".into());
        }
        return crate::verified_cleanup::delete_verified_file(
            hold,
            destination,
            &Entry {
                path: String::new(),
                identity: Some(identity(hold)?),
                size: fs::metadata(hold).map_err(|e| e.to_string())?.len(),
                sha256: s(expected, "")?.into(),
            },
            job,
        );
    }
    let remaining = signature(hold, job)?;
    if remaining
        .as_object()
        .ok_or("파일 서명 오류")?
        .iter()
        .any(|(p, v)| expected.get(p) != Some(v))
    {
        return Err("정리 보관소에 새 파일·변경된 파일이 있습니다.".into());
    }
    let tree = scan_tree(hold, job)?;
    if let Some(expected_ids) = expected_ids {
        for entry in &tree.files {
            if json!(entry.identity) != expected_ids[&entry.path] {
                return Err("정리할 원본 파일의 ID가 바뀌었습니다.".into());
            }
        }
    }
    for entry in &tree.files {
        job.check()?;
        crate::verified_cleanup::delete_verified_file(
            &hold.join(&entry.path),
            &destination.join(&entry.path),
            entry,
            job,
        )?;
    }
    let mut dirs = tree.directories;
    dirs.sort_by_key(|p| std::cmp::Reverse(p.split('/').count()));
    for p in dirs {
        job.check()?;
        fs::remove_dir(hold.join(p)).map_err(|e| e.to_string())?;
    }
    fs::remove_dir(hold).map_err(|e| e.to_string())
}

// Remove verified workspace and redundant rollout bytes from all owned bundle
// attempts. Keep small manifests/checksums; source conversation recovery is a
// separate, explicitly reported directory on the selected destination volume.
fn cleanup_bundles(
    source: &Adapter,
    central: &Journal,
    id: &str,
    plan: &Value,
    job: &Job,
) -> Result<(), String> {
    if plan["request"]["kind"] == "files" {
        return Ok(());
    }
    let events = central.events(id)?;
    // Include incomplete export attempts, not only the last successful bundle.
    for intent in events.iter().filter(|e| e["phase"] == "bundle-intent") {
        let intent = &intent["payload"];
        let i = intent["index"].as_u64().ok_or("백업 선택 기록 누락")?;
        let bundle = Path::new(s(intent, "path")?);
        if !bundle.exists() {
            continue;
        }
        let created = events
            .iter()
            .find(|e| e["phase"] == "bundle-created" && e["payload"]["path"] == intent["path"]);
        if let Some(created) = created {
            if identity(bundle)? != created["payload"]["identity"] {
                return Err("임시 백업 폴더의 소유권이 변경됐습니다.".into());
            }
        } else {
            let tree = scan_tree(bundle, job)?;
            if tree.files.is_empty() && tree.directories.is_empty() {
                continue;
            }
            return Err("임시 백업의 소유권 기록을 확인할 수 없습니다. 원본을 덮어쓰지 않고 복구를 중단합니다.".into());
        }
        for (ri, r) in arr(plan, "roots")?
            .iter()
            .filter(|r| r["group"] == i)
            .enumerate()
        {
            let staged = bundle.join(format!("files/root-{:02}", ri + 1));
            if staged.exists() {
                cleanup_hold(
                    &staged,
                    Path::new(s(r, "destination")?),
                    &r["signature"],
                    None,
                    job,
                )?;
            }
        }
    }
    for intent in events.iter().filter(|e| e["phase"] == "bundle-intent") {
        let bundle = Path::new(s(&intent["payload"], "path")?);
        if !bundle.exists() {
            continue;
        }
        for stamp in arr(plan, "threadStamps")? {
            let tid = s(&stamp["thread"], "id")?;
            let path = bundle.join("conversations").join(format!("{tid}.jsonl"));
            if !path.exists() {
                continue;
            }
            let original = PathBuf::from(s(&stamp["thread"], "path")?);
            let reference = if original.exists() {
                original
            } else {
                source
                    .journal
                    .history()?
                    .iter()
                    .flat_map(|r| r["payload"]["saved"].as_array().into_iter().flatten())
                    .find(|v| v["id"] == tid)
                    .and_then(|v| v["path"].as_str())
                    .map(PathBuf::from)
                    .ok_or("대화 복구 사본을 확인할 수 없습니다.")?
            };
            let entry = Entry {
                path: String::new(),
                identity: Some(identity(&path)?),
                size: fs::metadata(&path).map_err(|e| e.to_string())?.len(),
                sha256: s(stamp, "sha256")?.into(),
            };
            crate::verified_cleanup::delete_verified_file(&path, &reference, &entry, job)?;
        }
    }
    Ok(())
}

// Retire BEFORE allocating a replacement. Every extant entry must have its
// original identity and content receipt; a hard-crash partial or any edit stops
// the retry without allocating another attempt. Historical receipts are retained.
fn retire_incomplete_bundles(
    central: &Journal,
    id: &str,
    index: usize,
    job: &Job,
) -> Result<(), String> {
    let events = central.events(id)?;
    for intent in events
        .iter()
        .filter(|e| e["phase"] == "bundle-intent" && e["payload"]["index"] == index)
    {
        let bundle = Path::new(s(&intent["payload"], "path")?);
        if !bundle.exists() {
            continue;
        }
        let created = events
            .iter()
            .find(|e| e["phase"] == "bundle-created" && e["payload"]["path"] == json!(bundle))
            .ok_or("임시 백업 소유권 기록 누락: 안전 종료에서 보관 위치를 확인하세요.")?;
        if identity(bundle)? != created["payload"]["identity"] {
            return Err("임시 백업 폴더가 교체됐습니다. 그대로 보관합니다.".into());
        }
        let tree = scan_tree(bundle, job)?;
        let receipts = events
            .iter()
            .filter(|e| {
                e["phase"] == "bundle-owned-entry" && e["payload"]["bundle"] == json!(bundle)
            })
            .map(|e| &e["payload"])
            .collect::<Vec<_>>();
        let receipt = |path: &Path| -> Result<&Value, String> {
            let r = receipts
                .iter()
                .rev()
                .find(|r| {
                    r["path"]
                        .as_str()
                        .is_some_and(|p| key(Path::new(p)) == key(path))
                })
                .ok_or(
                    "미확인 임시 파일·폴더를 보존합니다. 안전 종료에서 보관 위치를 확인하세요.",
                )?;
            if identity(path)? != r["identity"] {
                return Err("임시 파일·폴더가 교체됐습니다. 그대로 보관합니다.".into());
            }
            Ok(r)
        };
        // Validate the WHOLE attempt before removing any entry.
        for dir in &tree.directories {
            receipt(&bundle.join(dir))?;
        }
        for f in &tree.files {
            let path = bundle.join(&f.path);
            let r = receipt(&path)?;
            if r["file"] != true || r["sha256"] != f.sha256 {
                return Err("임시 파일이 변경됐습니다. 그대로 보관합니다.".into());
            }
            if let Some(reference) = r["reference"].as_str() {
                let reference = Path::new(reference);
                if identity(reference)? != r["referenceIdentity"]
                    || digest(reference, job)? != f.sha256
                {
                    return Err("임시 파일의 원본이 변경됐습니다. 양쪽 파일을 보관합니다.".into());
                }
            }
        }
        for f in &tree.files {
            let path = bundle.join(&f.path);
            let r = receipt(&path)?;
            if let Some(reference) = r["reference"].as_str() {
                crate::verified_cleanup::delete_verified_file(&path, Path::new(reference), f, job)?;
            } else {
                // Generated control bytes need no recovery copy, but use the
                // same handle-bound identity/content checked deletion primitive.
                crate::verified_cleanup::delete_verified_file(&path, &path, f, job)?;
            }
        }
        let mut dirs = tree.directories;
        dirs.sort_by_key(|p| std::cmp::Reverse(p.len()));
        for dir in dirs {
            let path = bundle.join(dir);
            receipt(&path)?;
            fs::remove_dir(path).map_err(|e| e.to_string())?;
        }
        if identity(bundle)? != created["payload"]["identity"] {
            return Err("임시 백업 소유권 변경".into());
        }
        fs::remove_dir(bundle).map_err(|e| e.to_string())?;
        central.record(id, "bundle-retired", json!({"index":index,"path":bundle}))?;
    }
    Ok(())
}

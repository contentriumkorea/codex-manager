use crate::transfer::{read_locked, PartialFile};
use crate::{
    absolute, all_threads, clean_path, digest, ensure_plain, identity, rename_new,
    valid_bundle_path, Adapter, Job,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub struct ThreadImport<'a> {
    pub source: &'a Path,
    pub id: &'a str,
    pub cwd: &'a Path,
    pub runtime: &'a [PathBuf],
    pub project: Option<&'a str>,
    pub previous_project: Option<&'a str>,
    pub title: &'a str,
    pub archived: bool,
    pub expected_sha256: Option<&'a str>,
}

fn key(path: &Path) -> String {
    clean_path(path)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}
fn within(path: &Path, parent: &Path) -> bool {
    key(path).starts_with(&(key(parent) + "/"))
}
pub fn rollout_target(home: &Path, id: &str) -> Result<PathBuf, String> {
    absolute(home)?;
    uuid::Uuid::parse_str(id).map_err(|_| "대화 ID 형식 오류")?;
    let now = chrono::Utc::now();
    let dir = home
        .join("sessions")
        .join(now.format("%Y/%m/%d").to_string());
    ensure_plain(&dir)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for offset in 0..1000 {
        let path = dir.join(format!(
            "rollout-{}-{id}.jsonl",
            (now + chrono::Duration::seconds(offset)).format("%Y-%m-%dT%H-%M-%S")
        ));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("대화 저장 파일 이름을 확보할 수 없습니다.".into())
}
pub fn prepare_rollout(
    source: &Path,
    home: &Path,
    id: &str,
    cwd: &Path,
    paths: &[PathBuf],
) -> Result<PathBuf, String> {
    prepare_rollout_with_job(source, home, id, cwd, paths, None, &Job::new())
}
pub fn prepare_rollout_with_job(
    source: &Path,
    home: &Path,
    id: &str,
    cwd: &Path,
    paths: &[PathBuf],
    expected: Option<&str>,
    job: &Job,
) -> Result<PathBuf, String> {
    job.check()?;
    absolute(home)?;
    absolute(cwd)?;
    for path in paths {
        absolute(path)?;
    }
    uuid::Uuid::parse_str(id).map_err(|_| "대화 ID 형식 오류")?;
    let mut reader = crate::RolloutReader::open(source, Some(id), job)?;
    let size = reader.raw_size();
    let hash = reader.raw_sha256().to_owned();
    if expected.is_some_and(|expected| !hash.eq_ignore_ascii_case(expected)) {
        return Err("검증 이후 백업 대화가 변경됐습니다.".into());
    }
    let mut context = None;
    let mut environment = None;
    let mut settings = None;
    let mut count = 0;
    while let Some(record) = reader.next_record()? {
        let item = record.value;
        if count == 0
            && (item["type"] != "session_meta"
                || item["payload"]["id"] != id
                || !item["payload"].is_object())
        {
            return Err("대화 원본의 ID 또는 형식이 일치하지 않습니다.".into());
        }
        if item["type"] == "turn_context" {
            if !item["payload"].is_object() {
                return Err("대화 작업 설정 형식 오류".into());
            }
            context = Some(count);
        }
        if item["type"] == "event_msg" && item["payload"]["type"] == "thread_settings_applied" {
            if !item["payload"]["thread_settings"].is_object() {
                return Err("대화 설정 이벤트 형식 오류".into());
            }
            settings = Some(count);
        }
        if item["type"] == "world_state"
            && item["payload"]["state"]["environments"]["environments"]["local"]
                .get("cwd")
                .is_some()
        {
            environment = Some(count);
        }
        count += 1;
    }
    if count == 0 {
        return Err("대화 기록이 비어 있습니다.".into());
    }
    let mut reader = crate::RolloutReader::open(source, Some(id), job)?;
    if reader.raw_sha256() != hash {
        return Err("Rollout changed between reads".into());
    }
    let target = rollout_target(home, id)?;
    let temporary =
        target.with_file_name(format!(".manager-import-{}.partial", uuid::Uuid::new_v4()));
    let mut output = PartialFile::create(&temporary)?;
    job.phase("대화 이식 파일 준비", size);
    let mut done = 0;
    let mut index = 0;
    while let Some(record) = reader.next_record()? {
        let line = record.line;
        let n = line.len();
        if index == 0
            || Some(index) == context
            || Some(index) == settings
            || Some(index) == environment
        {
            let mut item: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            if Some(index) == environment {
                item["payload"]["state"]["environments"]["environments"]["local"]["cwd"] =
                    json!(cwd);
            } else {
                let settings = if Some(index) == settings {
                    &mut item["payload"]["thread_settings"]
                } else {
                    &mut item["payload"]
                };
                settings["cwd"] = json!(cwd);
                settings["runtime_workspace_roots"] = json!(paths);
            }
            writeln!(output.writer(), "{item}").map_err(|e| e.to_string())?;
        } else {
            output
                .writer()
                .write_all(line.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        done += n as u64;
        job.advance(done, &source.to_string_lossy(), n as u64);
        index += 1;
    }
    if index != count || reader.raw_sha256() != hash {
        return Err("대화 이식 파일을 준비하는 동안 원본이 변경됐습니다.".into());
    }
    job.check()?;
    output.publish(&target)?;
    Ok(target)
}
impl Adapter {
    pub(crate) fn rollout_owner(&self, operation: &str, id: &str) -> Result<Option<Value>, String> {
        Ok(self
            .journal
            .events(operation)?
            .into_iter()
            .rev()
            .find(|e| {
                [
                    "rollout-publish-intent",
                    "rollout-published",
                    "rollout-location",
                ]
                .contains(&e["phase"].as_str().unwrap_or(""))
                    && e["payload"]["threadId"] == id
            })
            .map(|e| e["payload"].clone()))
    }
    fn checked_rollout_path(&self, path: &Path) -> Result<(), String> {
        absolute(path)?;
        if !within(path, &self.store.home.join("sessions"))
            && !within(path, &self.store.home.join("archived_sessions"))
        {
            return Err("가져온 대화 파일이 저장소 범위 밖에 있습니다.".into());
        }
        Ok(())
    }
    pub(crate) fn owns_rollout(
        &self,
        operation: &str,
        thread: &Value,
        job: &Job,
    ) -> Result<bool, String> {
        let Some(owner) =
            self.rollout_owner(operation, thread["id"].as_str().ok_or("대화 ID 오류")?)?
        else {
            return Ok(false);
        };
        let path = Path::new(
            thread["path"]
                .as_str()
                .ok_or("대화 기록 경로가 없습니다.")?,
        );
        self.checked_rollout_path(path)?;
        if !path.exists() && !path.with_extension("jsonl.zst").exists() {
            return Ok(false);
        }
        let resolved = crate::resolve_rollout_path(path)?;
        let path = resolved.as_path();
        let planned = Path::new(owner["path"].as_str().ok_or("가져오기 파일 계획 누락")?);
        let logical = |p: &Path| {
            if p.extension().is_some_and(|e| e == "zst") {
                p.with_extension("")
            } else {
                p.to_owned()
            }
        };
        if key(&logical(path)) != key(&logical(planned))
            && !(within(path, &self.store.home.join("archived_sessions"))
                && logical(path).file_name() == logical(planned).file_name())
        {
            return Ok(false);
        }
        if !path.is_file() {
            return Ok(false);
        }
        if owner["identity"]
            .as_str()
            .is_some_and(|id| identity(path).is_ok_and(|actual| actual == id))
        {
            return Ok(true);
        }
        if let Some(hash) = owner["derivedSha256"].as_str() {
            if digest(path, job)? == hash {
                return Ok(true);
            }
        }
        if let Some(hash) = owner["decodedSha256"].as_str() {
            return crate::decoded_rollout_digest(path, job).map(|actual| actual == hash);
        }
        Ok(false)
    }
    pub(crate) fn restore_name(&mut self, id: &str, title: &str) -> Result<(), String> {
        let title = title.trim();
        if title.is_empty() {
            return Ok(());
        }
        self.call("thread/name/set", json!({"threadId":id,"name":title}))?;
        if self.read_thread(id)?["name"] != title {
            return Err("가져온 대화 제목을 확인할 수 없습니다.".into());
        }
        Ok(())
    }
    pub(crate) fn register_rollout(
        &mut self,
        path: &Path,
        request: &ThreadImport<'_>,
        job: &Job,
    ) -> Result<Value, String> {
        job.check()?;
        self.guard()?;
        self.checked_rollout_path(path)?;
        let result=self.call("thread/resume",json!({"threadId":request.id,"path":path,"cwd":request.cwd,"runtimeWorkspaceRoots":request.runtime}))?;
        if result["thread"]["id"] != request.id {
            return Err("가져온 대화 ID가 일치하지 않습니다.".into());
        }
        Ok(result["thread"].clone())
    }
    pub fn import_thread(
        &mut self,
        source: &Path,
        id: &str,
        cwd: &Path,
        paths: &[PathBuf],
        pid: &str,
        title: &str,
    ) -> Result<PathBuf, String> {
        let request = ThreadImport {
            source,
            id,
            cwd,
            runtime: paths,
            project: if pid.is_empty() { None } else { Some(pid) },
            previous_project: None,
            title,
            archived: false,
            expected_sha256: None,
        };
        let job = Job::new();
        self.guard()?;
        let target =
            prepare_rollout_with_job(source, &self.store.home, id, cwd, paths, None, &job)?;
        self.register_rollout(&target, &request, &job)?;
        self.assign(id, request.project)?;
        self.restore_name(id, title)?;
        Ok(target)
    }
    pub(crate) fn validate_owned_import(
        &mut self,
        request: &ThreadImport<'_>,
        operation: &str,
        thread: &Value,
        job: &Job,
    ) -> Result<PathBuf, String> {
        if !self.owns_rollout(operation, thread, job)? {
            return Err("같은 ID의 다른 대화가 있습니다. 현재 기록은 덮어쓰지 않습니다.".into());
        }
        let raw = self.read_thread(request.id)?;
        let events = self.journal.events(operation)?;
        let assigned = events
            .iter()
            .any(|e| e["phase"] == "thread-assigned" && e["payload"]["threadId"] == request.id);
        let pid = raw["projectId"].as_str();
        if pid != request.project
            && (assigned || (pid.is_some() && pid != request.previous_project))
        {
            return Err("가져온 대화의 소속이 이후 변경됐습니다.".into());
        }
        let wanted = request.title.trim();
        let initial = events
            .iter()
            .rev()
            .find(|e| e["phase"] == "thread-registered" && e["payload"]["threadId"] == request.id)
            .map(|e| &e["payload"]["name"]);
        let named = events
            .iter()
            .any(|e| e["phase"] == "thread-named" && e["payload"]["threadId"] == request.id);
        if !raw["name"].is_null()
            && raw["name"] != wanted
            && (named || initial != Some(&raw["name"]))
        {
            return Err("가져온 대화 제목이 이후 변경됐습니다.".into());
        }
        let path = PathBuf::from(thread["path"].as_str().ok_or("가져온 대화 경로 누락")?);
        if key(Path::new(raw["cwd"].as_str().ok_or("대화 작업 경로 누락")?)) != key(request.cwd)
            || crate::runtime_roots_with_job(&path, job)? != request.runtime
        {
            return Err("가져온 대화의 작업 폴더가 이후 변경됐습니다.".into());
        }
        if crate::workflow::messages_digest(&path, job)?
            != crate::workflow::messages_digest(&crate::resolve_rollout_path(request.source)?, job)?
        {
            return Err("중단 후 대화 내용이 변경됐습니다.".into());
        }
        Ok(path)
    }
    pub(crate) fn import_owned(
        &mut self,
        request: &ThreadImport<'_>,
        operation: &str,
        job: &Job,
    ) -> Result<PathBuf, String> {
        self.guard()?;
        job.check()?;
        if !valid_bundle_path(operation) || operation.contains('/') {
            return Err("가져오기 작업 ID 오류".into());
        }
        let resolved_source = crate::resolve_rollout_path(request.source)?;
        let source_lock = read_locked(&resolved_source)?;
        if let Some(expected) = request.expected_sha256 {
            if !digest(&crate::resolve_rollout_path(request.source)?, job)?
                .eq_ignore_ascii_case(expected)
            {
                return Err("검증 이후 백업 대화가 변경됐습니다.".into());
            }
        }
        let existing = all_threads(&self.snapshot()?)
            .into_iter()
            .find(|t| t["id"] == request.id);
        let owner = self.rollout_owner(operation, request.id)?;
        let target = if let Some(thread) = &existing {
            self.validate_owned_import(request, operation, thread, job)?
        } else if let Some(owner) = owner {
            let planned = PathBuf::from(owner["path"].as_str().ok_or("가져오기 파일 계획 누락")?);
            let target = if planned.exists() || planned.with_extension("jsonl.zst").exists() {
                crate::resolve_rollout_path(&planned)?
            } else {
                planned
            };
            self.checked_rollout_path(&target)?;
            if target.exists() {
                let fake = json!({"id":request.id,"path":target});
                if !self.owns_rollout(operation, &fake, job)? {
                    return Err("가져오기 대상 파일이 다른 파일로 변경됐습니다.".into());
                }
                target
            } else {
                let stage = PathBuf::from(
                    owner["stage"]
                        .as_str()
                        .ok_or("가져오기 준비 파일 계획 누락")?,
                );
                let staging = self
                    .store
                    .home
                    .join(".codex-manager-import-stage")
                    .join(operation);
                absolute(&stage)?;
                if !within(&stage, &staging)
                    || !stage.is_file()
                    || identity(&stage)? != owner["identity"].as_str().ok_or("준비 파일 ID 누락")?
                {
                    return Err("중단된 대화 준비 파일을 확인할 수 없습니다.".into());
                }
                self.require_closed()?;
                fs::create_dir_all(target.parent().ok_or("대화 저장 폴더 누락")?)
                    .map_err(|e| e.to_string())?;
                rename_new(&stage, &target)?;
                target
            }
        } else {
            let staging = self
                .store
                .home
                .join(".codex-manager-import-stage")
                .join(operation);
            let stage = prepare_rollout_with_job(
                request.source,
                &staging,
                request.id,
                request.cwd,
                request.runtime,
                request.expected_sha256,
                job,
            )?;
            let target = rollout_target(&self.store.home, request.id)?;
            self.journal.record(operation,"rollout-publish-intent",json!({"threadId":request.id,"projectId":request.project,"stage":stage,"path":target,"identity":identity(&stage)?,"derivedSha256":digest(&stage,job)?,"decodedSha256":crate::decoded_rollout_digest(&stage,job)?}))?;
            job.check()?;
            self.require_closed()?;
            rename_new(&stage, &target)?;
            target
        };
        self.journal.record(operation,"rollout-published",json!({"threadId":request.id,"projectId":request.project,"path":target,"identity":identity(&target)?,"derivedSha256":digest(&target,job)?,"decodedSha256":crate::decoded_rollout_digest(&target,job)?}))?;
        if existing.is_none() {
            self.journal.record(
                operation,
                "registering",
                json!({"threadId":request.id,"projectId":request.project}),
            )?;
            let thread = self.register_rollout(&target, request, job)?;
            self.journal.record(
                operation,
                "thread-registered",
                json!({"threadId":request.id,"projectId":request.project,"name":thread["name"]}),
            )?;
        }
        job.check()?;
        let current = self.read_thread(request.id)?;
        let needs_assignment = current["projectId"].as_str() != request.project;
        let needs_name = current["name"].as_str() != Some(request.title.trim());
        let mut archived = existing.as_ref().is_some_and(|t| t["archived"] == true);
        if archived && (needs_assignment || needs_name) {
            self.call("thread/unarchive", json!({"threadId":request.id}))?;
            archived = false;
        }
        if needs_assignment {
            self.assign(request.id, request.project)?;
        }
        self.journal.record(
            operation,
            "thread-assigned",
            json!({"threadId":request.id,"projectId":request.project}),
        )?;
        job.check()?;
        if needs_name {
            self.restore_name(request.id, request.title)?;
        }
        self.journal.record(
            operation,
            "thread-named",
            json!({"threadId":request.id,"name":request.title.trim()}),
        )?;
        if request.archived && !archived {
            job.check()?;
            self.call("thread/archive", json!({"threadId":request.id}))?;
        }
        let current = all_threads(&self.snapshot()?)
            .into_iter()
            .find(|t| t["id"] == request.id)
            .ok_or("가져온 대화가 목록에 없습니다.")?;
        let location = Path::new(current["path"].as_str().ok_or("가져온 대화 경로 누락")?);
        self.checked_rollout_path(location)?;
        self.release_server();
        self.journal.record(operation,"rollout-location",json!({"threadId":request.id,"projectId":request.project,"path":location,"identity":identity(location)?,"derivedSha256":digest(location,job)?,"decodedSha256":crate::decoded_rollout_digest(location,job)?}))?;
        drop(source_lock);
        Ok(location.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Journal, Store};
    #[test]
    fn invalid_or_changed_rollout_is_never_published() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let source = root.join("invalid.jsonl");
        let id = "00000000-0000-4000-8000-000000000077";
        fs::write(
            &source,
            format!(
                "{}\ninvalid\n",
                json!({"type":"session_meta","payload":{"id":id}})
            ),
        )
        .unwrap();
        assert!(prepare_rollout_with_job(
            &source,
            &root.join("home"),
            id,
            &root,
            &[],
            None,
            &Job::new()
        )
        .is_err());
        assert!(!root.join("home/sessions").exists());
        fs::write(
            &source,
            format!("{}\n", json!({"type":"session_meta","payload":{"id":id}})),
        )
        .unwrap();
        assert!(prepare_rollout_with_job(
            &source,
            &root.join("home"),
            id,
            &root,
            &[],
            Some(&"0".repeat(64)),
            &Job::new()
        )
        .is_err());
        assert!(!root.join("home/sessions").exists());
    }

    #[test]
    fn compressed_owned_rollout_requires_full_decoded_receipt() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        let home = root.join("home");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let adapter = Adapter::isolated(
            crate::resolve_store(&home).unwrap(),
            root.join("missing.exe"),
            crate::Journal::open(&root.join("journal.sqlite")).unwrap(),
        )
        .unwrap();
        let id = "00000000-0000-4000-8000-000000000077";
        let planned = home.join("sessions/rollout.jsonl");
        let text = format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":id,"cwd":root}})
        );
        fs::write(&planned, &text).unwrap();
        let op = adapter.journal.begin(&json!({"kind":"test"})).unwrap();
        let raw = digest(&planned, &Job::new()).unwrap();
        adapter.journal.record(&op,"rollout-location",json!({"threadId":id,"path":planned,"identity":identity(&planned).unwrap(),"decodedSha256":raw})).unwrap();
        let zst = planned.with_extension("jsonl.zst");
        fs::write(&zst, zstd::stream::encode_all(text.as_bytes(), 1).unwrap()).unwrap();
        fs::remove_file(&planned).unwrap();
        let thread = json!({"id":id,"path":planned});
        assert!(adapter.owns_rollout(&op, &thread, &Job::new()).unwrap());
        let changed = text.replace("session_meta", "turn_context");
        fs::write(
            &zst,
            zstd::stream::encode_all(changed.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        assert!(!adapter
            .owns_rollout(&op, &thread, &Job::new())
            .unwrap_or(false));
    }
    #[test]
    #[ignore = "requires current installed Codex and a fresh isolated home"]
    fn resume_after_registration_before_assignment_and_archived_repeat() {
        let root =
            PathBuf::from(std::env::var_os("CODEX_TEST_HOME").unwrap()).join("partial-import");
        assert!(!root.exists());
        fs::create_dir_all(root.join("home")).unwrap();
        let workspace = root.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let home = root.join("home");
        let mut adapter = Adapter::isolated(
            Store {
                home: home.clone(),
                sqlite_home: home.clone(),
                database: None,
            },
            std::env::var_os("CODEX_TEST_BINARY").unwrap().into(),
            Journal::open(&root.join("state/journal.sqlite")).unwrap(),
        )
        .unwrap();
        let p = adapter
            .create("Partial", std::slice::from_ref(&workspace))
            .unwrap();
        let pid = p["id"].as_str().unwrap();
        let id = "00000000-0000-4000-8000-000000000078";
        let source = root.join("source.jsonl");
        fs::write(&source,format!("{}\n{}\n",json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":workspace,"originator":"codex_cli_rs","cli_version":"0.0.0","source":"cli","model_provider":"openai"}}),json!({"timestamp":"2026-11-02T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"recovery fixture"}]}}))).unwrap();
        let op = adapter.journal.begin(&json!({"kind":"fixture"})).unwrap();
        let paths = vec![workspace.clone()];
        let hash = digest(&source, &Job::new()).unwrap();
        let request = ThreadImport {
            source: &source,
            id,
            cwd: &workspace,
            runtime: &paths,
            project: Some(pid),
            previous_project: None,
            title: "Recovered",
            archived: true,
            expected_sha256: Some(&hash),
        };
        let target = prepare_rollout_with_job(
            &source,
            &home,
            id,
            &workspace,
            &paths,
            Some(&hash),
            &Job::new(),
        )
        .unwrap();
        adapter.journal.record(&op,"rollout-published",json!({"threadId":id,"path":target,"identity":identity(&target).unwrap(),"derivedSha256":digest(&target,&Job::new()).unwrap()})).unwrap();
        adapter
            .register_rollout(&target, &request, &Job::new())
            .unwrap();
        adapter.release_server();
        assert!(adapter.read_thread(id).unwrap()["projectId"].is_null());
        adapter.import_owned(&request, &op, &Job::new()).unwrap();
        adapter.release_server();
        assert_eq!(adapter.read_thread(id).unwrap()["name"], "Recovered");
        assert_eq!(adapter.read_thread(id).unwrap()["projectId"], pid);
        adapter.import_owned(&request, &op, &Job::new()).unwrap();
        assert_eq!(all_threads(&adapter.snapshot().unwrap()).len(), 1);
    }
}

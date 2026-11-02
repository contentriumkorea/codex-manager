use crate::desktop_state::{sync_state, CatalogDeletion};
use crate::{
    absolute, check_version, digest, ensure_plain, read_store, resolve_store, runtime_roots,
    Catalog, CodexProcess, Job, Journal, Store,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

pub struct Adapter {
    pub store: Store,
    pub binary: PathBuf,
    pub journal: Journal,
    isolated: bool,
    server: Option<CodexProcess>,
    mutation_lock: std::sync::Mutex<Option<fs::File>>,
    pub(crate) recovery_override: Option<PathBuf>,
    pub(crate) undo_attempt: std::sync::Mutex<Option<(String, bool)>>,
}
pub fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 256 || name.chars().any(char::is_control) {
        return Err("이름은 줄바꿈 없이 1~256자로 입력하세요.".into());
    }
    Ok(name.to_owned())
}
pub fn catalog_json(catalog: Catalog) -> Value {
    use sha2::{Digest, Sha256};
    let mut value = json!({"projects":catalog.projects.into_iter().map(|g|json!({"project":g.project,"threads":g.threads})).collect::<Vec<_>>(),"unassigned":catalog.unassigned,"unresolved":catalog.unresolved});
    value["revision"] = json!(format!(
        "{:x}",
        Sha256::digest(value.to_string().as_bytes())
    ));
    value
}
pub fn all_threads(catalog: &Catalog) -> Vec<Value> {
    catalog
        .projects
        .iter()
        .flat_map(|g| g.threads.iter())
        .chain(catalog.unassigned.iter())
        .chain(catalog.unresolved.iter())
        .cloned()
        .collect()
}
pub fn roots(project: &Value) -> Result<Vec<PathBuf>, String> {
    project["roots"]
        .as_array()
        .ok_or("프로젝트 폴더 목록 오류")?
        .iter()
        .map(|r| absolute(Path::new(r["path"].as_str().ok_or("폴더 경로 오류")?)))
        .collect()
}
impl Adapter {
    pub(crate) fn ensure_mutation_lock(&self) -> Result<(), String> {
        let mut lock = self.mutation_lock.lock().map_err(|_| "저장소 잠금 오류")?;
        if lock.is_some() {
            return Ok(());
        }
        let path = self.store.home.join(".codex-manager-write.lock");
        ensure_plain(&path)?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        *lock =
            Some(options.open(path).map_err(|_| {
                "다른 Codex Manager가 같은 저장소에서 작업하고 있습니다.".to_owned()
            })?);
        Ok(())
    }
    pub(crate) fn require_closed(&self) -> Result<(), String> {
        if self.isolated {
            return Ok(());
        }
        let owned = self
            .server
            .as_ref()
            .map(CodexProcess::pid)
            .into_iter()
            .collect::<Vec<_>>();
        crate::platform::desktop_closed_except(&owned)
    }
    pub(crate) fn isolated_checkpoint(&self, request: &Value, checkpoint: &str) -> bool {
        self.isolated && request["__testInterruptAt"] == checkpoint
    }
    pub fn protected_storage(&self) -> Vec<PathBuf> {
        let mut paths = vec![
            self.store.home.clone(),
            self.store.sqlite_home.clone(),
            self.journal.path.parent().unwrap().to_owned(),
        ];
        if let Ok(exe) = std::env::current_exe() {
            let name = exe
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if [
                "codexmanager.exe",
                "codex-manager-preview.exe",
                "codex-manager.exe",
            ]
            .contains(&name.as_str())
            {
                if let Some(parent) = exe.parent() {
                    paths.push(parent.to_owned());
                }
            }
        }
        paths
    }
    pub fn open(store: Store, binary: PathBuf, journal: Journal) -> Result<Self, String> {
        absolute(&binary)?;
        Ok(Self {
            store,
            binary,
            journal,
            isolated: false,
            server: None,
            mutation_lock: std::sync::Mutex::new(None),
            recovery_override: None,
            undo_attempt: std::sync::Mutex::new(None),
        })
    }
    pub fn isolated(store: Store, binary: PathBuf, journal: Journal) -> Result<Self, String> {
        let actual = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".codex")));
        if actual.as_ref().is_some_and(|p| p == &store.home) {
            return Err("격리 저장소가 실제 Codex 저장소와 같습니다.".into());
        }
        let mut adapter = Self::open(store, binary, journal)?;
        adapter.isolated = true;
        Ok(adapter)
    }
    pub fn snapshot(&self) -> Result<Catalog, String> {
        read_store(&resolve_store(&self.store.home)?)
    }
    pub fn guard(&self) -> Result<(), String> {
        self.ensure_mutation_lock()?;
        self.require_closed()?;
        ensure_plain(&self.store.home)?;
        check_version(&self.binary)?;
        Ok(())
    }
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.require_closed()?;
        if self.server.is_none() {
            self.guard()?;
            self.server = Some(CodexProcess::start(
                &self.binary,
                &self.store.home,
                Some(&self.store.sqlite_home),
            )?);
        }
        if ![
            "thread/read",
            "project/read",
            "project/list",
            "thread/list",
            "thread/unsubscribe",
        ]
        .contains(&method)
        {
            self.mark_undo_mutation()?;
        }
        self.server.as_mut().unwrap().request(method, params)
    }
    pub(crate) fn mark_undo_mutation(&self) -> Result<(), String> {
        let mut attempt = self
            .undo_attempt
            .lock()
            .map_err(|_| "복구 작업 잠금 오류")?;
        if let Some((id, started)) = attempt.as_mut() {
            if !*started {
                self.journal.record(id, "undo-mutating", json!({}))?;
                *started = true;
            }
        }
        Ok(())
    }
    fn reconcile_legacy_project(
        &mut self,
        operation: &str,
        payload: &Value,
        job: &Job,
    ) -> Result<String, String> {
        self.guard()?;
        job.check()?;
        let before = &payload["project"];
        let alias = before["id"].as_str().ok_or("기존 프로젝트 ID 누락")?;
        let paths = roots(before)?;
        let mut project=self.call("project/create",json!({"idempotencyKey":payload["projectKey"],"name":before["name"],"roots":paths.iter().map(|p|json!({"path":p})).collect::<Vec<_>>()}))?["project"].clone();
        if project["name"] != before["name"] || roots(&project)? != paths {
            return Err("변환 중인 프로젝트가 이후 변경됐습니다.".into());
        }
        let native = project["id"]
            .as_str()
            .ok_or("프로젝트 변환 결과 누락")?
            .to_owned();
        let threads = payload["threads"].as_array().ok_or("대화 연결 계획 누락")?;
        for thread in threads {
            job.check()?;
            let current = self.read_thread(thread["id"].as_str().ok_or("대화 ID 누락")?)?;
            if current["projectId"] != thread["nativeBefore"] && current["projectId"] != native {
                return Err("변환 중 대화 소속이 다른 곳에서 변경됐습니다.".into());
            }
        }
        project["legacyAlias"] = json!(alias);
        // Publish the alias before assigning conversations. The durable operation
        // retains the complete original selection across either crash boundary.
        self.sync(std::slice::from_ref(&project), &HashMap::new(), &[], &[])?;
        self.journal.record(
            operation,
            "legacy-alias-published",
            json!({"projectId":native}),
        )?;
        let mut assignments = HashMap::new();
        for thread in threads {
            job.check()?;
            let id = thread["id"].as_str().ok_or("대화 ID 누락")?;
            self.assign(id, Some(&native))?;
            assignments.insert(id.to_owned(), Some(native.clone()));
        }
        self.sync(&[project], &assignments, &[], &[])?;
        self.journal.record(
            operation,
            "completed",
            json!({"materialized":true,"projectId":native}),
        )?;
        Ok(native)
    }
    pub fn resume_legacy(&mut self, job: &Job) -> Result<bool, String> {
        let pending = self
            .journal
            .history()?
            .into_iter()
            .filter(|r| {
                r["payload"]["kind"] == "rust-legacy"
                    && r["payload"]["home"] == json!(self.store.home)
                    && !["completed", "cancelled"].contains(&r["state"].as_str().unwrap_or(""))
            })
            .collect::<Vec<_>>();
        for record in &pending {
            let id = record["id"].as_str().ok_or("연결 정리 ID 누락")?;
            if let Err(error) = self.reconcile_legacy_project(id, &record["payload"], job) {
                self.journal
                    .record(id, "needs_recovery", json!({"error":error}))?;
                return Err(error);
            }
        }
        Ok(!pending.is_empty())
    }
    pub fn materialize_legacy(
        &mut self,
        selected: &[String],
        job: &Job,
    ) -> Result<HashMap<String, String>, String> {
        if self.resume_legacy(job)? {
            return Err(
                "중단됐던 프로젝트 연결 정리를 완료했습니다. 새로 고침된 목록에서 다시 선택하세요."
                    .into(),
            );
        }
        let snapshot = self.snapshot()?;
        let mut mapping = HashMap::new();
        for group in snapshot
            .projects
            .iter()
            .filter(|g| g.project["legacy"] == true)
        {
            if !selected.iter().any(|id| {
                group.project["id"] == *id
                    || group
                        .threads
                        .iter()
                        .any(|t| t["id"] == *id && t["projectId"] == group.project["id"])
            }) {
                continue;
            }
            job.check()?;
            self.guard()?;
            let mut threads = group
                .threads
                .iter()
                .filter(|t| t["projectId"] == group.project["id"])
                .cloned()
                .collect::<Vec<_>>();
            for thread in &mut threads {
                thread["nativeBefore"] = self
                    .read_thread(thread["id"].as_str().ok_or("대화 ID 누락")?)?["projectId"]
                    .clone();
            }
            let payload = json!({"kind":"rust-legacy","project":group.project,"threads":threads,"projectKey":uuid::Uuid::new_v4().to_string(),"resources":[]});
            let operation = self.begin(payload.clone())?;
            match self.reconcile_legacy_project(&operation, &payload, job) {
                Ok(native) => {
                    mapping.insert(
                        group.project["id"]
                            .as_str()
                            .ok_or("프로젝트 ID 누락")?
                            .to_owned(),
                        native,
                    );
                }
                Err(error) => {
                    self.journal
                        .record(&operation, "needs_recovery", json!({"error":error}))?;
                    return Err(error);
                }
            }
        }
        Ok(mapping)
    }
    pub fn read_thread(&mut self, id: &str) -> Result<Value, String> {
        Ok(
            self.call("thread/read", json!({"threadId":id,"includeTurns":false}))?["thread"]
                .clone(),
        )
    }
    pub fn create(&mut self, name: &str, paths: &[PathBuf]) -> Result<Value, String> {
        self.create_key(name, paths, &uuid::Uuid::new_v4().to_string())
    }
    pub(crate) fn create_key(
        &mut self,
        name: &str,
        paths: &[PathBuf],
        key: &str,
    ) -> Result<Value, String> {
        let name = valid_name(name)?;
        for path in paths {
            absolute(path)?;
        }
        let project=self.call("project/create",json!({"idempotencyKey":key,"name":name,"roots":paths.iter().map(|p|json!({"path":p})).collect::<Vec<_>>()}))?["project"].clone();
        if project["id"].as_str().is_none() {
            return Err("프로젝트 생성 결과를 확인할 수 없습니다.".into());
        }
        self.sync(std::slice::from_ref(&project), &HashMap::new(), &[], &[])?;
        Ok(project)
    }
    pub(crate) fn sync(
        &self,
        projects: &[Value],
        assignments: &HashMap<String, Option<String>>,
        removed: &[String],
        deleted: &[String],
    ) -> Result<(), String> {
        self.require_closed()?;
        sync_state(
            &self.store.home,
            projects,
            assignments,
            removed,
            deleted,
            &self.recovery_root(),
        )
    }
    pub fn recovery_root(&self) -> PathBuf {
        self.recovery_override
            .clone()
            .unwrap_or_else(|| self.journal.path.parent().unwrap().join("recovery"))
    }
    pub fn release_server(&mut self) {
        self.server = None;
    }
    pub fn update_project(&mut self, project: &Value) -> Result<(), String> {
        let id = project["id"].as_str().ok_or("프로젝트 ID 오류")?;
        let name = valid_name(project["name"].as_str().ok_or("프로젝트 이름 오류")?)?;
        let paths = roots(project)?;
        self.call("project/update",json!({"projectId":id,"name":name,"roots":paths.iter().map(|p|json!({"path":p})).collect::<Vec<_>>()}))?;
        let read = self.call("project/read", json!({"projectId":id}))?["project"].clone();
        if read["name"] != project["name"] || roots(&read)? != paths {
            return Err("프로젝트 변경 재조회가 일치하지 않습니다.".into());
        }
        Ok(())
    }
    pub(crate) fn set_name(&mut self, id: &str, name: &str) -> Result<(), String> {
        let name = valid_name(name)?;
        self.call("thread/name/set", json!({"threadId":id,"name":name}))?;
        if self.read_thread(id)?["name"] != name {
            return Err("대화 제목 변경이 저장되지 않았습니다.".into());
        }
        Ok(())
    }
    pub(crate) fn assign(&mut self, id: &str, project: Option<&str>) -> Result<(), String> {
        self.call(
            "thread/metadata/update",
            json!({"threadId":id,"projectId":project.unwrap_or("")}),
        )?;
        if self.read_thread(id)?["projectId"].as_str() != project {
            return Err("대화 소속 변경이 저장되지 않았습니다.".into());
        }
        Ok(())
    }
    pub fn relocate(
        &mut self,
        thread: &Value,
        cwd: &Path,
        paths: &[PathBuf],
    ) -> Result<(), String> {
        let id = thread["id"].as_str().ok_or("대화 ID 오류")?;
        absolute(cwd)?;
        for p in paths {
            absolute(p)?;
        }
        let current = self.read_thread(id)?;
        if current["status"]["type"] == "active" {
            return Err("실행 중인 대화는 변경할 수 없습니다.".into());
        }
        // Resume of an already loaded thread can retain its additional roots.
        // Unload only our idle session before applying an exact workspace list.
        if self
            .call("thread/unsubscribe", json!({"threadId":id}))
            .is_err()
        {
            self.release_server();
        }
        self.call(
            "thread/resume",
            json!({"threadId":id,"cwd":cwd,"runtimeWorkspaceRoots":paths}),
        )?;
        self.call("thread/settings/update", json!({"threadId":id,"cwd":cwd}))?;
        self.assign(id, thread["projectId"].as_str())?;
        let read = self.read_thread(id)?;
        if crate::clean_path(Path::new(
            read["cwd"].as_str().ok_or("변경한 작업 경로가 없습니다.")?,
        )) != crate::clean_path(cwd)
        {
            return Err("작업 경로 변경이 저장되지 않았습니다.".into());
        }
        let catalog = self.snapshot()?;
        let saved = all_threads(&catalog)
            .into_iter()
            .find(|t| t["id"] == id)
            .ok_or("변경된 대화를 찾을 수 없습니다.")?;
        let actual = runtime_roots(Path::new(
            saved["path"].as_str().ok_or("대화 기록 경로 누락")?,
        ))?;
        if actual != paths {
            return Err("작업 폴더 목록 변경이 저장되지 않았습니다.".into());
        }
        Ok(())
    }
    pub(crate) fn project(&self, id: &str) -> Result<Value, String> {
        self.snapshot()?
            .projects
            .into_iter()
            .find(|g| g.project["id"] == id)
            .map(|g| g.project)
            .ok_or_else(|| "프로젝트가 바뀌었습니다. 새로고침하세요.".into())
    }
    fn selected_threads(&mut self, ids: &[String]) -> Result<Vec<Value>, String> {
        if ids.is_empty() {
            return Err("대화를 선택하세요.".into());
        }
        let catalog = self.snapshot()?;
        let threads = all_threads(&catalog);
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for id in ids {
            if !seen.insert(id) {
                return Err("선택에 중복 대화가 있습니다.".into());
            }
            let thread = threads
                .iter()
                .find(|t| t["id"] == *id)
                .cloned()
                .ok_or("선택한 대화가 바뀌었습니다.")?;
            let current = self.read_thread(id)?;
            if current["status"]["type"] == "active" {
                return Err("실행 중인 대화를 종료한 뒤 변경하세요.".into());
            }
            result.push(thread);
        }
        Ok(result)
    }
    pub(crate) fn begin(&self, mut payload: Value) -> Result<String, String> {
        payload["home"] = json!(self.store.home);
        let mut resources = payload["resources"].as_array().cloned().unwrap_or_default();
        resources.push(json!(format!("home:{}", self.store.home.display())));
        payload["resources"] = json!(resources);
        self.journal.begin(&payload)
    }
    pub(crate) fn finish(&self, op: &str, result: Result<(), String>) -> Result<String, String> {
        match result {
            Ok(()) => {
                self.journal.record(op, "completed", json!({}))?;
                Ok(op.to_owned())
            }
            Err(e) => {
                self.journal
                    .record(op, "needs_recovery", json!({"error":e}))?;
                Err(format!("{e}\n작업 내역에서 복구할 수 있습니다. ({op})"))
            }
        }
    }
    pub fn rename_project(&mut self, id: &str, name: &str) -> Result<String, String> {
        self.guard()?;
        let before = self.project(id)?;
        let mut after = before.clone();
        after["name"] = json!(valid_name(name)?);
        let op=self.begin(json!({"kind":"rust-project-edit","projects":[before],"afterProjects":[after],"resources":[id]}))?;
        let result = (|| {
            self.update_project(&after)?;
            self.sync(&[after], &HashMap::new(), &[], &[])
        })();
        self.finish(&op, result)
    }
    pub fn set_roots(&mut self, id: &str, paths: &[PathBuf]) -> Result<String, String> {
        self.guard()?;
        for p in paths {
            absolute(p)?;
        }
        let before = self.project(id)?;
        let mut after = before.clone();
        after["roots"] = json!(paths.iter().map(|p| json!({"path":p})).collect::<Vec<_>>());
        let op=self.begin(json!({"kind":"rust-project-edit","projects":[before],"afterProjects":[after],"resources":[id]}))?;
        let result = (|| {
            self.update_project(&after)?;
            self.sync(&[after], &HashMap::new(), &[], &[])
        })();
        self.finish(&op, result)
    }
    pub fn rename_threads(&mut self, ids: &[String], names: &[String]) -> Result<String, String> {
        self.guard()?;
        if names.len() != ids.len() {
            return Err("제목 개수가 일치하지 않습니다.".into());
        }
        for name in names {
            valid_name(name)?;
        }
        let threads = self.selected_threads(ids)?;
        let op=self.begin(json!({"kind":"rust-thread-names","threads":threads,"names":names,"resources":ids.iter().map(|id|format!("thread:{id}")).collect::<Vec<_>>()}))?;
        let result = (|| {
            for (id, name) in ids.iter().zip(names) {
                self.journal
                    .record(&op, "renaming", json!({"threadId":id}))?;
                self.set_name(id, name)?;
            }
            Ok(())
        })();
        self.finish(&op, result)
    }
    pub fn move_threads(&mut self, ids: &[String], target: &str) -> Result<String, String> {
        self.guard()?;
        let project = self.project(target)?;
        let threads = self.selected_threads(ids)?;
        let op=self.begin(json!({"kind":"rust-assign","threads":threads,"target":target,"resources":ids.iter().map(|id|format!("thread:{id}")).chain(std::iter::once(target.to_owned())).collect::<Vec<_>>()}))?;
        let result = (|| {
            let mut assignments = HashMap::new();
            for id in ids {
                self.journal
                    .record(&op, "relinking", json!({"threadId":id}))?;
                self.assign(id, Some(target))?;
                assignments.insert(id.clone(), Some(target.to_owned()));
            }
            self.sync(&[project], &assignments, &[], &[])
        })();
        self.finish(&op, result)
    }
    pub fn merge(&mut self, sources: &[String], target: &str) -> Result<String, String> {
        self.guard()?;
        if sources.is_empty()
            || sources.iter().any(|id| id == target)
            || sources.iter().collect::<HashSet<_>>().len() != sources.len()
        {
            return Err("서로 다른 원본과 대상 프로젝트를 선택하세요.".into());
        }
        let before_target = self.project(target)?;
        let mut projects = Vec::new();
        let mut selected = Vec::new();
        let mut paths = roots(&before_target)?;
        let catalog = self.snapshot()?;
        crate::connections::require_connections(&catalog, sources)?;
        for id in sources {
            let p = self.project(id)?;
            for path in roots(&p)? {
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
            projects.push(p);
            selected.extend(
                all_threads(&catalog)
                    .into_iter()
                    .filter(|t| t["projectId"] == *id),
            );
        }
        let requested = selected
            .iter()
            .map(|t| t["id"].as_str().unwrap_or("").to_owned())
            .collect::<Vec<_>>();
        let all = all_threads(&catalog);
        let ids = crate::expand_thread_ids(&all, &requested)?;
        selected = all
            .into_iter()
            .filter(|t| {
                t["id"]
                    .as_str()
                    .is_some_and(|id| ids.iter().any(|selected| selected == id))
            })
            .collect();
        if !ids.is_empty() {
            self.selected_threads(&ids)?;
        }
        let mut after = before_target.clone();
        after["roots"] = json!(paths.iter().map(|p| json!({"path":p})).collect::<Vec<_>>());
        let keys: HashMap<_, _> = sources
            .iter()
            .map(|id| (id.clone(), uuid::Uuid::new_v4().to_string()))
            .collect();
        let op=self.begin(json!({"kind":"rust-merge","projects":projects,"targetBefore":before_target,"targetAfter":after,"threads":selected,"restoreKeys":keys,"resources":sources.iter().cloned().chain(std::iter::once(target.to_owned())).collect::<Vec<_>>()}))?;
        let result = (|| {
            self.update_project(&after)?;
            let mut assignments = HashMap::new();
            for id in ids {
                self.journal
                    .record(&op, "relinking", json!({"threadId":id}))?;
                self.assign(&id, Some(target))?;
                assignments.insert(id, Some(target.to_owned()));
            }
            for id in sources {
                if all_threads(&self.snapshot()?)
                    .iter()
                    .any(|t| t["projectId"] == *id)
                {
                    return Err("원본 프로젝트에 대화가 남아 있습니다.".into());
                }
                self.journal
                    .record(&op, "removing-project", json!({"projectId":id}))?;
                self.call("project/delete", json!({"projectId":id}))?;
            }
            self.sync(&[after], &assignments, sources, &[])
        })();
        self.finish(&op, result)
    }
    pub fn delete(
        &mut self,
        project_ids: &[String],
        thread_ids: &[String],
        permanent: bool,
        job: &Job,
    ) -> Result<String, String> {
        self.delete_checked(project_ids, thread_ids, permanent, job, false, &mut |_| {
            Ok(())
        })
    }
    pub(crate) fn delete_checked(
        &mut self,
        project_ids: &[String],
        thread_ids: &[String],
        permanent: bool,
        job: &Job,
        exact_selection: bool,
        before_each: &mut dyn FnMut(&mut Adapter) -> Result<(), String>,
    ) -> Result<String, String> {
        job.check()?;
        self.guard()?;
        let catalog = self.snapshot()?;
        let all = all_threads(&catalog);
        crate::connections::require_connections(&catalog, project_ids)?;
        let mut projects = Vec::new();
        let mut selected: HashSet<_> = thread_ids.iter().cloned().collect();
        for id in project_ids {
            projects.push(self.project(id)?);
            selected.extend(
                all.iter()
                    .filter(|t| t["projectId"] == *id)
                    .filter_map(|t| t["id"].as_str().map(str::to_owned)),
            );
        }
        if selected.is_empty() && projects.is_empty() {
            return Err("삭제할 항목을 선택하세요.".into());
        }
        let ids = crate::expand_thread_ids(&all, &selected.into_iter().collect::<Vec<_>>())?;
        if exact_selection
            && (ids.len() != thread_ids.len() || ids.iter().any(|id| !thread_ids.contains(id)))
        {
            return Err(
                "미리보기에서 승인하지 않은 대화 의존성이 생겼습니다. 원본을 유지합니다.".into(),
            );
        }
        let threads = if ids.is_empty() {
            Vec::new()
        } else {
            self.selected_threads(&ids)?
        };
        if !permanent
            && threads
                .iter()
                .any(|t| t["attachmentCount"].as_u64().unwrap_or(0) > 0)
        {
            return Err("첨부가 연결된 대화는 복구 가능 삭제를 지원하지 않습니다. 백업 의존성을 확인하거나 완전 삭제를 선택하세요.".into());
        }
        let keys: HashMap<_, _> = project_ids
            .iter()
            .map(|id| (id.clone(), uuid::Uuid::new_v4().to_string()))
            .collect();
        let recovery = self.recovery_root().join(uuid::Uuid::new_v4().to_string());
        // Resume leaves rollout handles open in the owned server. Release them
        // before acquiring stable read handles for recovery copies.
        self.server = None;
        let mut saved = Vec::new();
        if !permanent {
            ensure_plain(&recovery)?;
            fs::create_dir_all(&recovery).map_err(|e| e.to_string())?;
            job.phase_items("삭제 전 대화 복구 사본 준비", threads.len() as u64);
            for (i, t) in threads.iter().enumerate() {
                job.check()?;
                let id = t["id"].as_str().ok_or("대화 ID 오류")?;
                uuid::Uuid::parse_str(id).map_err(|_| "대화 ID 형식 오류")?;
                let source = Path::new(t["path"].as_str().ok_or("대화 원본이 없습니다.")?);
                let sha = digest(source, job)?;
                let path = recovery.join(format!("{id}.jsonl"));
                let entry = crate::Entry {
                    identity: Some(crate::identity(source)?),
                    path: id.into(),
                    size: fs::metadata(source).map_err(|e| e.to_string())?.len(),
                    sha256: sha.clone(),
                };
                crate::copy_file(source, &path, &entry, job, &mut 0, entry.size)?;
                saved.push(
                    json!({"id":id,"path":path,"sha256":sha,"runtimeRoots":runtime_roots(source)?}),
                );
                job.advance((i + 1) as u64, id, 0);
            }
        }
        let op=self.begin(json!({"kind":"rust-delete","projects":projects,"threads":threads,"permanent":permanent,"saved":saved,"restoreKeys":keys,"resources":project_ids.iter().cloned().chain(ids.iter().map(|id|format!("thread:{id}"))).collect::<Vec<_>>()}))?;
        let result = (|| {
            job.check()?;
            let mut cache = CatalogDeletion::lock(&self.store.home)?;
            let mut removed = Vec::new();
            job.phase_items("Codex 대화 삭제·목록 반영", ids.len() as u64);
            for (i, id) in ids.iter().enumerate() {
                job.check()?;
                before_each(self)?;
                let current = all_threads(&self.snapshot()?);
                if exact_selection {
                    // Earlier RPCs may cascade approved children, while another
                    // writer may add descendants after recovery copies or the
                    // callback. Revalidate the remaining original authorization;
                    // never add new dependencies to this deletion plan.
                    let authorized = ids
                        .iter()
                        .filter(|id| current.iter().any(|t| t["id"] == **id))
                        .cloned()
                        .collect::<Vec<_>>();
                    let expanded = crate::expand_thread_ids(&current, &authorized)?;
                    if expanded != authorized {
                        return Err("미리보기에서 승인하지 않은 대화 의존성이 생겼습니다. 원본을 유지합니다.".into());
                    }
                }
                if !current.iter().any(|t| t["id"] == *id) {
                    continue;
                }
                self.journal
                    .record(&op, "deleting", json!({"threadId":id}))?;
                let call = self.call("thread/delete", json!({"threadId":id}));
                let remaining: HashSet<_> = all_threads(&self.snapshot()?)
                    .iter()
                    .filter_map(|t| t["id"].as_str().map(str::to_owned))
                    .collect();
                let confirmed: Vec<_> = ids
                    .iter()
                    .filter(|tid| !remaining.contains(*tid))
                    .cloned()
                    .collect();
                cache.remove(&confirmed)?;
                removed = confirmed;
                if let Err(e) = call {
                    cache.commit()?;
                    self.sync(&[], &HashMap::new(), &[], &removed)?;
                    return Err(e);
                }
                job.advance((i + 1) as u64, id, 0);
            }
            cache.commit()?;
            if ids.iter().any(|id| !removed.contains(id)) && !ids.is_empty() {
                return Err("삭제 후 대화 목록을 확인할 수 없습니다.".into());
            }
            for id in project_ids {
                job.check()?;
                self.call("project/delete", json!({"projectId":id}))?;
            }
            self.sync(&[], &HashMap::new(), project_ids, &removed)?;
            Ok(())
        })();
        match result {
            Ok(()) => self.finish(&op, Ok(())),
            Err(error) => {
                let payload = self
                    .journal
                    .operation(&op)?
                    .ok_or("삭제 작업 기록이 없습니다.")?["payload"]
                    .clone();
                match self.reconcile_delete(&payload, job) {
                    Ok(report) if report["complete"] == true => {
                        self.journal.record(
                            &op,
                            "completed",
                            json!({"verifiedAfterError":true,"error":error,"report":report}),
                        )?;
                        Ok(op)
                    }
                    Ok(report) if permanent || report["noChanges"] == true => {
                        self.journal.record(
                            &op,
                            "cancelled",
                            json!({"finalized":true,"error":error,"report":report}),
                        )?;
                        Err(format!(
                            "{error}\n처리된 삭제 결과를 정리했습니다. 남은 항목은 유지합니다."
                        ))
                    }
                    Ok(report) => {
                        self.journal.record(
                            &op,
                            "deletion-reconciled",
                            json!({"report":report}),
                        )?;
                        self.finish(&op, Err(error))
                    }
                    Err(cleanup) => self.finish(
                        &op,
                        Err(format!("{error}\n삭제 목록 정리 확인 필요: {cleanup}")),
                    ),
                }
            }
        }
    }
}

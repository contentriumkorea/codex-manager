use codex_manager_core::*;
mod portable;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub struct Options {
    pub data: PathBuf,
    pub home: Option<PathBuf>,
    pub health: Option<PathBuf>,
    pub no_update: bool,
    pub test_root: Option<PathBuf>,
}
pub struct Service {
    pub options: Options,
    pub journal: Journal,
    settings: Mutex<Value>,
    jobs: Mutex<HashMap<String, Job>>,
    gate: crate::job_gate::JobGate,
    job_order: Mutex<VecDeque<String>>,
    pub _instance: fs::File,
    pub _legacy_instance: crate::instance::LegacyLock,
    pub updater: crate::updater::Updater,
    pub shell_status: Mutex<Result<(), String>>,
}
fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .ok_or_else(|| format!("입력 누락: {key}"))
}
fn strings(args: &Value, key: &str) -> Result<Vec<String>, String> {
    args[key]
        .as_array()
        .ok_or_else(|| format!("선택 누락: {key}"))?
        .iter()
        .map(|v| v.as_str().map(str::to_owned).ok_or("선택 값 오류".into()))
        .collect()
}
fn paths(args: &Value, key: &str) -> Result<Vec<PathBuf>, String> {
    strings(args, key).map(|v| v.into_iter().map(PathBuf::from).collect())
}
impl Service {
    pub fn new(options: Options) -> Result<Self, String> {
        absolute(&options.data)?;
        if let Some(root) = &options.test_root {
            absolute(root)?;
            if !options.data.starts_with(root) {
                return Err("시험 설정은 시험 폴더 안에 있어야 합니다.".into());
            }
        }
        fs::create_dir_all(&options.data).map_err(|e| e.to_string())?;
        let mut lock = fs::OpenOptions::new();
        lock.create(true).read(true).write(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            lock.share_mode(0);
        }
        let instance = lock
            .open(options.data.join("application.rust.lock"))
            .map_err(|_| "같은 설정을 사용하는 Codex Manager가 이미 실행 중입니다.".to_owned())?;
        let legacy = crate::instance::LegacyLock::acquire(&options.data)?;
        let path = options.data.join("settings.json");
        let settings = if path.is_file() {
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) if v.is_object() => v,
                _ => {
                    let invalid = options
                        .data
                        .join(format!("settings.invalid-{}.json", uuid::Uuid::new_v4()));
                    fs::copy(&path, invalid).map_err(|e| e.to_string())?;
                    json!({})
                }
            }
        } else {
            json!({})
        };
        let journal = Journal::open(&options.data.join("journal.sqlite"))?;
        Ok(Self {
            options,
            journal,
            settings: Mutex::new(settings),
            jobs: Mutex::new(HashMap::new()),
            gate: crate::job_gate::JobGate::default(),
            job_order: Mutex::new(VecDeque::new()),
            _instance: instance,
            _legacy_instance: legacy,
            updater: crate::updater::Updater::new(),
            shell_status: Mutex::new(Err("창 초기화 중".into())),
        })
    }
    fn home(&self) -> Result<PathBuf, String> {
        if let Some(home) = &self.options.home {
            return Ok(home.clone());
        }
        if let Some(home) = self.settings.lock().map_err(|_| "설정 잠금 오류")?["home"].as_str()
        {
            return Ok(home.into());
        }
        std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".codex")))
            .ok_or("Codex 저장소 위치를 찾을 수 없습니다.".into())
    }
    fn seeds(&self) -> Result<Vec<PathBuf>, String> {
        let settings = self.settings.lock().map_err(|_| "설정 잠금 오류")?;
        let mut seeds = settings["homes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        drop(settings);
        seeds.push(self.home()?);
        Ok(seeds)
    }
    fn checked_home(&self, home: &str) -> Result<Store, String> {
        let home = Path::new(home);
        absolute(home)?;
        if let Some(root) = &self.options.test_root {
            if !clean_path(home).starts_with(clean_path(root)) {
                return Err("시험 폴더 밖의 저장소에는 접근할 수 없습니다.".into());
            }
        }
        let seeds = self.seeds()?;
        if !seeds.iter().any(|p| {
            clean_path(p).to_string_lossy().to_lowercase()
                == clean_path(home).to_string_lossy().to_lowercase()
        }) {
            return Err("설정에서 저장소를 먼저 연결하세요.".into());
        }
        let store = resolve_store(home)?;
        if let Some(root) = &self.options.test_root {
            if !clean_path(&store.sqlite_home).starts_with(clean_path(root)) {
                return Err("시험 데이터베이스가 시험 범위 밖에 있습니다.".into());
            }
        }
        Ok(store)
    }
    fn change_settings(
        &self,
        change: impl FnOnce(&mut Value) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut settings = self.settings.lock().map_err(|_| "설정 잠금 오류")?;
        let mut merged = settings.clone();
        change(&mut merged)?;
        let temporary = self
            .options
            .data
            .join(format!(".settings-{}.partial", uuid::Uuid::new_v4()));
        write_json_new(&temporary, &merged)?;
        fs::rename(temporary, self.options.data.join("settings.json"))
            .map_err(|e| e.to_string())?;
        *settings = merged;
        Ok(())
    }
    fn save(&self, values: Value) -> Result<(), String> {
        self.change_settings(|settings| {
            for (key, value) in values.as_object().ok_or("설정 형식 오류")? {
                settings[key] = value.clone();
            }
            Ok(())
        })
    }
    fn add_homes(&self, new: &[PathBuf]) -> Result<(), String> {
        self.change_settings(|settings| {
            let mut homes = settings["homes"].as_array().cloned().unwrap_or_default();
            for home in new {
                let value = json!(home);
                if !homes.contains(&value) {
                    homes.push(value);
                }
            }
            settings["homes"] = json!(homes);
            Ok(())
        })
    }
    pub fn bootstrap(&self) -> Result<Value, String> {
        self.bootstrap_pane(None)
    }
    fn pane(pane: Option<&str>) -> Result<Option<&str>, String> {
        if pane.is_some_and(|p| !["left", "right"].contains(&p)) {
            return Err("탐색기 선택 오류".into());
        }
        Ok(pane)
    }
    fn pane_home(&self, pane: Option<&str>) -> Result<PathBuf, String> {
        if let Some(pane) = Self::pane(pane)? {
            if let Some(home) =
                self.settings.lock().map_err(|_| "설정 잠금 오류")?["panes"][pane]["home"].as_str()
            {
                return Ok(home.into());
            }
        }
        self.home()
    }
    fn size_cache(&self, home: &Path) -> PathBuf {
        use sha2::{Digest, Sha256};
        let key = clean_path(home).to_string_lossy().to_lowercase();
        self.options.data.join(format!(
            "folder-sizes-{:x}.json",
            Sha256::digest(key.as_bytes())
        ))
    }
    pub fn bootstrap_pane(&self, pane: Option<&str>) -> Result<Value, String> {
        let home = self.pane_home(pane)?;
        let seeds = self.seeds()?;
        let mut stores = Vec::new();
        for home in &seeds {
            if stores.iter().any(|s: &Value| s["home"] == json!(home)) {
                continue;
            }
            let info = match resolve_store(home).and_then(|s| read_store(&s)) {
                Ok(c) => {
                    json!({"home":home,"available":true,"projects":c.projects.len(),"threads":all_threads(&c).len()})
                }
                Err(e) => json!({"home":home,"available":false,"error":e}),
            };
            stores.push(info);
        }
        let cache_path = self.size_cache(&home);
        let sizes = fs::read(&cache_path)
            .ok()
            .and_then(|s| serde_json::from_slice::<Value>(&s).ok())
            .unwrap_or(json!({}));
        Ok(
            json!({"version":VERSION,"home":home,"pane":pane,"stores":stores,"settings":self.settings.lock().map_err(|_|"설정 잠금 오류")?.clone(),"sizes":sizes,"testMode":self.options.test_root.is_some(),"noUpdate":self.options.no_update}),
        )
    }
    pub fn catalog(&self, home: &str) -> Result<Value, String> {
        let store = self.checked_home(home)?;
        read_store(&store).map(catalog_json)
    }
    pub fn select_home(&self, home: &str, pane: Option<&str>) -> Result<(), String> {
        Self::pane(pane)?;
        let path = PathBuf::from(home);
        absolute(&path)?;
        if let Some(root) = &self.options.test_root {
            if !path.starts_with(root) {
                return Err("시험 폴더 밖의 저장소를 선택할 수 없습니다.".into());
            }
        }
        let store = resolve_store(&path)?;
        if let Some(root) = &self.options.test_root {
            if !clean_path(&store.sqlite_home).starts_with(clean_path(root)) {
                return Err("시험 데이터베이스가 시험 범위 밖에 있습니다.".into());
            }
        }
        read_store(&store)?;
        self.change_settings(|settings| {
            let mut homes = settings["homes"].as_array().cloned().unwrap_or_default();
            let value = json!(path);
            if !homes.contains(&value) {
                homes.push(value.clone());
            }
            settings["homes"] = json!(homes);
            if let Some(pane) = pane {
                if !settings["panes"].is_object() {
                    settings["panes"] = json!({});
                }
                if !settings["panes"][pane].is_object() {
                    settings["panes"][pane] = json!({});
                }
                settings["panes"][pane]["home"] = value;
            } else {
                settings["home"] = value;
            }
            Ok(())
        })
    }
    pub fn files(
        &self,
        home: &str,
        project_ids: &[String],
        directory: &str,
    ) -> Result<Vec<Value>, String> {
        let catalog = read_store(&self.checked_home(home)?)?;
        let allowed = catalog
            .projects
            .iter()
            .filter(|g| {
                g.project["id"]
                    .as_str()
                    .is_some_and(|id| project_ids.iter().any(|p| p == id))
            })
            .map(|g| roots(&g.project))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        list_files(Path::new(directory), &allowed)
    }
    pub fn transcript(&self, home: &str, thread_id: &str) -> Result<Value, String> {
        let catalog = read_store(&self.checked_home(home)?)?;
        let thread = all_threads(&catalog)
            .into_iter()
            .find(|t| t["id"] == thread_id)
            .ok_or("대화가 목록에 없습니다.")?;
        let path = Path::new(
            thread["path"]
                .as_str()
                .ok_or("대화 기록 경로가 없습니다.")?,
        );
        ensure_plain(path)?;
        let mut messages = VecDeque::new();
        let mut total = 0;
        let mut reader = RolloutReader::open_live(path, Some(thread_id), &Job::new())?;
        while let Some(record) = reader.next_record()? {
            let item = record.value;
            let p = &item["payload"];
            if item["type"] != "response_item"
                || p["type"] != "message"
                || !["user", "assistant"].contains(&p["role"].as_str().unwrap_or(""))
            {
                continue;
            }
            let text = p["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                continue;
            }
            total += 1;
            messages.push_back(json!({"role":p["role"],"text":text}));
            while messages.len() > 200 {
                messages.pop_front();
            }
        }
        Ok(json!({"thread":thread,"messages":messages,"total":total}))
    }
    pub fn history(&self) -> Result<Vec<Value>, String> {
        let mut history = self.journal.history()?;
        for item in &mut history {
            let id = item["id"].as_str().ok_or("작업 ID 오류")?;
            let events = self.journal.events(id)?;
            item["recovered"] = json!(events.iter().any(|e| e["payload"]["recovered"] == true));
            if let Some(last) = events.last() {
                item["lastEvent"] = last.clone();
            }
        }
        Ok(history)
    }
    fn adapter(&self, home: &str) -> Result<Adapter, String> {
        let store = self.checked_home(home)?;
        let binary = if let Some(root) = &self.options.test_root {
            let path = match std::env::var_os("CODEX_TEST_BINARY") {
                Some(path) => PathBuf::from(path),
                None => {
                    find_codex().unwrap_or_else(|_| self.options.data.join("missing-codex.exe"))
                }
            };
            if !root.is_dir() {
                return Err("시험 폴더가 없습니다.".into());
            }
            let catalog = read_store(&store)?;
            for group in &catalog.projects {
                for path in roots(&group.project)? {
                    if !clean_path(&path).starts_with(clean_path(root)) {
                        return Err("시험 프로젝트의 연결 폴더가 시험 범위 밖에 있습니다.".into());
                    }
                }
            }
            for thread in all_threads(&catalog) {
                if let Some(path) = thread["path"].as_str() {
                    if !clean_path(Path::new(path)).starts_with(clean_path(root)) {
                        return Err("시험 대화 원본이 시험 범위 밖에 있습니다.".into());
                    }
                }
            }
            path
        } else {
            let configured = self.settings.lock().map_err(|_| "설정 잠금 오류")?["codex_binary"]
                .as_str()
                .map(PathBuf::from);
            match configured {
                Some(path) => path,
                None => {
                    find_codex().unwrap_or_else(|_| self.options.data.join("missing-codex.exe"))
                }
            }
        };
        if self.options.test_root.is_some() {
            Adapter::isolated(store, binary, self.journal.clone())
        } else {
            Adapter::open(store, binary, self.journal.clone())
        }
    }
    pub fn job(&self, id: &str) -> Result<Value, String> {
        self.jobs
            .lock()
            .map_err(|_| "작업 잠금 오류")?
            .get(id)
            .map(Job::snapshot)
            .ok_or("작업을 찾을 수 없습니다.".into())
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        self.jobs
            .lock()
            .map_err(|_| "작업 잠금 오류")?
            .get(id)
            .ok_or("작업을 찾을 수 없습니다.")?
            .cancel();
        Ok(())
    }
    pub fn start(self: &Arc<Self>, action: String, args: Value) -> Result<String, String> {
        self.start_kind(action, args, false)
    }
    fn start_kind(
        self: &Arc<Self>,
        action: String,
        args: Value,
        automatic: bool,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let readonly = [
            "connection_plan",
            "scan",
            "sizes",
            "cross_store_conclude_plan",
            "cross_store_plan",
            "discover_backups",
            "verify_backup",
            "update_check",
            "update_prepare",
            "portable_status",
            "portable_discover",
            "portable_inspect",
            "portable_plan",
            "portable_transfer_plan",
            "folder_list",
            "layout_preferences",
        ]
        .contains(&action.as_str());
        let job = Job::new();
        if !readonly {
            self.gate.begin(&id, &job, automatic)?;
        }
        self.jobs
            .lock()
            .map_err(|_| "작업 잠금 오류")?
            .insert(id.clone(), job.clone());
        {
            let mut order = self.job_order.lock().map_err(|_| "작업 잠금 오류")?;
            order.push_back(id.clone());
            let mut jobs = self.jobs.lock().map_err(|_| "작업 잠금 오류")?;
            let excess = order.len().saturating_sub(128);
            for _ in 0..excess {
                if let Some(old) = order.pop_front() {
                    if jobs
                        .get(&old)
                        .is_some_and(|j| j.snapshot()["state"] == "running")
                    {
                        order.push_back(old);
                    } else {
                        jobs.remove(&old);
                    }
                }
            }
        }
        let worker_id = id.clone();
        let service = Arc::clone(self);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                service.run(&action, &args, &job)
            }))
            .unwrap_or_else(|_| {
                Err("작업을 완료하지 못했습니다. 원본을 확인하고 작업 내역에서 복구하세요.".into())
            });
            job.finish(result);
            if !readonly {
                service.gate.finish(&worker_id);
            }
        });
        Ok(id)
    }
    fn run(&self, action: &str, args: &Value, job: &Job) -> Result<Value, String> {
        if let Some(root) = &self.options.test_root {
            for key in ["path", "destination", "bundle", "anchor"] {
                if let Some(path) = args[key].as_str() {
                    if !clean_path(Path::new(path)).starts_with(clean_path(root)) {
                        return Err("시험 작업의 경로가 시험 범위 밖에 있습니다.".into());
                    }
                }
            }
            for key in ["roots", "destinations", "sources"] {
                if let Some(values) = args[key].as_array() {
                    for value in values {
                        if let Some(path) = value.as_str() {
                            if !clean_path(Path::new(path)).starts_with(clean_path(root)) {
                                return Err("시험 작업의 경로가 시험 범위 밖에 있습니다.".into());
                            }
                        }
                    }
                }
            }
        }
        match action {
            "layout_preferences" => {
                self.save(json!({"split_view":args["enabled"].as_bool().unwrap_or(false)}))?;
                return Ok(json!({}));
            }
            "folder_list" => return self.destination_folders(str_arg(args, "path")?),
            "portable_discover" => {
                return portable_discover(Path::new(str_arg(args, "path")?), 8, 5000, job)
            }
            "portable_inspect" => {
                return portable_inspect(
                    Path::new(str_arg(args, "anchor")?),
                    str_arg(args, "namespace")?,
                    job,
                )
            }
            "pane_preferences" => {
                let pane = str_arg(args, "pane")?;
                Self::pane(Some(pane))?;
                let mut result = Value::Null;
                self.change_settings(|settings| {
                    if !settings["panes"].is_object() {
                        settings["panes"] = json!({});
                    }
                    if !settings["panes"][pane].is_object() {
                        settings["panes"][pane] = json!({});
                    }
                    if let Some(destination) = args.get("destination") {
                        if let Some(path) = destination.as_str().filter(|p| !p.is_empty()) {
                            absolute(Path::new(path))?;
                            settings["panes"][pane]["destination"] = json!(path);
                        } else if destination.is_null() || destination == "" {
                            settings["panes"][pane]["destination"] = Value::Null;
                        } else {
                            return Err("대상 폴더 설정 오류".into());
                        }
                    }
                    result = settings["panes"][pane].clone();
                    Ok(())
                })?;
                return Ok(result);
            }
            "accept_terms" => {
                self.save(json!({"webview2_terms":"2026-11-02"}))?;
                return Ok(json!({"accepted":true}));
            }
            "preferences" => {
                if let Some(binary) = args["codex_binary"].as_str().filter(|s| !s.is_empty()) {
                    absolute(Path::new(binary))?;
                }
                self.save(json!({"startup_update_check":args["startup_update_check"].as_bool().unwrap_or(true),"codex_binary":args["codex_binary"].as_str().filter(|s|!s.is_empty())}))?;
                return Ok(json!({}));
            }
            "update_check" => return self.updater.check(job),
            "update_prepare" => return self.updater.prepare(&self.options.data, job),
            "scan" => {
                let mut search = paths(args, "roots")?;
                if search.is_empty() {
                    if let Some(root) = &self.options.test_root {
                        search.push(root.clone());
                    } else {
                        if let Some(user) = std::env::var_os("USERPROFILE") {
                            search.push(user.into());
                        }
                        for drive in ["C:/", "D:/", "E:/", "F:/", "G:/"] {
                            let path = PathBuf::from(drive);
                            if path.is_dir() {
                                search.push(path);
                            }
                        }
                    }
                }
                if let Some(root) = &self.options.test_root {
                    if search.iter().any(|p| !p.starts_with(root)) {
                        return Err("시험 폴더 밖을 검색할 수 없습니다.".into());
                    }
                }
                let seeds = self.seeds()?;
                let stores = discover_stores_with_job(&seeds, &search, job)?;
                job.check()?;
                self.add_homes(&stores.iter().map(|s| s.home.clone()).collect::<Vec<_>>())?;
                return self.bootstrap();
            }
            "discover_backups" => {
                return discover_bundles(Path::new(str_arg(args, "path")?), job).map(|v| json!(v))
            }
            "verify_backup" => return load_verified_bundle(Path::new(str_arg(args, "path")?), job),
            "sizes" => {
                let catalog = read_store(&self.checked_home(str_arg(args, "home")?)?)?;
                let mut sizes = json!({});
                for group in catalog.projects {
                    job.check()?;
                    let paths = roots(&group.project)?;
                    let mut total = 0u64;
                    let mut error = None;
                    for root in paths {
                        match folder_size(&root, job) {
                            Ok(n) => total = total.saturating_add(n),
                            Err(e) => {
                                error = Some(e);
                                break;
                            }
                        }
                    }
                    sizes[group.project["id"].as_str().ok_or("프로젝트 ID 누락")?] =
                        json!({"bytes":total,"error":error,"cachedAt":chrono_now()});
                }
                let cache = self.size_cache(Path::new(str_arg(args, "home")?));
                let temporary = self
                    .options
                    .data
                    .join(format!(".sizes-{}.partial", uuid::Uuid::new_v4()));
                write_json_new(&temporary, &sizes)?;
                fs::rename(temporary, cache).map_err(|e| e.to_string())?;
                return Ok(sizes);
            }
            _ => {}
        }
        if self.options.test_root.is_none()
            && self.settings.lock().map_err(|_| "설정 잠금 오류")?["webview2_terms"] != "2026-11-02"
        {
            return Err("설정에서 포함 구성요소 이용약관을 확인하고 동의한 뒤 사용하세요.".into());
        }
        let home = str_arg(args, "home")?;
        let mut adapter = self.adapter(home)?;
        if action.starts_with("portable_") {
            return self.portable_action(action, args, &mut adapter, job);
        }
        if [
            "cross_store_conclude_plan",
            "cross_store_plan",
            "cross_store_transfer",
            "cross_store_resume",
            "cross_store_conclude",
        ]
        .contains(&action)
        {
            let target_home = if [
                "cross_store_resume",
                "cross_store_conclude_plan",
                "cross_store_conclude",
            ]
            .contains(&action)
            {
                let record = self
                    .journal
                    .operation(str_arg(args, "id")?)?
                    .ok_or("전송 기록이 없습니다.")?;
                if record["payload"]["sourceHome"] != home {
                    return Err("작업을 시작한 저장소를 선택하세요.".into());
                }
                str_arg(&record["payload"], "targetHome")?.to_owned()
            } else {
                str_arg(args, "targetHome")?.to_owned()
            };
            let mut target = self.adapter(&target_home)?;
            if let Some(root) = &self.options.test_root {
                let checked = if [
                    "cross_store_resume",
                    "cross_store_conclude_plan",
                    "cross_store_conclude",
                ]
                .contains(&action)
                {
                    self.journal
                        .operation(str_arg(args, "id")?)?
                        .ok_or("전송 기록 누락")?["payload"]["request"]
                        .clone()
                } else {
                    args.clone()
                };
                if let Some(destination) = checked["destination"].as_str() {
                    absolute(Path::new(destination))?;
                    if !clean_path(Path::new(destination)).starts_with(clean_path(root)) {
                        return Err("시험 대상이 시험 범위 밖에 있습니다.".into());
                    }
                }
                if checked["kind"] == "files" {
                    for path in strings(&checked, "ids")? {
                        absolute(Path::new(&path))?;
                        if !clean_path(Path::new(&path)).starts_with(clean_path(root)) {
                            return Err("시험 파일이 시험 범위 밖에 있습니다.".into());
                        }
                    }
                }
            }
            return match action {
                "cross_store_conclude_plan" => {
                    cross_store_conclude_plan(&adapter, &target, str_arg(args, "id")?, job)
                }
                "cross_store_conclude" => cross_store_conclude(
                    &mut adapter,
                    &mut target,
                    str_arg(args, "id")?,
                    &args["expectedRevision"],
                    job,
                ),
                "cross_store_plan" => cross_store_plan(&adapter, &target, args, job),
                "cross_store_transfer" => {
                    cross_store_transfer(&mut adapter, &mut target, args, job)
                }
                _ => cross_store_resume(&mut adapter, &mut target, str_arg(args, "id")?, job),
            };
        }
        if let Some(expected) = args["expectedRevision"].as_str() {
            if catalog_json(adapter.snapshot()?)["revision"] != expected {
                return Err(
                    "목록이 다른 곳에서 변경됐습니다. 새로 고침한 뒤 다시 선택하세요.".into(),
                );
            }
        }
        if action == "connection_plan" {
            return adapter.connection_plan(&strings(args, "projects")?);
        }
        if action == "confirm_connections" {
            return adapter.confirm_connections(&args["plan"], job);
        }
        let mut mapped_args = args.clone();
        if [
            "rename_project",
            "set_roots",
            "rename_threads",
            "move_threads",
            "merge",
            "delete",
            "transfer",
            "import",
        ]
        .contains(&action)
        {
            let selected = ["id", "target"]
                .iter()
                .filter_map(|key| args[*key].as_str().map(str::to_owned))
                .chain(["ids", "projects", "threads"].iter().flat_map(|key| {
                    args[*key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                }))
                .collect::<Vec<_>>();
            let aliases = adapter.materialize_legacy(&selected, job)?;
            for key in ["id", "target"] {
                if let Some(replacement) = mapped_args[key].as_str().and_then(|id| aliases.get(id))
                {
                    mapped_args[key] = json!(replacement);
                }
            }
            for key in ["ids", "projects"] {
                if let Some(ids) = mapped_args[key].as_array_mut() {
                    for id in ids {
                        if let Some(replacement) = id.as_str().and_then(|id| aliases.get(id)) {
                            *id = json!(replacement);
                        }
                    }
                }
            }
        }
        let args = &mapped_args;
        let op = match action {
            "resume_legacy" => {
                return adapter
                    .resume_legacy(job)
                    .map(|done| json!({"reconciled":done}))
            }
            "create" => {
                let project = adapter.create(str_arg(args, "name")?, &paths(args, "roots")?)?;
                return Ok(json!({"project":project}));
            }
            "rename_project" => {
                adapter.rename_project(str_arg(args, "id")?, str_arg(args, "name")?)?
            }
            "set_roots" => adapter.set_roots(str_arg(args, "id")?, &paths(args, "roots")?)?,
            "rename_threads" => {
                adapter.rename_threads(&strings(args, "ids")?, &strings(args, "names")?)?
            }
            "move_threads" => {
                adapter.move_threads(&strings(args, "ids")?, str_arg(args, "target")?)?
            }
            "merge" => adapter.merge(&strings(args, "ids")?, str_arg(args, "target")?)?,
            "delete" => adapter.delete(
                &strings(args, "projects")?,
                &strings(args, "threads")?,
                args["permanent"].as_bool().unwrap_or(false),
                job,
            )?,
            "backup" => {
                let ids = strings(args, "ids")?;
                let parent = Path::new(str_arg(args, "destination")?);
                absolute(parent)?;
                let catalog = adapter.snapshot()?;
                let mut results = Vec::new();
                for id in ids {
                    job.check()?;
                    let project = &catalog
                        .projects
                        .iter()
                        .find(|g| g.project["id"] == id)
                        .ok_or("프로젝트가 바뀌었습니다.")?
                        .project;
                    let name = project["name"].as_str().unwrap_or("프로젝트");
                    let safe = name
                        .chars()
                        .map(|c| {
                            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                                '_'
                            } else {
                                c
                            }
                        })
                        .collect::<String>();
                    let destination = parent.join(format!(
                        "{} {}",
                        safe.trim_end_matches(['.', ' ']),
                        uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
                    ));
                    let op = adapter.export_backup(&id, &destination, job)?;
                    let manifest: Value = serde_json::from_slice(
                        &fs::read(destination.join("manifest.json")).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    results.push(json!({"operationId":op,"path":destination,"dependencies":manifest["dependencies"]}));
                }
                let mut backups = self.settings.lock().map_err(|_| "설정 잠금 오류")?["backups"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let path = json!(parent);
                if !backups.contains(&path) {
                    backups.push(path);
                }
                self.save(json!({"backups":backups}))?;
                return Ok(json!({"backups":results}));
            }
            "import" => adapter.import_backup(
                Path::new(str_arg(args, "bundle")?),
                &paths(args, "destinations")?,
                args["target"].as_str(),
                args["skipExisting"].as_bool().unwrap_or(false),
                args["resume"].as_str(),
                job,
            )?,
            "transfer" => adapter.transfer_project(
                str_arg(args, "id")?,
                &paths(args, "destinations")?,
                args["clean"].as_bool().unwrap_or(true),
                job,
            )?,
            "files" => adapter.edit_files(
                str_arg(args, "kind")?,
                &paths(args, "sources")?,
                Path::new(str_arg(args, "destination")?),
                args["name"].as_str(),
                job,
            )?,
            "files_to_folder" => adapter.edit_files_to_folder(
                str_arg(args, "kind")?,
                &paths(args, "sources")?,
                Path::new(str_arg(args, "destination")?),
                job,
            )?,
            "mkdir" => adapter.mkdir(
                Path::new(str_arg(args, "destination")?),
                str_arg(args, "name")?,
                job,
            )?,
            "undo" => {
                adapter.undo_with_job(str_arg(args, "id")?, job)?;
                return Ok(json!({"operationId":args["id"]}));
            }
            "finalize_delete" => return adapter.finalize_delete(str_arg(args, "id")?, job),
            "keep_current" => return adapter.keep_current(str_arg(args, "id")?, job),
            _ => return Err("지원하지 않는 작업입니다.".into()),
        };
        Ok(json!({"operationId":op}))
    }
    pub fn apply_update(&self) -> Result<(), String> {
        self.gate.stop()?;
        for job in self.jobs.lock().map_err(|_| "작업 잠금 오류")?.values() {
            if job.snapshot()["state"] == "running" {
                job.cancel();
            }
        }
        let result = self.updater.apply(&self.options.data);
        if result.is_err() {
            self.gate.resume();
        }
        result
    }
    pub fn can_close(&self) -> bool {
        self.gate.stop().is_ok()
    }
}
fn chrono_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn folder_size(root: &Path, job: &Job) -> Result<u64, String> {
    absolute(root)?;
    let mut stack = vec![root.to_owned()];
    let mut total = 0u64;
    job.phase("폴더 용량 자동 계산", 0);
    while let Some(dir) = stack.pop() {
        job.check()?;
        ensure_plain(&dir)?;
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            ensure_plain(&path)?;
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                total = total.checked_add(meta.len()).ok_or("용량 합계 초과")?;
            }
            job.advance(total, &dir.to_string_lossy(), 0);
        }
    }
    Ok(total)
}

#[cfg(test)]
mod pane_tests {
    use super::*;
    #[test]
    fn safe_conclusion_dispatch_pins_recorded_homes_and_preview_is_readonly() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(format!("conclusion-service-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let target = root.join("target");
        for path in [&source, &target] {
            fs::create_dir_all(path).unwrap();
        }
        let service = Service::new(Options {
            data: root.join("manager"),
            home: Some(source.clone()),
            health: None,
            no_update: true,
            test_root: Some(root.clone()),
        })
        .unwrap();
        service
            .add_homes(&[source.clone(), target.clone()])
            .unwrap();
        let id=service.journal.begin(&json!({"kind":"rust-cross-store","home":source,"sourceHome":source,"targetHome":target,"request":{"kind":"projects","destination":root.join("output")},"plan":{"sourceSqliteHome":source,"targetSqliteHome":target,"roots":[],"threads":[],"stagingBase":root.join("staging")},"resources":[format!("home:{}",source.display()),format!("home:{}",target.display())]})).unwrap();
        let before = digest(&service.journal.path, &Job::new()).unwrap();
        let args = json!({"home":source,"targetHome":root.join("forged"),"id":id});
        let preview = service
            .run("cross_store_conclude_plan", &args, &Job::new())
            .unwrap();
        assert_eq!(preview["targetHome"], json!(target));
        assert_eq!(before, digest(&service.journal.path, &Job::new()).unwrap());
        assert!(!root.join("staging").exists());
        for child in preview["childJournals"].as_array().unwrap() {
            assert!(!Path::new(child["path"].as_str().unwrap()).exists());
        }
        assert!(service
            .run(
                "cross_store_conclude_plan",
                &json!({"home":target,"id":id}),
                &Job::new()
            )
            .unwrap_err()
            .contains("시작한 저장소"));
        let result = json!({"safeConclusion":true,"transferCompleted":false,"retained":preview});
        service
            .journal
            .record(&id, "cancelled", result.clone())
            .unwrap();
        assert_eq!(
            service
                .run("cross_store_conclude", &args, &Job::new())
                .unwrap(),
            result
        );
    }
    #[test]
    fn concurrent_pane_updates_preserve_homes_preferences_and_size_isolation() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(format!("panes-{}", uuid::Uuid::new_v4()));
        let default = root.join("default");
        let left = root.join("left");
        let right = root.join("right");
        for p in [&default, &left, &right] {
            fs::create_dir_all(p).unwrap();
        }
        let service = Arc::new(
            Service::new(Options {
                data: root.join("manager"),
                home: Some(default.clone()),
                health: None,
                no_update: true,
                test_root: Some(root.clone()),
            })
            .unwrap(),
        );
        std::thread::scope(|scope| {
            for (pane, home) in [("left", left.clone()), ("right", right.clone())] {
                let service = Arc::clone(&service);
                scope.spawn(move || {
                    for _ in 0..8 {
                        service
                            .select_home(home.to_str().unwrap(), Some(pane))
                            .unwrap();
                        service
                            .run(
                                "pane_preferences",
                                &json!({"pane":pane,"destination":home.join("output")}),
                                &Job::new(),
                            )
                            .unwrap();
                    }
                });
            }
        });
        let settings: Value =
            serde_json::from_slice(&fs::read(service.options.data.join("settings.json")).unwrap())
                .unwrap();
        for (pane, home) in [("left", &left), ("right", &right)] {
            assert_eq!(settings["panes"][pane]["home"], json!(home));
            assert_eq!(
                settings["panes"][pane]["destination"],
                json!(home.join("output"))
            );
            assert!(service.checked_home(home.to_str().unwrap()).is_ok());
            assert_eq!(
                service.bootstrap_pane(Some(pane)).unwrap()["home"],
                json!(home)
            );
        }
        assert_eq!(service.bootstrap().unwrap()["home"], json!(default));
        assert_ne!(service.size_cache(&left), service.size_cache(&right));
        assert!(service
            .select_home(left.to_str().unwrap(), Some("third"))
            .is_err());
    }
}

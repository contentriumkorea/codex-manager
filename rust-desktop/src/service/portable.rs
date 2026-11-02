use super::*;

fn registration_key(home: &str, project: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(
            format!(
                "{}\n{project}",
                clean_path(Path::new(home)).to_string_lossy().to_lowercase()
            )
            .as_bytes()
        )
    )
}
pub(super) fn should_capture(previous: &Value, fingerprint: &str, force: bool) -> bool {
    force || previous["fingerprint"].as_str() != Some(fingerprint)
}
impl Service {
    pub fn open_file(&self, home: &str, projects: &[String], path: &str) -> Result<(), String> {
        let path = Path::new(path);
        absolute(path)?;
        ensure_plain(path)?;
        if !path.is_file() {
            return Err("일반 파일을 선택하세요.".into());
        }
        if path.components().any(|c| {
            c.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(".codex-manager")
        }) {
            return Err("대화 보관본은 프로젝트 관리에서 여세요.".into());
        }
        let parent = path.parent().ok_or("파일 경로 오류")?;
        if !self
            .files(home, projects, &parent.to_string_lossy())?
            .iter()
            .any(|f| f["path"] == json!(path) && f["linked"] == false)
        {
            return Err("선택한 프로젝트의 일반 파일만 열 수 있습니다.".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "shell32")]
            extern "system" {
                fn ShellExecuteW(
                    hwnd: *mut std::ffi::c_void,
                    operation: *const u16,
                    file: *const u16,
                    parameters: *const u16,
                    directory: *const u16,
                    show: i32,
                ) -> isize;
            }
            let operation = "open\0".encode_utf16().collect::<Vec<_>>();
            let file = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let result = unsafe {
                ShellExecuteW(
                    std::ptr::null_mut(),
                    operation.as_ptr(),
                    file.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                )
            };
            if result <= 32 {
                return Err(format!(
                    "기본 앱으로 파일을 열지 못했습니다. Windows 코드 {result}"
                ));
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            Err("Windows 기본 앱 열기만 지원합니다.".into())
        }
    }
    pub fn start_automatic_loop(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            loop {
                let Some(service) = weak.upgrade() else {
                    break;
                };
                let registrations = service
                    .settings
                    .lock()
                    .ok()
                    .and_then(|s| s["portableAutos"].as_object().cloned())
                    .unwrap_or_default();
                for entry in registrations.values().filter(|e| e["enabled"] == true) {
                    // One automatic writer only. Foreground acquisition cancels it and waits.
                    if let Ok(id) = service.start_kind("portable_tick".into(), entry.clone(), true)
                    {
                        while service.job(&id).is_ok_and(|j| j["state"] == "running") {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                    }
                }
                drop(service);
                for _ in 0..60 {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    if weak.strong_count() == 0 {
                        return;
                    }
                }
            }
        });
    }
    fn registration(&self, key: &str) -> Result<Value, String> {
        Ok(self.settings.lock().map_err(|_| "설정 잠금 오류")?["portableAutos"][key].clone())
    }
    fn save_registration(&self, key: &str, value: Value) -> Result<(), String> {
        self.change_settings(|s| {
            if !s["portableAutos"].is_object() {
                s["portableAutos"] = json!({});
            }
            s["portableAutos"][key] = value;
            Ok(())
        })
    }
    pub(super) fn portable_action(
        &self,
        action: &str,
        args: &Value,
        adapter: &mut Adapter,
        job: &Job,
    ) -> Result<Value, String> {
        if ["portable_transfer_plan", "portable_transfer"].contains(&action) {
            return self.portable_transfer(action, args, adapter, job);
        }
        let anchor = Path::new(str_arg(args, "anchor")?);
        absolute(anchor)?;
        match action {
            "portable_plan" => {
                return adapter.portable_restore_plan(
                    anchor,
                    str_arg(args, "namespace")?,
                    args["target"].as_str(),
                    job,
                )
            }
            "portable_restore" => {
                return adapter.portable_restore(
                    anchor,
                    str_arg(args, "namespace")?,
                    args["target"].as_str(),
                    args["resume"].as_str(),
                    job,
                )
            }
            _ => {}
        }
        let home = str_arg(args, "home")?;
        let project = str_arg(args, "project")?;
        let key = registration_key(home, project);
        let mut entry = self.registration(&key)?;
        if action == "portable_status" {
            let status = adapter
                .portable_namespace(project, anchor)
                .and_then(|ns| portable_status(anchor, &ns));
            return Ok(
                json!({"registration":entry,"snapshot":status.as_ref().ok(),"statusError":status.err()}),
            );
        }
        if action == "portable_configure" {
            if args["enabled"] != true {
                if entry.is_object() {
                    entry["enabled"] = json!(false);
                    self.save_registration(&key, entry.clone())?;
                }
                return Ok(entry);
            }
            adapter.portable_namespace(project, anchor)?; // Existing current root, never recreate stale anchors.
            entry = json!({"home":home,"project":project,"anchor":anchor,"enabled":true,"lastSuccessfulSave":entry["lastSuccessfulSave"],"error":entry["error"]});
            self.save_registration(&key, entry.clone())?;
        }
        if !["portable_configure", "portable_tick", "portable_save"].contains(&action) {
            return Err("알 수 없는 폴더 보관 작업".into());
        }
        if action == "portable_tick"
            && (entry["enabled"] != true || entry["anchor"] != args["anchor"])
        {
            return Ok(json!({"skipped":true}));
        }
        if !entry.is_object() {
            entry = json!({"home":home,"project":project,"anchor":anchor,"enabled":false});
        }
        if entry["anchor"] != json!(anchor) {
            entry["enabled"] = json!(false);
            entry["anchor"] = json!(anchor);
        }
        let attempt = (|| {
            if let Err(error) = adapter.portable_namespace(project, anchor) {
                entry["fingerprint"] = Value::Null;
                if action == "portable_tick" {
                    entry["enabled"] = json!(false);
                }
                return Err(error);
            }
            let fingerprint = match crate::portable_fingerprint::fingerprint(
                &adapter.snapshot()?,
                project,
                anchor,
                job,
            ) {
                Ok(value) => value,
                Err(error) => {
                    entry["fingerprint"] = Value::Null;
                    return Err(error);
                }
            };
            if !should_capture(&entry, &fingerprint, action != "portable_tick") {
                return Ok(json!({"skipped":true,"integrityVerified":false}));
            }
            // Save even a failed attempt's stamp: unchanged corruption isn't decoded every minute.
            entry["fingerprint"] = json!(fingerprint);
            entry["lastAttempt"] = json!(chrono_now());
            let result = adapter.portable_snapshot(project, anchor, job)?;
            entry["lastSuccessfulSave"] = result["lastSuccessfulSave"].clone();
            entry["error"] = Value::Null;
            entry["retentionWarning"] = result["retentionWarning"].clone();
            Ok(result)
        })();
        if let Err(error) = &attempt {
            entry["error"] = json!(error);
            entry["lastAttempt"] = json!(chrono_now());
            // Cancellation must retry; a foreground request is not an attempted source verification.
            if job.check().is_err() {
                entry["fingerprint"] = Value::Null;
            }
            if !anchor.is_dir() {
                entry["enabled"] = json!(false);
            }
        }
        self.save_registration(&key, entry)?;
        attempt
    }
    fn portable_transfer(
        &self,
        action: &str,
        args: &Value,
        adapter: &mut Adapter,
        job: &Job,
    ) -> Result<Value, String> {
        let destination = Path::new(str_arg(args, "destination")?);
        absolute(destination)?;
        ensure_plain(destination)?;
        if !destination.is_dir() {
            return Err("대상 폴더를 선택하세요.".into());
        }
        let catalog = adapter.snapshot()?;
        let revision = catalog_json(adapter.snapshot()?)["revision"].clone();
        if action == "portable_transfer" && args["expectedRevision"] != revision {
            return Err("목록이 바뀌었습니다. 전송 미리보기를 다시 확인하세요.".into());
        }
        let ids = strings(args, "ids")?;
        if ids.is_empty() {
            return Err("프로젝트를 선택하세요.".into());
        }
        let copy = args["copy"] == true;
        let mut plans = Vec::new();
        let mut selected_anchors = Vec::new();
        let mut targets: Vec<PathBuf> = Vec::new();
        for id in ids {
            let group = catalog
                .projects
                .iter()
                .find(|g| g.project["id"] == id)
                .ok_or("프로젝트가 없습니다.")?;
            let roots = roots(&group.project)?;
            // Prefer an outer root when SDK order starts with a nested restored root.
            let anchor = roots
                .iter()
                .find(|p| {
                    !roots
                        .iter()
                        .any(|outer| outer != *p && p.starts_with(outer))
                })
                .ok_or("연결 폴더가 없습니다.")?;
            adapter.portable_namespace(&id, anchor)?;
            let target = destination.join(anchor.file_name().ok_or("원본 폴더 이름이 없습니다.")?);
            let mapped = roots
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if let Ok(relative) = p.strip_prefix(anchor) {
                        target.join(relative)
                    } else {
                        destination.join(format!(
                            "{}-root-{}",
                            anchor.file_name().unwrap().to_string_lossy(),
                            i + 1
                        ))
                    }
                })
                .collect::<Vec<_>>();
            for path in if copy {
                vec![target.clone()]
            } else {
                mapped.clone()
            } {
                if path.exists() || roots.iter().any(|r| paths_overlap(r, &path)) {
                    return Err(format!(
                        "대상 폴더가 이미 있거나 원본과 겹칩니다: {}",
                        path.display()
                    ));
                }
                if targets
                    .iter()
                    .any(|previous| paths_overlap(previous, &path))
                {
                    return Err(
                        "선택한 프로젝트들의 대상 폴더가 겹칩니다. 하나씩 전송하세요.".into(),
                    );
                }
            }
            targets.push(target.clone());
            if !copy {
                targets.extend(mapped.iter().filter(|p| !p.starts_with(&target)).cloned());
            }
            if selected_anchors
                .iter()
                .any(|p: &PathBuf| paths_overlap(p, anchor))
            {
                return Err("공유·중첩 폴더의 프로젝트는 한 번에 하나씩 전송하세요.".into());
            }
            selected_anchors.push(anchor.clone());
            plans.push(json!({"id":id,"name":group.project["name"],"anchor":anchor,"destination":target,"roots":roots,"destinations":mapped,"conversationCount":group.threads.len()}));
        }
        let plan = json!({"projects":plans,"revision":revision,"copy":copy,"destination":destination,"warning":"폴더 안의 모든 일반 파일과 기존 대화 보관본을 함께 전송합니다. 복사는 원본 프로젝트 연결을 유지합니다. 이동은 대화 경로를 갱신하고 검증 후 원본을 정리합니다."});
        if action == "portable_transfer_plan" {
            return Ok(plan);
        }
        let mut results = Vec::new();
        for p in plan["projects"].as_array().unwrap() {
            job.check()?;
            let id = str_arg(p, "id")?;
            let anchor = Path::new(str_arg(p, "anchor")?);
            adapter.portable_snapshot(id, anchor, job)?;
            if copy {
                let op =
                    adapter.edit_files_to_folder("copy", &[anchor.to_owned()], destination, job)?;
                let recorded = adapter
                    .journal
                    .operation(&op)?
                    .ok_or("복사 작업 기록이 없습니다.")?;
                if recorded["payload"]["plans"][0]["target"] != p["destination"] {
                    return Err(format!("전송 중 같은 이름의 폴더가 생겨 별도 이름으로 보관했습니다. 작업 내역을 확인하세요. ({op})"));
                }
                let ns = adapter.portable_namespace(id, anchor)?;
                portable_inspect(Path::new(str_arg(p, "destination")?), &ns, job)?;
                results.push(json!({"operationId":op,"path":p["destination"]}));
            } else {
                let key = registration_key(str_arg(args, "home")?, id);
                let mut reg = self.registration(&key)?;
                if reg.is_object() {
                    reg["enabled"] = json!(false);
                    reg["error"]=json!("폴더 이동으로 자동 보관이 해제됐습니다. 이동 결과를 확인한 뒤 새 위치에서 다시 켜세요.");
                    self.save_registration(&key, reg)?;
                }
                let op = adapter.transfer_project(id, &paths(p, "destinations")?, true, job)?;
                results.push(json!({"operationId":op,"path":p["destination"]}));
            }
        }
        Ok(json!({"transfers":results}))
    }
    pub(super) fn destination_folders(&self, directory: &str) -> Result<Value, String> {
        let path = Path::new(directory);
        absolute(path)?;
        ensure_plain(path)?;
        if path.components().any(|c| {
            let name = c.as_os_str().to_string_lossy();
            name.eq_ignore_ascii_case(".codex-manager") || name.eq_ignore_ascii_case(FILE_RECOVERY)
        }) {
            return Err("대화 보관·복구 폴더를 대상 폴더로 사용할 수 없습니다.".into());
        }
        if let Some(root) = &self.options.test_root {
            if !clean_path(path).starts_with(clean_path(root)) {
                return Err("시험 폴더 밖을 탐색할 수 없습니다.".into());
            }
        }
        let mut folders = Vec::new();
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(".codex-manager")
                || entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(FILE_RECOVERY)
            {
                continue;
            }
            if entry.file_type().map_err(|e| e.to_string())?.is_dir() && ensure_plain(&p).is_ok() {
                folders.push(json!({"name":entry.file_name().to_string_lossy(),"path":p}));
            }
        }
        folders.sort_by_key(|f| f["name"].as_str().unwrap_or("").to_lowercase());
        Ok(json!({"path":path,"parent":path.parent(),"folders":folders}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_ticks_skip_failed_and_successful_sources_but_manual_forces_verification() {
        for error in [Value::Null, json!("corrupt")] {
            let entry = json!({"fingerprint":"same","error":error,"lastSuccessfulSave":"previous"});
            assert!(!should_capture(&entry, "same", false));
            assert!(should_capture(&entry, "changed", false));
            assert!(should_capture(&entry, "same", true));
            assert_eq!(entry["lastSuccessfulSave"], "previous");
        }
        assert!(should_capture(&Value::Null, "new", false));
    }
    #[test]
    fn completed_job_retention_is_bounded_without_losing_recent_results() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(root.join("home")).unwrap();
        let service = Arc::new(
            Service::new(Options {
                data: root.join("manager"),
                home: Some(root.join("home")),
                health: None,
                no_update: true,
                test_root: Some(root),
            })
            .unwrap(),
        );
        let mut ids = Vec::new();
        for _ in 0..140 {
            let id = service
                .start("layout_preferences".into(), json!({"enabled":false}))
                .unwrap();
            while service.job(&id).unwrap()["state"] == "running" {
                std::thread::yield_now();
            }
            ids.push(id);
        }
        assert!(service.jobs.lock().unwrap().len() <= 128);
        assert!(service.job(&ids[0]).is_err());
        assert_eq!(
            service.job(ids.last().unwrap()).unwrap()["state"],
            "completed"
        );
    }
    #[test]
    fn update_preempts_auto_then_failed_update_reopens_gate_and_manual_job_is_not_cancelled() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(root.join("home")).unwrap();
        let service = Arc::new(
            Service::new(Options {
                data: root.join("manager"),
                home: Some(root.join("home")),
                health: None,
                no_update: true,
                test_root: Some(root),
            })
            .unwrap(),
        );
        let auto = Job::new();
        service.gate.begin("auto", &auto, true).unwrap();
        let worker_service = service.clone();
        let worker_job = auto.clone();
        let worker = std::thread::spawn(move || {
            while worker_job.check().is_ok() {
                std::thread::yield_now();
            }
            worker_job.finish(Err("cancelled".into()));
            worker_service.gate.finish("auto");
        });
        assert!(service.apply_update().is_err()); // No prepared update: no installer or shutdown is launched.
        worker.join().unwrap();
        assert_eq!(auto.snapshot()["state"], "cancelled");
        let manual = Job::new();
        service.gate.begin("manual", &manual, false).unwrap();
        assert!(service.apply_update().is_err());
        assert!(manual.check().is_ok());
        assert!(!service.can_close());
        service.gate.finish("manual");
        assert!(service.can_close());
        assert!(service.gate.begin("after-exit", &Job::new(), true).is_err());
    }
    #[test]
    #[ignore = "installed native SDK; isolated fixture homes only"]
    fn native_automatic_save_persistence_skip_retry_and_missing_root() {
        let root =
            PathBuf::from(std::env::var_os("CODEX_TEST_HOME").unwrap()).join("automatic-service");
        assert!(!root.exists());
        fs::create_dir_all(&root).unwrap();
        let home = root.join("home");
        let anchor = root.join("project");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&anchor).unwrap();
        let options = || Options {
            data: root.join("manager"),
            home: Some(home.clone()),
            health: None,
            no_update: true,
            test_root: Some(root.clone()),
        };
        let service = Service::new(options()).unwrap();
        let mut adapter = service.adapter(home.to_str().unwrap()).unwrap();
        let external = root.join("external workspace");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("asset.txt"), b"external bytes").unwrap();
        let p = adapter
            .create("Automatic fixture", &[anchor.clone(), external])
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let seed = root.join("seed.jsonl");
        fs::write(&seed,format!("{}\n{}\n",json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":anchor,"runtime_workspace_roots":[anchor],"originator":"codex_cli_rs","cli_version":"0.0.0","source":"cli","model_provider":"openai"}}),json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"portable test"}]}}))).unwrap();
        adapter
            .import_thread(
                &seed,
                &id,
                &anchor,
                std::slice::from_ref(&anchor),
                p["id"].as_str().unwrap(),
                "Fixture",
            )
            .unwrap();
        adapter.release_server();
        let args = json!({"home":home,"project":p["id"],"anchor":anchor,"enabled":true});
        let first = service
            .run("portable_configure", &args, &Job::new())
            .unwrap();
        assert_eq!(first["complete"], true);
        assert_eq!(
            service.run("portable_tick", &args, &Job::new()).unwrap()["skipped"],
            true
        );
        let source = PathBuf::from(
            all_threads(&adapter.snapshot().unwrap())[0]["path"]
                .as_str()
                .unwrap(),
        );
        let raw = fs::read(&source).unwrap();
        fs::write(&source, b"corrupt\n").unwrap();
        assert!(service.run("portable_tick", &args, &Job::new()).is_err());
        let status = service.run("portable_status", &args, &Job::new()).unwrap();
        assert_eq!(
            status["snapshot"]["lastSuccessfulSave"],
            first["lastSuccessfulSave"]
        );
        assert!(status["registration"]["error"].is_string());
        assert_eq!(
            service.run("portable_tick", &args, &Job::new()).unwrap()["skipped"],
            true
        );
        assert!(service.run("portable_save", &args, &Job::new()).is_err());
        fs::write(&source, raw).unwrap();
        assert_eq!(
            service.run("portable_tick", &args, &Job::new()).unwrap()["complete"],
            true
        );
        drop(adapter);
        drop(service);
        let service = Service::new(options()).unwrap();
        assert_eq!(
            service.run("portable_status", &args, &Job::new()).unwrap()["registration"]["enabled"],
            true
        );
        // Exercise production dispatch: ordinary files, portable folder copy, in-place
        // restoration into another isolated store, then nested-root folder move.
        let archive = root.join("unregistered archive");
        fs::create_dir(&archive).unwrap();
        let document = anchor.join("ordinary.txt");
        fs::write(&document, b"ordinary bytes").unwrap();
        let files = json!({"home":home,"kind":"copy","sources":[document],"destination":archive});
        let copied_file = service.run("files_to_folder", &files, &Job::new()).unwrap();
        assert_eq!(
            fs::read(archive.join("ordinary.txt")).unwrap(),
            b"ordinary bytes"
        );
        service
            .run(
                "undo",
                &json!({"home":home,"id":copied_file["operationId"]}),
                &Job::new(),
            )
            .unwrap();
        let mut request = json!({"home":home,"ids":[p["id"]],"destination":archive,"copy":true});
        let preview = service
            .run("portable_transfer_plan", &request, &Job::new())
            .unwrap();
        request["expectedRevision"] = preview["revision"].clone();
        let copied = service
            .run("portable_transfer", &request, &Job::new())
            .unwrap();
        let copied_anchor = PathBuf::from(copied["transfers"][0]["path"].as_str().unwrap());
        assert_eq!(
            fs::read(copied_anchor.join("ordinary.txt")).unwrap(),
            b"ordinary bytes"
        );
        let found = service
            .run("portable_discover", &json!({"path":archive}), &Job::new())
            .unwrap();
        let ns = found["projects"][0]["namespace"].clone();
        let target_home = root.join("target");
        fs::create_dir(&target_home).unwrap();
        service
            .add_homes(std::slice::from_ref(&target_home))
            .unwrap();
        let restore = json!({"home":target_home,"anchor":copied_anchor,"namespace":ns});
        assert_eq!(
            service.run("portable_plan", &restore, &Job::new()).unwrap()["canRestore"],
            true
        );
        let restored = service
            .run("portable_restore", &restore, &Job::new())
            .unwrap();
        assert_eq!(restored["verified"], true);
        let move_parent = root.join("move destination");
        fs::create_dir(&move_parent).unwrap();
        let mut moving = json!({"home":target_home,"ids":[restored["projectId"]],"destination":move_parent,"copy":false});
        let move_plan = service
            .run("portable_transfer_plan", &moving, &Job::new())
            .unwrap();
        moving["expectedRevision"] = move_plan["revision"].clone();
        service
            .run("portable_transfer", &moving, &Job::new())
            .unwrap();
        assert!(!copied_anchor.exists());
        assert_eq!(
            fs::read(move_parent.join("project/ordinary.txt")).unwrap(),
            b"ordinary bytes"
        );
        assert_eq!(
            service.catalog(target_home.to_str().unwrap()).unwrap()["projects"][0]["project"]
                ["roots"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        fs::rename(&anchor, root.join("moved-away")).unwrap();
        assert!(service.run("portable_tick", &args, &Job::new()).is_err());
        assert!(!anchor.exists());
        assert_eq!(
            service.run("portable_status", &args, &Job::new()).unwrap()["registration"]["enabled"],
            false
        );
    }
}

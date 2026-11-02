use crate::{
    absolute, all_threads, backup_project, clean_path, digest, manifest_tree, map_workspace,
    paths_overlap, resume_copy_tree, roots, runtime_roots, scan_tree, verified_bundle, Adapter,
    Job, ThreadImport,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

pub fn messages_digest(path: &Path, job: &Job) -> Result<String, String> {
    let mut hash = Sha256::new();
    let mut reader = crate::RolloutReader::open_live(path, None, job)?;
    while let Some(record) = reader.next_record()? {
        let value = record.value;
        if value["type"] == "response_item" && value["payload"]["type"] == "message" {
            hash.update(serde_json::to_vec(&value["payload"]).map_err(|e| e.to_string())?);
            hash.update(b"\n");
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn validate_destinations(
    adapter: &Adapter,
    sources: &[PathBuf],
    destinations: &[PathBuf],
) -> Result<(), String> {
    for (i, destination) in destinations.iter().enumerate() {
        absolute(destination)?;
        if adapter
            .protected_storage()
            .iter()
            .any(|p| paths_overlap(destination, p))
        {
            return Err("Codex 저장소나 프로그램 설정 폴더를 대상으로 사용할 수 없습니다.".into());
        }
        if sources
            .iter()
            .any(|source| paths_overlap(source, destination))
            || destinations
                .iter()
                .take(i)
                .any(|d| paths_overlap(d, destination))
        {
            return Err("원본·대상 폴더가 겹칩니다.".into());
        }
    }
    Ok(())
}
impl Adapter {
    pub fn export_backup(
        &mut self,
        id: &str,
        destination: &Path,
        job: &Job,
    ) -> Result<String, String> {
        self.release_server();
        let snapshot = self.snapshot()?;
        let project = self.project(id)?;
        if roots(&project)?.iter().any(|source| {
            self.protected_storage()
                .iter()
                .any(|p| paths_overlap(source, p))
        }) {
            return Err("Codex 저장소·데이터베이스·Manager 설정 폴더를 프로젝트 백업으로 내보낼 수 없습니다.".into());
        }
        validate_destinations(self, &roots(&project)?, &[destination.to_owned()])?;
        let op=self.journal.begin(&json!({"kind":"rust-backup","home":self.store.home,"project":project,"destination":destination,"resources":[format!("backup:{}",destination.display())]}))?;
        let result = backup_project(&snapshot, id, destination, job).map(|_| ());
        match result {
            Ok(()) => self.finish(&op, Ok(())),
            Err(e) => {
                self.journal.record(
                    &op,
                    "cancelled",
                    json!({"error":e,"partialDestination":destination}),
                )?;
                Err(e)
            }
        }
    }
    pub fn import_backup(
        &mut self,
        bundle: &Path,
        destinations: &[PathBuf],
        target: Option<&str>,
        skip_existing: bool,
        resume: Option<&str>,
        job: &Job,
    ) -> Result<String, String> {
        self.guard()?;
        self.release_server();
        let verified = verified_bundle(bundle, job)?;
        let manifest = verified.manifest;
        if !manifest["dependencies"]
            .as_array()
            .ok_or("백업 의존성 정보 누락")?
            .is_empty()
        {
            return Err("외부 첨부·작업 폴더 또는 다른 프로젝트의 자식 대화가 포함된 백업입니다. 의존성을 먼저 확인하세요.".into());
        }
        let root_entries = manifest["roots"].as_array().ok_or("백업 루트 누락")?;
        if root_entries.len() != destinations.len() {
            return Err("가져올 폴더의 대상 개수가 일치하지 않습니다.".into());
        }
        let sources = root_entries
            .iter()
            .map(|r| bundle.join(r["bundle_path"].as_str().unwrap_or("")))
            .collect::<Vec<_>>();
        validate_destinations(self, &sources, destinations)?;
        let mapping = root_entries
            .iter()
            .zip(destinations)
            .map(|(r, d)| {
                (
                    PathBuf::from(r["original_path"].as_str().unwrap_or("")),
                    d.clone(),
                )
            })
            .collect::<Vec<_>>();
        let conversations = manifest["conversations"]
            .as_array()
            .ok_or("대화 목록 누락")?;
        let mut mapped = HashMap::new();
        for t in conversations {
            let cwd = map_workspace(
                Path::new(t["cwd"].as_str().ok_or("작업 경로 누락")?),
                &mapping,
            )?;
            let mut runtime = t["runtime_roots"]
                .as_array()
                .ok_or("작업 폴더 목록 누락")?
                .iter()
                .map(|p| {
                    map_workspace(
                        Path::new(p.as_str().ok_or("작업 폴더 경로 누락")?),
                        &mapping,
                    )
                })
                .collect::<Result<Vec<_>, String>>()?;
            if runtime.is_empty() {
                runtime.push(cwd.clone());
            }
            mapped.insert(
                t["id"].as_str().ok_or("대화 ID 누락")?.to_owned(),
                (cwd, runtime),
            );
        }
        let existing = all_threads(&self.snapshot()?);
        let previous = if let Some(id) = resume {
            Some(
                self.journal
                    .operation(id)?
                    .ok_or("중단된 가져오기 기록이 없습니다.")?,
            )
        } else {
            None
        };
        let manifest_hash = digest(&bundle.join("manifest.json"), job)?;
        let mut skipped = HashSet::new();
        let mut intended = HashSet::new();
        let mut previous_pid = None;
        let original_target;
        let op = if let Some(previous) = &previous {
            let p = &previous["payload"];
            if p["kind"] != "rust-import"
                || ["completed", "cancelled"].contains(&previous["state"].as_str().unwrap_or(""))
                || p["home"] != json!(self.store.home)
                || p["manifestHash"] != manifest_hash
                || p["destinations"] != json!(destinations)
                || p["target"].as_str() != target
            {
                return Err("중단된 가져오기와 백업·대상 설정이 다릅니다.".into());
            }
            skipped = p["skipped"]
                .as_array()
                .ok_or("중복 선택 기록 누락")?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            for e in self.journal.events(resume.unwrap())? {
                if let Some(id) = e["payload"]["threadId"].as_str() {
                    intended.insert(id.to_owned());
                }
                if let Some(pid) = e["payload"]["projectId"].as_str() {
                    previous_pid = Some(pid.to_owned());
                }
            }
            original_target = p["targetBefore"].clone();
            resume.unwrap().to_owned()
        } else {
            if skip_existing {
                skipped = conversations
                    .iter()
                    .filter_map(|t| t["id"].as_str())
                    .filter(|id| existing.iter().any(|e| e["id"] == *id))
                    .map(str::to_owned)
                    .collect();
            }
            original_target = if let Some(id) = target {
                self.project(id)?
            } else {
                Value::Null
            };
            for destination in destinations {
                if destination.exists() {
                    return Err("대상 폴더가 이미 있습니다. 기존 파일을 덮어쓰지 않습니다.".into());
                }
            }
            self.begin(json!({"kind":"rust-import","bundle":bundle,"manifestHash":manifest_hash,"bundleId":manifest["bundle_id"],"destinations":destinations,"target":target,"targetBefore":original_target,"skipped":skipped,"projectKey":uuid::Uuid::new_v4().to_string(),"resources":[format!("bundle:{}",manifest["bundle_id"].as_str().unwrap_or(""))]}))?
        };
        let payload = self.journal.operation(&op)?.ok_or("가져오기 기록 누락")?["payload"].clone();
        let result = (|| {
            for t in conversations {
                let id = t["id"].as_str().ok_or("대화 ID 오류")?;
                if let Some(current) = existing.iter().find(|e| e["id"] == id) {
                    if skipped.contains(id) {
                        continue;
                    }
                    let (cwd, runtime) = mapped.get(id).ok_or("대화 경로 매핑 누락")?;
                    let source = bundle.join(t["rollout"].as_str().ok_or("대화 경로 누락")?);
                    let request = ThreadImport {
                        source: &source,
                        id,
                        cwd,
                        runtime,
                        project: previous_pid.as_deref(),
                        previous_project: None,
                        title: t["title"].as_str().unwrap_or("가져온 대화"),
                        archived: t["archived"] == true,
                        expected_sha256: None,
                    };
                    if !intended.contains(id) || !self.owns_rollout(&op, current, job)? {
                        return Err(
                            "같은 ID의 대화가 있습니다. 중복 대화 유지 옵션을 선택하세요.".into(),
                        );
                    }
                    self.validate_owned_import(&request, &op, current, job)?;
                }
            }
            for ((r, source), destination) in root_entries.iter().zip(&sources).zip(destinations) {
                job.check()?;
                self.journal
                    .record(&op, "copying", json!({"destination":destination}))?;
                resume_copy_tree(
                    source,
                    destination,
                    &manifest_tree(&manifest, r["id"].as_str().ok_or("루트 ID 오류")?)?,
                    job,
                )?;
            }
            job.check()?;
            job.phase("프로젝트 등록", 0);
            let project = if let Some(id) = target {
                let mut p = self.project(id)?;
                let mut paths = roots(&original_target)?;
                for path in destinations {
                    // A merge into a folder already covered by a project root
                    // must not create nested roots that later exports reject.
                    let covered = paths
                        .iter()
                        .any(|root| map_workspace(path, &[(root.clone(), root.clone())]).is_ok());
                    if !covered {
                        paths.push(path.clone());
                    }
                }
                let current_paths = roots(&p)?;
                if current_paths != roots(&original_target)? && current_paths != paths {
                    return Err("대상 프로젝트가 이후 변경됐습니다.".into());
                }
                p["roots"] = json!(paths.iter().map(|p| json!({"path":p})).collect::<Vec<_>>());
                self.update_project(&p)?;
                p
            } else {
                let p = self.create_key(
                    manifest["project"]["name"]
                        .as_str()
                        .ok_or("프로젝트 이름 누락")?,
                    destinations,
                    payload["projectKey"].as_str().ok_or("복원 키 누락")?,
                )?;
                if roots(&p)? != destinations {
                    return Err("가져오는 프로젝트의 폴더가 이후 변경됐습니다.".into());
                }
                p
            };
            let pid = project["id"].as_str().ok_or("프로젝트 ID 누락")?.to_owned();
            self.journal
                .record(&op, "project-created", json!({"projectId":pid}))?;
            let mut assignments = HashMap::new();
            job.phase_items("대화 가져오기·연결 검증", conversations.len() as u64);
            for (i, t) in conversations.iter().enumerate() {
                job.check()?;
                let id = t["id"].as_str().ok_or("대화 ID 누락")?;
                if skipped.contains(id) {
                    continue;
                }
                let (cwd, runtime) = mapped.get(id).ok_or("대화 경로 매핑 누락")?;
                let source = bundle.join(t["rollout"].as_str().ok_or("대화 기록 경로 누락")?);
                let expected = verified
                    .checksums
                    .iter()
                    .find(|entry| entry["path"] == t["rollout"])
                    .and_then(|entry| entry["sha256"].as_str())
                    .ok_or("대화 무결성 정보 누락")?;
                self.import_owned(
                    &ThreadImport {
                        source: &source,
                        id,
                        cwd,
                        runtime,
                        project: Some(&pid),
                        previous_project: None,
                        title: t["title"].as_str().unwrap_or("가져온 대화"),
                        archived: t["archived"] == true,
                        expected_sha256: Some(expected),
                    },
                    &op,
                    job,
                )?;
                let read = self.read_thread(id)?;
                if clean_path(Path::new(
                    read["cwd"].as_str().ok_or("가져온 작업 경로 누락")?,
                )) != *cwd
                    || read["projectId"] != pid
                {
                    return Err("가져온 대화의 연결을 확인할 수 없습니다.".into());
                }
                let active = all_threads(&self.snapshot()?)
                    .into_iter()
                    .find(|v| v["id"] == id)
                    .ok_or("가져온 대화가 목록에 없습니다.")?;
                if active["parentThreadId"] != t["parent_id"] {
                    return Err("가져온 대화의 부모 연결이 일치하지 않습니다.".into());
                }
                assignments.insert(id.to_owned(), Some(pid.clone()));
                job.advance((i + 1) as u64, t["title"].as_str().unwrap_or(id), 0);
            }
            self.release_server();
            for t in conversations {
                let id = t["id"].as_str().unwrap();
                if skipped.contains(id) {
                    continue;
                }
                let active = all_threads(&self.snapshot()?)
                    .into_iter()
                    .find(|v| v["id"] == id)
                    .ok_or("가져온 대화 누락")?;
                if runtime_roots(Path::new(
                    active["path"].as_str().ok_or("가져온 대화 경로 누락")?,
                ))? != mapped[id].1
                {
                    return Err("가져온 대화의 작업 폴더를 확인할 수 없습니다.".into());
                }
            }
            self.sync(&[project], &assignments, &[], &[])?;
            self.journal.record(
                &op,
                "import-verified",
                json!({"projectId":pid,"skipped":skipped.len()}),
            )?;
            Ok(())
        })();
        self.finish(&op, result)
    }
    pub fn transfer_project(
        &mut self,
        id: &str,
        destinations: &[PathBuf],
        clean: bool,
        job: &Job,
    ) -> Result<String, String> {
        self.guard()?;
        let project = self.project(id)?;
        let paths = roots(&project)?;
        if paths.iter().any(|source| {
            self.protected_storage()
                .iter()
                .any(|p| paths_overlap(source, p))
        }) {
            return Err("Codex 저장소·데이터베이스·Manager 프로그램과 설정 폴더는 프로젝트 이동으로 변경할 수 없습니다.".into());
        }
        if paths.len() != destinations.len() || paths.is_empty() {
            return Err("원본 폴더마다 새 대상 폴더를 지정하세요.".into());
        }
        // Nested registered roots travel with their outer root. Require exactly the same
        // relative mapping, then copy and recover each physical tree only once.
        let mut physical = Vec::new();
        for (index, source) in paths.iter().enumerate() {
            if paths
                .iter()
                .take(index)
                .any(|p| clean_path(p) == clean_path(source))
            {
                return Err("같은 연결 폴더가 중복되어 있습니다.".into());
            }
            absolute(source)?;
            if !source.is_dir() {
                return Err("연결된 원본 폴더가 없습니다.".into());
            }
            absolute(&destinations[index])?;
            if let Some((outer, parent)) = paths
                .iter()
                .enumerate()
                .filter(|(i, p)| *i != index && source.starts_with(p))
                .min_by_key(|(_, p)| p.components().count())
            {
                let expected = destinations[outer]
                    .join(source.strip_prefix(parent).map_err(|e| e.to_string())?);
                if clean_path(&expected) != clean_path(&destinations[index]) {
                    return Err(
                        "중첩 연결 폴더는 상위 폴더 안의 같은 상대 위치로 이동하세요.".into(),
                    );
                }
            } else {
                physical.push(index);
            }
        }
        let physical_destinations = physical
            .iter()
            .map(|i| destinations[*i].clone())
            .collect::<Vec<_>>();
        validate_destinations(self, &paths, &physical_destinations)?;
        let snapshot = self.snapshot()?;
        crate::connections::require_connections(&snapshot, &[id.to_owned()])?;
        for g in &snapshot.projects {
            if g.project["id"] == id {
                continue;
            }
            if roots(&g.project)?
                .iter()
                .any(|r| paths.iter().any(|p| paths_overlap(p, r)))
            {
                return Err("다른 프로젝트와 공유하는 폴더는 이동할 수 없습니다.".into());
            }
        }
        let requested = all_threads(&snapshot)
            .into_iter()
            .filter(|t| t["projectId"] == id)
            .filter_map(|t| t["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let all = all_threads(&snapshot);
        let ids = crate::expand_thread_ids(&all, &requested)?;
        let threads = all
            .iter()
            .filter(|t| {
                t["id"]
                    .as_str()
                    .is_some_and(|id| ids.iter().any(|selected| selected == id))
            })
            .cloned()
            .collect::<Vec<_>>();
        for path in &paths {
            if path.join(".git").is_file() {
                return Err("연결된 Git worktree는 별도 이동 처리가 필요합니다.".into());
            }
        }
        if clean {
            for thread in &all {
                if thread["id"]
                    .as_str()
                    .is_some_and(|id| ids.iter().any(|selected| selected == id))
                {
                    continue;
                }
                if thread["cwd"]
                    .as_str()
                    .is_some_and(|cwd| paths.iter().any(|p| paths_overlap(Path::new(cwd), p)))
                {
                    return Err("선택하지 않은 대화가 사용하는 폴더입니다. 원본을 유지하거나 연결 범위를 먼저 정리하세요.".into());
                }
                if let Some(path) = thread["path"].as_str() {
                    let runtime = crate::runtime_roots_with_job(Path::new(path), job)?;
                    if runtime
                        .iter()
                        .any(|root| paths.iter().any(|p| paths_overlap(root, p)))
                    {
                        return Err("선택하지 않은 대화의 보조 작업 폴더입니다. 원본을 유지하거나 연결 범위를 먼저 정리하세요.".into());
                    }
                }
            }
        }
        let mapping = paths
            .iter()
            .cloned()
            .zip(destinations.iter().cloned())
            .collect::<Vec<_>>();
        let mut mapped = HashMap::new();
        for t in &threads {
            let tid = t["id"].as_str().ok_or("대화 ID 누락")?;
            let cwd = map_workspace(
                Path::new(t["cwd"].as_str().ok_or("대화 작업 경로 누락")?),
                &mapping,
            )?;
            let mut runtime =
                runtime_roots(Path::new(t["path"].as_str().ok_or("대화 기록 누락")?))?;
            if runtime.is_empty() {
                runtime.push(PathBuf::from(t["cwd"].as_str().unwrap()));
            }
            let runtime = runtime
                .iter()
                .map(|p| map_workspace(p, &mapping))
                .collect::<Result<Vec<_>, _>>()?;
            mapped.insert(tid.to_owned(), (cwd, runtime));
        }
        // Keep only the verified transfer plan locally. Project bytes are copied once
        // to the chosen destination, never duplicated onto the system drive.
        self.release_server();
        let mut inventory = Vec::new();
        let mut directories = Vec::new();
        let mut root_records = Vec::new();
        for &index in &physical {
            let source = &paths[index];
            let destination = &destinations[index];
            let rid = format!("root-{:02}", index + 1);
            let tree = scan_tree(source, job)?;
            for entry in &tree.files {
                inventory.push(json!({"root_id":rid,"relative_path":entry.path,"size":entry.size,"sha256":entry.sha256}));
            }
            for dir in &tree.directories {
                directories.push(json!([rid, dir]));
            }
            root_records.push(json!({"id":rid,"original_path":source,"destination":destination}));
        }
        let mut conversations = Vec::new();
        for thread in &threads {
            conversations.push(json!({"id":thread["id"],"cwd":thread["cwd"],"runtime_roots":runtime_roots(Path::new(thread["path"].as_str().ok_or("대화 기록 누락")?))?}));
        }
        let manifest = json!({"roots":root_records,"inventory":inventory,"directories":directories,"conversations":conversations});
        let recovery = self
            .recovery_root()
            .join(format!("transfer-{}.json", uuid::Uuid::new_v4()));
        fs::create_dir_all(recovery.parent().ok_or("복구 폴더 누락")?)
            .map_err(|e| e.to_string())?;
        crate::write_json_new(&recovery, &manifest)?;
        let plan_hash = digest(&recovery, job)?;
        let mut after = project.clone();
        after["roots"] = json!(destinations
            .iter()
            .map(|p| json!({"path":p}))
            .collect::<Vec<_>>());
        let op=self.begin(json!({"kind":"rust-transfer","projects":[project],"afterProjects":[after],"threads":threads,"destinations":destinations,"recovery":recovery,"transferPlanHash":plan_hash,"clean":clean,"resources":[id]}))?;
        let result = (|| {
            let mut captured = Vec::new();
            for &index in &physical {
                let source = &paths[index];
                let destination = &destinations[index];
                job.check()?;
                let tree = scan_tree(source, job)?;
                let expected = manifest_tree(&manifest, &format!("root-{:02}", index + 1))?;
                if tree.files.len() != expected.files.len()
                    || tree.directories.len() != expected.directories.len()
                    || expected.files.iter().any(|e| {
                        !tree
                            .files
                            .iter()
                            .any(|f| f.path == e.path && f.size == e.size && f.sha256 == e.sha256)
                    })
                    || expected
                        .directories
                        .iter()
                        .any(|d| !tree.directories.contains(d))
                {
                    return Err(
                        "복구 사본 생성 후 원본 폴더가 변경됐습니다. 원본을 유지했습니다.".into(),
                    );
                }
                crate::copy_tree(source, destination, &tree, job)?;
                captured.push(tree);
            }
            job.check()?;
            self.update_project(&after)?;
            for thread in &threads {
                let tid = thread["id"].as_str().unwrap();
                let (cwd, runtime) = &mapped[tid];
                self.journal
                    .record(&op, "relinking", json!({"threadId":tid}))?;
                self.relocate(thread, cwd, runtime)?;
            }
            self.sync(&[after], &HashMap::new(), &[], &[])?;
            if clean {
                self.release_server();
                for (&index, tree) in physical.iter().zip(&captured) {
                    let source = &paths[index];
                    let destination = &destinations[index];
                    self.journal.record(
                        &op,
                        "cleaning-source",
                        json!({"source":source,"destination":destination}),
                    )?;
                    crate::remove_verified_tree(source, destination, tree, job)?;
                }
            }
            Ok(())
        })();
        self.finish(&op, result)
    }
}

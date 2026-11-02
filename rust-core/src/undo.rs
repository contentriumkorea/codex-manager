use crate::{
    all_threads, clean_path, digest, load_verified_bundle, manifest_tree, map_workspace,
    resume_copy_tree, roots, Adapter, Job, ThreadImport,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
fn same_runtime_roots(a: &[PathBuf], b: &[PathBuf]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            clean_path(a)
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase()
                == clean_path(b)
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_lowercase()
        })
}

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .ok_or_else(|| format!("복구 정보 누락: {key}"))
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    v[key]
        .as_array()
        .ok_or_else(|| format!("복구 목록 누락: {key}"))
}
fn old_project(p: &Value) -> Value {
    json!({"id":p["id"],"name":p["name"],"roots":p["roots"].as_array().unwrap_or(&Vec::new()).iter().map(|r|if r.is_string(){json!({"path":r})}else{r.clone()}).collect::<Vec<_>>()})
}
impl Adapter {
    pub fn undo(&mut self, id: &str) -> Result<(), String> {
        self.undo_with_job(id, &Job::new())
    }
    pub fn undo_with_job(&mut self, id: &str, job: &Job) -> Result<(), String> {
        job.check()?;
        self.ensure_mutation_lock()?;
        let record = self.journal.operation(id)?.ok_or("작업 내역이 없습니다.")?;
        let mut p = record["payload"].clone();
        p["_operationState"] = record["state"].clone();
        let kind = field(&p, "kind")?.to_owned();
        if kind == "rust-cross-store" {
            return Err("저장소 간 전송은 되돌리기를 지원하지 않습니다. 미완료 전송은 전송 계속을 사용하세요.".into());
        }
        if clean_path(Path::new(field(&p, "home")?))
            .to_string_lossy()
            .to_lowercase()
            != clean_path(&self.store.home)
                .to_string_lossy()
                .to_lowercase()
        {
            return Err("작업을 시작한 Codex 저장소를 선택하세요.".into());
        }
        if self
            .journal
            .events(id)?
            .iter()
            .any(|e| e["payload"]["recovered"] == true)
        {
            return Err("이미 복구한 작업입니다.".into());
        }
        if ![
            "rust-files",
            "file-edit",
            "rust-project-edit",
            "rust-thread-names",
            "rust-assign",
            "rust-merge",
            "rust-delete",
            "rust-transfer",
            "rename-thread",
            "rename-threads",
            "connections",
            "management-delete",
        ]
        .contains(&kind.as_str())
        {
            return Err(
                "이 작업은 되돌릴 수 없습니다. 중단된 가져오기는 이어서 가져오기를 선택하세요."
                    .into(),
            );
        }
        if p["permanent"] == true {
            return Err("완전 삭제한 대화는 복구할 수 없습니다.".into());
        }
        if !["rust-files", "file-edit"].contains(&kind.as_str()) {
            self.guard()?;
        }
        let mut resources = p["resources"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        resources.push(format!("home:{}", self.store.home.display()));
        self.journal.claim(id, &resources)?;
        if kind == "rename-thread" {
            p["threads"] = json!([{"id":p["thread_id"],"name":p["old_name"]}]);
            p["names"] = json!([p["new_name"]]);
        }
        if kind == "rename-threads" {
            let changes = array(&p, "changes")?.clone();
            p["threads"] = json!(changes
                .iter()
                .map(|c| json!({"id":c["id"],"name":c["old_name"]}))
                .collect::<Vec<_>>());
            p["names"] = json!(changes
                .iter()
                .map(|c| c["new_name"].clone())
                .collect::<Vec<_>>());
        }
        *self
            .undo_attempt
            .lock()
            .map_err(|_| "복구 작업 잠금 오류")? = Some((id.to_owned(), false));
        let result = match kind.as_str() {
            "rust-files" => self.undo_files(id, &p, job),
            "file-edit" => {
                let mut converted = p.clone();
                if let Some(plans) = converted["plans"].as_array_mut() {
                    for plan in plans {
                        plan["sourceId"] = plan["source_id"].clone();
                    }
                }
                self.undo_files(id, &converted, job)
            }
            "rust-thread-names" | "rename-thread" | "rename-threads" => self.undo_names(&p),
            "rust-project-edit" => self.undo_project(&p),
            "rust-assign" => self.undo_assign(&p),
            "rust-merge" | "rust-delete" => self.undo_removed(id, &p, job),
            "rust-transfer" => self.undo_transfer(&p, job),
            "connections" => self.undo_legacy_connections(&p),
            "management-delete" => self.undo_legacy_delete(id, &p, job),
            _ => unreachable!(),
        };
        let changed = self
            .undo_attempt
            .lock()
            .map_err(|_| "복구 작업 잠금 오류")?
            .take()
            .is_some_and(|(_, started)| started);
        match result {
            Ok(()) => self
                .journal
                .record(id, "completed", json!({"recovered":true})),
            Err(error) => {
                self.journal.record(
                    id,
                    if !changed && record["state"] == "completed" {
                        "completed"
                    } else {
                        "needs_recovery"
                    },
                    json!({"error":error,"undoBlocked":!changed}),
                )?;
                Err(error)
            }
        }
    }
    fn undo_names(&mut self, p: &Value) -> Result<(), String> {
        let threads = array(p, "threads")?;
        let names = array(p, "names")?;
        if threads.len() != names.len() {
            return Err("제목 복구 정보 오류".into());
        }
        for (t, name) in threads.iter().zip(names) {
            let current = self.read_thread(field(t, "id")?)?;
            if current["name"] != *name && current["name"] != t["name"] {
                return Err("대화 제목이 이후 변경됐습니다. 덮어쓰지 않습니다.".into());
            }
        }
        for t in threads {
            self.restore_name(
                field(t, "id")?,
                t["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("제목 없는 대화"),
            )?;
        }
        Ok(())
    }
    fn undo_project(&mut self, p: &Value) -> Result<(), String> {
        let before = &p["projects"][0];
        let after = &p["afterProjects"][0];
        let current = self.project(field(before, "id")?)?;
        if (current["name"] != after["name"] && current["name"] != before["name"])
            || (current["roots"] != after["roots"] && current["roots"] != before["roots"])
        {
            return Err("프로젝트가 이후 변경됐습니다. 덮어쓰지 않습니다.".into());
        }
        self.update_project(before)?;
        self.sync(std::slice::from_ref(before), &HashMap::new(), &[], &[])
    }
    fn undo_assign(&mut self, p: &Value) -> Result<(), String> {
        let threads = array(p, "threads")?;
        for t in threads {
            let current = self.read_thread(field(t, "id")?)?;
            if current["projectId"] != p["target"] && current["projectId"] != t["projectId"] {
                return Err("대화 소속이 이후 변경됐습니다.".into());
            }
        }
        let mut assignments = HashMap::new();
        for t in threads {
            let tid = field(t, "id")?;
            self.assign(tid, t["projectId"].as_str())?;
            assignments.insert(tid.into(), t["projectId"].as_str().map(str::to_owned));
        }
        self.sync(&[], &assignments, &[], &[])
    }
    fn undo_removed(&mut self, id: &str, p: &Value, job: &Job) -> Result<(), String> {
        let merge = p["kind"] == "rust-merge";
        let originals = array(p, "projects")?;
        let threads = array(p, "threads")?;
        let known = all_threads(&self.snapshot()?);
        if merge {
            let target = &p["targetBefore"];
            let current = self.project(field(target, "id")?)?;
            if current["name"] != target["name"]
                || (roots(&current)? != roots(&p["targetAfter"])?
                    && roots(&current)? != roots(target)?)
            {
                return Err("합친 대상 프로젝트가 이후 변경됐습니다.".into());
            }
        }
        for t in threads {
            let tid = field(t, "id")?;
            if let Some(current) = known.iter().find(|v| v["id"] == tid) {
                let actual = self.read_thread(tid)?;
                if actual["projectId"] != t["projectId"]
                    && (!merge || actual["projectId"] != p["targetBefore"]["id"])
                {
                    let previous = self.journal.events(id)?.iter().any(|e| {
                        e["phase"] == "restoring-thread"
                            && e["payload"]["threadId"] == tid
                            && e["payload"]["projectId"] == actual["projectId"]
                    });
                    if !previous && !self.owns_rollout(id, current, job)? {
                        return Err("대화 소속이 이후 변경됐습니다.".into());
                    }
                }
                if clean_path(Path::new(field(current, "cwd")?))
                    != clean_path(Path::new(field(t, "cwd")?))
                {
                    return Err("대화 작업 경로가 이후 변경됐습니다.".into());
                }
            } else if merge {
                return Err("합친 대화가 이후 삭제됐습니다.".into());
            } else {
                let saved = array(p, "saved")?
                    .iter()
                    .find(|v| v["id"] == tid)
                    .ok_or("복구 사본 누락")?;
                if digest(Path::new(field(saved, "path")?), job)? != field(saved, "sha256")? {
                    return Err("복구 사본이 변경됐습니다.".into());
                }
            }
        }
        let mut mapping = HashMap::new();
        let mut restored = Vec::new();
        for project in originals {
            let old_id = field(project, "id")?;
            let existing = self
                .snapshot()?
                .projects
                .into_iter()
                .find(|g| g.project["id"] == old_id);
            let new = if let Some(group) = existing {
                if group.project["name"] != project["name"]
                    || roots(&group.project)? != roots(project)?
                {
                    return Err("프로젝트가 이후 변경됐습니다.".into());
                }
                group.project
            } else {
                self.create_key(
                    field(project, "name")?,
                    &roots(project)?,
                    p["restoreKeys"][old_id].as_str().ok_or("복원 키 누락")?,
                )?
            };
            mapping.insert(old_id.to_owned(), field(&new, "id")?.to_owned());
            restored.push(new);
        }
        let mut assignments = HashMap::new();
        for t in threads {
            let tid = field(t, "id")?;
            let pid = t["projectId"]
                .as_str()
                .map(|pid| mapping.get(pid).map(String::as_str).unwrap_or(pid));
            let exists = known.iter().any(|v| v["id"] == tid);
            self.journal.record(
                id,
                "restoring-thread",
                json!({"threadId":tid,"projectId":pid}),
            )?;
            if !exists
                || known
                    .iter()
                    .find(|v| v["id"] == tid)
                    .is_some_and(|v| self.owns_rollout(id, v, job).unwrap_or(false))
            {
                let saved = array(p, "saved")?
                    .iter()
                    .find(|v| v["id"] == tid)
                    .ok_or("복구 사본 누락")?;
                let paths = array(saved, "runtimeRoots")?
                    .iter()
                    .map(|v| v.as_str().map(PathBuf::from).ok_or("작업 폴더 경로 오류"))
                    .collect::<Result<Vec<_>, _>>()?;
                self.import_owned(
                    &ThreadImport {
                        source: Path::new(field(saved, "path")?),
                        id: tid,
                        cwd: Path::new(field(t, "cwd")?),
                        runtime: &paths,
                        project: pid,
                        previous_project: t["projectId"].as_str(),
                        title: t["name"].as_str().unwrap_or("복원한 대화"),
                        archived: t["archived"] == true,
                        expected_sha256: Some(field(saved, "sha256")?),
                    },
                    id,
                    job,
                )?;
            } else {
                self.assign(tid, pid)?;
            }
            assignments.insert(tid.to_owned(), pid.map(str::to_owned));
        }
        if merge {
            self.update_project(&p["targetBefore"])?;
            restored.push(p["targetBefore"].clone());
        }
        self.sync(&restored, &assignments, &[], &[])
    }
    fn undo_transfer(&mut self, p: &Value, job: &Job) -> Result<(), String> {
        let backup = Path::new(field(p, "recovery")?);
        let plan = p["transferPlanHash"].as_str();
        let manifest = if let Some(hash) = plan {
            if digest(backup, job)? != hash {
                return Err("폴더 이동 복구 계획이 변경됐습니다.".into());
            }
            serde_json::from_slice(&std::fs::read(backup).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        } else {
            load_verified_bundle(backup, job)?
        };
        let before = &p["projects"][0];
        let after = &p["afterProjects"][0];
        let current = self.project(field(before, "id")?)?;
        if current["name"] != before["name"]
            || (roots(&current)? != roots(before)? && roots(&current)? != roots(after)?)
        {
            return Err("프로젝트가 이후 변경됐습니다.".into());
        }
        let mapping = roots(before)?
            .into_iter()
            .zip(roots(after)?)
            .collect::<Vec<_>>();
        let mut original_connections = roots(&current)? == roots(before)?;
        for thread in array(p, "threads")? {
            job.check()?;
            let read = self.read_thread(field(thread, "id")?)?;
            let old = Path::new(field(thread, "cwd")?);
            let new = map_workspace(old, &mapping)?;
            let cwd = clean_path(Path::new(field(&read, "cwd")?));
            if read["projectId"] != before["id"] || (cwd != clean_path(old) && cwd != new) {
                return Err("대화 소속이나 작업 경로가 이후 변경됐습니다.".into());
            }
            let record = array(&manifest, "conversations")?
                .iter()
                .find(|v| v["id"] == thread["id"])
                .ok_or("복구 대화 정보 누락")?;
            let mut original = array(record, "runtime_roots")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|p| clean_path(Path::new(p)))
                        .ok_or("작업 폴더 경로 오류")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if original.is_empty() {
                original.push(clean_path(old));
            }
            let expected = original
                .iter()
                .map(|p| map_workspace(p, &mapping))
                .collect::<Result<Vec<_>, _>>()?;
            let current = all_threads(&self.snapshot()?)
                .into_iter()
                .find(|t| t["id"] == thread["id"])
                .ok_or("복구할 대화가 없습니다.")?;
            let actual = crate::runtime_roots_with_job(Path::new(field(&current, "path")?), job)?;
            original_connections &=
                cwd == clean_path(old) && same_runtime_roots(&actual, &original);
            if !same_runtime_roots(&actual, &original) && !same_runtime_roots(&actual, &expected) {
                return Err(
                    "대화의 보조 작업 폴더가 이후 변경됐습니다. 현재 설정을 덮어쓰지 않습니다."
                        .into(),
                );
            }
        }
        self.release_server();
        // A copy cancelled before relinking has nothing to undo if every
        // original byte and connection remains intact. Preserve partial outputs.
        let complete = |actual: &crate::Tree, expected: &crate::Tree| {
            actual.files.len() == expected.files.len()
                && actual.directories.len() == expected.directories.len()
                && expected.files.iter().all(|e| {
                    actual
                        .files
                        .iter()
                        .any(|a| a.path == e.path && a.size == e.size && a.sha256 == e.sha256)
                })
                && expected
                    .directories
                    .iter()
                    .all(|d| actual.directories.contains(d))
        };
        let mut all_originals_complete = true;
        for root in array(&manifest, "roots")? {
            let expected = manifest_tree(&manifest, field(root, "id")?)?;
            let original = Path::new(field(root, "original_path")?);
            if original.exists() {
                let actual = crate::scan_tree(original, job)?;
                if actual.files.iter().any(|a| {
                    !expected
                        .files
                        .iter()
                        .any(|e| a.path == e.path && a.size == e.size && a.sha256 == e.sha256)
                }) || actual
                    .directories
                    .iter()
                    .any(|d| !expected.directories.contains(d))
                {
                    return Err(
                        "원래 폴더에 이후 작업한 파일이 있습니다. 현재 파일을 보존했습니다.".into(),
                    );
                }
                all_originals_complete &= complete(&actual, &expected);
            } else {
                all_originals_complete = false;
            }
        }
        if plan.is_some()
            && p["_operationState"] != "completed"
            && original_connections
            && all_originals_complete
        {
            return Ok(());
        }
        if plan.is_some() {
            for root in array(&manifest, "roots")? {
                let actual = crate::scan_tree(Path::new(field(root, "destination")?), job)?;
                let expected = manifest_tree(&manifest, field(root, "id")?)?;
                if !complete(&actual, &expected) {
                    return Err(
                        "이동 대상의 파일이 이후 변경됐습니다. 현재 작업을 보존했습니다.".into(),
                    );
                }
            }
        }
        self.mark_undo_mutation()?;
        for root in array(&manifest, "roots")? {
            let source = if plan.is_some() {
                PathBuf::from(field(root, "destination")?)
            } else {
                backup.join(field(root, "bundle_path")?)
            };
            resume_copy_tree(
                &source,
                Path::new(field(root, "original_path")?),
                &manifest_tree(&manifest, field(root, "id")?)?,
                job,
            )?;
        }
        self.update_project(before)?;
        for thread in array(p, "threads")? {
            let record = array(&manifest, "conversations")?
                .iter()
                .find(|v| v["id"] == thread["id"])
                .ok_or("복구 대화 정보 누락")?;
            let mut paths = array(record, "runtime_roots")?
                .iter()
                .map(|v| v.as_str().map(PathBuf::from).ok_or("작업 폴더 경로 오류"))
                .collect::<Result<Vec<_>, _>>()?;
            if paths.is_empty() {
                paths.push(PathBuf::from(field(record, "cwd")?));
            }
            self.relocate(thread, Path::new(field(record, "cwd")?), &paths)?;
        }
        self.sync(std::slice::from_ref(before), &HashMap::new(), &[], &[])
    }
    fn undo_legacy_connections(&mut self, p: &Value) -> Result<(), String> {
        let before = old_project(&p["project"]);
        if p.get("expected_project").is_some() {
            let current = self.project(field(&before, "id")?)?;
            let after = old_project(&p["expected_project"]);
            if (current["name"] != before["name"] || roots(&current)? != roots(&before)?)
                && (current["name"] != after["name"] || roots(&current)? != roots(&after)?)
            {
                return Err("프로젝트가 이후 변경됐습니다.".into());
            }
        }
        let assignments = p["assignments"].as_object().ok_or("복구 대화 연결 누락")?;
        for (tid, old) in assignments {
            let current = self.read_thread(tid)?;
            if p.get("expected_assignments").is_some()
                && current["projectId"] != *old
                && current["projectId"] != p["expected_assignments"][tid]
            {
                return Err("대화 소속이 이후 변경됐습니다.".into());
            }
        }
        self.update_project(&before)?;
        let mut restored = HashMap::new();
        for (tid, old) in assignments {
            self.assign(tid, old.as_str())?;
            restored.insert(tid.clone(), old.as_str().map(str::to_owned));
        }
        self.sync(&[before], &restored, &[], &[])
    }
    fn undo_legacy_delete(&mut self, id: &str, p: &Value, job: &Job) -> Result<(), String> {
        let recovery = Path::new(field(p, "recovery")?);
        let meta = recovery.join("metadata.json");
        if digest(&meta, job)? != field(p, "metadata_digest")? {
            return Err("이전 버전 복구 정보가 변경됐습니다.".into());
        }
        let data: Value = serde_json::from_slice(&std::fs::read(meta).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let mut converted =
            json!({"kind":"rust-delete","projects":[],"threads":[],"saved":[],"restoreKeys":{}});
        let original = &data["project"];
        if !original.is_null() {
            let project = old_project(original);
            converted["projects"] = json!([project]);
            converted["restoreKeys"][field(original, "id")?] =
                json!(format!("restore-delete-{id}"));
        }
        let mut threads = Vec::new();
        let mut saved = Vec::new();
        for t in array(&data, "threads")? {
            let tid = field(t, "id")?;
            threads.push(json!({"id":tid,"projectId":t["project_id"],"cwd":t["cwd"],"name":t["title"],"archived":t["archived"]}));
            saved.push(json!({"id":tid,"path":recovery.join(format!("{tid}.jsonl")),"sha256":data["hashes"][tid],"runtimeRoots":t["runtime_roots"]}));
        }
        converted["threads"] = json!(threads);
        converted["saved"] = json!(saved);
        self.release_server();
        let inventory = &data["inventory"];
        let manifest =
            json!({"inventory":inventory["entries"],"directories":inventory["directories"]});
        if let Some(roots) = data["roots"].as_object() {
            for (rid, target) in roots {
                resume_copy_tree(
                    &recovery.join("files").join(rid),
                    Path::new(target.as_str().ok_or("이전 복구 폴더 경로 오류")?),
                    &manifest_tree(&manifest, rid)?,
                    job,
                )?;
            }
        }
        self.undo_removed(id, &converted, job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_roots_accept_windows_separator_case_and_verbatim_equivalence() {
        let planned = vec![
            PathBuf::from(r"D:/Work\한글"),
            PathBuf::from(r"\\server\share/Extra"),
        ];
        let actual = vec![
            PathBuf::from(r"\\?\d:\work\한글"),
            PathBuf::from(r"\\?\UNC\SERVER\SHARE\extra"),
        ];
        assert!(same_runtime_roots(&planned, &actual));
    }

    #[test]
    fn runtime_roots_still_reject_additional_or_changed_roots() {
        let expected = vec![PathBuf::from(r"D:\Work"), PathBuf::from(r"D:\Extra")];
        assert!(!same_runtime_roots(
            &expected,
            &[PathBuf::from(r"D:\Work"), PathBuf::from(r"D:\Other")]
        ));
        assert!(!same_runtime_roots(
            &expected,
            &[
                PathBuf::from(r"D:\Work"),
                PathBuf::from(r"D:\Extra"),
                PathBuf::from(r"D:\Additional")
            ]
        ));
        assert!(!same_runtime_roots(&expected, &[PathBuf::from(r"D:\Work")]));
    }
}

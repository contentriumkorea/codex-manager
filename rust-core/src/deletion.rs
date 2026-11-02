use crate::desktop_state::CatalogDeletion;
use crate::{all_threads, clean_path, read_database_catalog, resolve_store, Adapter, Job};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
impl Adapter {
    pub fn keep_current(&mut self, id: &str, job: &Job) -> Result<Value, String> {
        self.guard()?;
        job.check()?;
        let record = self.journal.operation(id)?.ok_or("작업 기록이 없습니다.")?;
        if record["payload"]["kind"] == "rust-cross-store" {
            return Err("저장소 간 전송은 두 저장소를 연결한 뒤 전송 계속 또는 보존 후 안전 종료를 사용하세요.".into());
        }
        if record["payload"]["kind"] == "rust-legacy" {
            return Err("중단된 프로젝트 연결 정리는 계속 진행해 완료해야 합니다.".into());
        }
        if record["payload"]["home"] != json!(self.store.home)
            || ["completed", "cancelled"].contains(&record["state"].as_str().unwrap_or(""))
        {
            return Err("이 저장소의 미완료 작업을 선택하세요.".into());
        }
        if record["payload"]["kind"] == "rust-delete" {
            return self.finalize_delete(id, job);
        }
        self.journal
            .record(id, "cancelled", json!({"keptCurrent":true}))?;
        Ok(json!({"keptCurrent":true}))
    }
    pub(crate) fn reconcile_delete(&self, payload: &Value, job: &Job) -> Result<Value, String> {
        self.require_closed()?;
        let database = resolve_store(&self.store.home)?
            .database
            .ok_or("삭제 결과를 확인할 데이터베이스가 없습니다.")?;
        let native = read_database_catalog(&database)?;
        let known_threads = all_threads(&native)
            .into_iter()
            .filter_map(|t| t["id"].as_str().map(str::to_owned))
            .collect::<HashSet<_>>();
        let known_projects = native
            .projects
            .into_iter()
            .filter_map(|g| g.project["id"].as_str().map(str::to_owned))
            .collect::<HashSet<_>>();
        let threads = payload["threads"]
            .as_array()
            .ok_or("삭제 계획의 대화 목록이 없습니다.")?
            .iter()
            .map(|t| {
                t["id"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("삭제 대화 ID 오류")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let projects = payload["projects"]
            .as_array()
            .ok_or("삭제 계획의 프로젝트 목록이 없습니다.")?
            .iter()
            .map(|p| {
                p["id"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("삭제 프로젝트 ID 오류")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let deleted = threads
            .iter()
            .filter(|id| !known_threads.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let remaining = threads
            .iter()
            .filter(|id| known_threads.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let removed = projects
            .iter()
            .filter(|id| !known_projects.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let remaining_projects = projects
            .iter()
            .filter(|id| known_projects.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        // Cancellation stops further deletions. Reconcile already committed RPCs
        // even when the token is cancelled, so the desktop cannot show dead titles.
        job.phase_items("처리된 삭제 결과·목록 정리", deleted.len() as u64);
        if !deleted.is_empty() || !removed.is_empty() {
            let mut cache = CatalogDeletion::lock(&self.store.home)?;
            cache.remove(&deleted)?;
            cache.commit()?;
            self.sync(&[], &HashMap::new(), &removed, &deleted)?;
        }
        job.advance(deleted.len() as u64, "삭제 결과 확인", 0);
        Ok(
            json!({"deleted":deleted,"remainingThreads":remaining,"removedProjects":removed,"remainingProjects":remaining_projects,"complete":remaining.is_empty()&&remaining_projects.is_empty(),"noChanges":deleted.is_empty()&&removed.is_empty()}),
        )
    }
    pub fn finalize_delete(&mut self, id: &str, job: &Job) -> Result<Value, String> {
        self.guard()?;
        let record = self
            .journal
            .operation(id)?
            .ok_or("삭제 작업을 찾을 수 없습니다.")?;
        if ["completed", "cancelled"].contains(&record["state"].as_str().unwrap_or("")) {
            return Err("이미 종료된 삭제 작업입니다.".into());
        }
        let payload = &record["payload"];
        if payload["kind"] != "rust-delete"
            || payload["home"].as_str().is_none_or(|home| {
                clean_path(Path::new(home)).to_string_lossy().to_lowercase()
                    != clean_path(&self.store.home)
                        .to_string_lossy()
                        .to_lowercase()
            })
        {
            return Err("이 저장소의 삭제 작업이 아닙니다.".into());
        }
        let report = self.reconcile_delete(payload, job)?;
        self.journal.record(
            id,
            if report["complete"] == true {
                "completed"
            } else {
                "cancelled"
            },
            json!({"finalized":true,"report":report}),
        )?;
        Ok(report)
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use crate::Journal;
    use std::{fs, path::PathBuf};

    fn import_fixture(a: &mut Adapter, root: &Path, id: &str, parent: Option<&str>) {
        let source = parent.map(|p| json!({"subagent":{"thread_spawn":{"parent_thread_id":p,"depth":1,"agent_nickname":"fixture","agent_role":"default"}}})).unwrap_or(json!("cli"));
        let meta = json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":root,"runtime_workspace_roots":[root],"originator":"codex_cli_rs","cli_version":"0.0.0","source":source,"model_provider":"openai"}});
        let event = json!({"timestamp":"2026-11-02T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"fictional fixture","images":[],"local_images":[],"text_elements":[]}});
        let path = root.join(format!("{id}.jsonl"));
        fs::write(&path, format!("{meta}\n{event}\n")).unwrap();
        a.import_thread(&path, id, root, &[root.to_owned()], "", "Fixture")
            .unwrap();
    }

    #[test]
    #[ignore = "installed SDK with isolated home only"]
    fn in_flight_descendant_cannot_expand_batch_delete_authorization() {
        for protected in [false, true] {
            let root = PathBuf::from(std::env::var_os("CODEX_TEST_HOME").unwrap())
                .join(format!("in-flight-descendant-{protected}"));
            assert!(!root.exists());
            fs::create_dir_all(root.join("home")).unwrap();
            let mut a = Adapter::isolated(
                resolve_store(&root.join("home")).unwrap(),
                PathBuf::from(std::env::var_os("CODEX_TEST_BINARY").unwrap()),
                Journal::open(&root.join("manager/journal.sqlite")).unwrap(),
            )
            .unwrap();
            let p = "00000000-0000-4000-8000-000000000201";
            let u = "00000000-0000-4000-8000-000000000202";
            let x = "00000000-0000-4000-8000-000000000203";
            for id in [p, u] {
                import_fixture(&mut a, &root, id, None);
            }
            let mut calls = 0;
            let result = a.delete_checked(
                &[],
                &[p.into(), u.into()],
                false,
                &Job::new(),
                true,
                &mut |a| {
                    calls += 1;
                    if calls == 2 {
                        // Initial authorization and all recovery copies already exist;
                        // P's native RPC has completed. Insert X inside this same batch.
                        assert!(!all_threads(&a.snapshot()?).iter().any(|t| t["id"] == p));
                        import_fixture(a, &root, x, Some(u));
                        if protected {
                            fs::write(
                                a.store.home.join(".codex-global-state.json"),
                                json!({"projectless-thread-ids":[x]}).to_string(),
                            )
                            .unwrap();
                        }
                    }
                    Ok(())
                },
            );
            let error = result.expect_err("a new descendant must stop the in-flight batch");
            assert!(
                error.contains(if protected {
                    "명시적으로"
                } else {
                    "승인하지 않은"
                }),
                "{error}"
            );
            assert_eq!(calls, 2);
            let threads = all_threads(&a.snapshot().unwrap());
            assert!(!threads.iter().any(|t| t["id"] == p));
            assert!(threads.iter().any(|t| t["id"] == u));
            let child = threads
                .iter()
                .find(|t| t["id"] == x)
                .expect("unapproved X must survive");
            assert_eq!(child["parentThreadId"], u);
            assert_eq!(child["internal"], true);
            let history = a.journal.history().unwrap();
            let deletion = history
                .iter()
                .find(|r| r["payload"]["kind"] == "rust-delete")
                .unwrap();
            assert_eq!(deletion["state"], "needs_recovery");
            for key in ["threads", "saved"] {
                let records = deletion["payload"][key].as_array().unwrap();
                assert_eq!(records.len(), 2);
                assert!(records.iter().all(|t| t["id"] == p || t["id"] == u));
            }
            let saved = deletion["payload"]["saved"].as_array().unwrap();
            let recovery = Path::new(saved[0]["path"].as_str().unwrap())
                .parent()
                .unwrap();
            assert!(!recovery.join(format!("{x}.jsonl")).exists());
            let events = a.journal.events(deletion["id"].as_str().unwrap()).unwrap();
            let deleting: Vec<_> = events.iter().filter(|e| e["phase"] == "deleting").collect();
            assert_eq!(deleting.len(), 1);
            assert_eq!(deleting[0]["payload"]["threadId"], p);
            a.release_server();
        }
    }
}

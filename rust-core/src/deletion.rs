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

use crate::{all_threads, catalog_json, expand_thread_ids, Adapter, Catalog, Job};
use serde_json::{json, Value};
use std::collections::HashMap;

// Children of explicit members already participate in the operation dependency
// graph. Only display members outside that graph need an explicit connection.
fn missing_members(
    catalog: &Catalog,
    project: &str,
    selected: &[String],
) -> Result<Vec<Value>, String> {
    let group = catalog
        .projects
        .iter()
        .find(|g| g.project["id"] == project)
        .ok_or("프로젝트가 바뀌었습니다.")?;
    let all = all_threads(catalog);
    let seeds = all
        .iter()
        .filter(|t| selected.iter().any(|id| t["projectId"] == *id))
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let included = expand_thread_ids(&all, &seeds)?;
    Ok(group
        .threads
        .iter()
        .filter(|t| !included.iter().any(|id| t["id"] == *id))
        .cloned()
        .collect())
}

pub(crate) fn require_connections(catalog: &Catalog, projects: &[String]) -> Result<(), String> {
    for project in projects {
        if !missing_members(catalog, project, projects)?.is_empty() {
            return Err("폴더·원래 대화 기준으로 표시한 대화의 실제 연결 확인이 필요합니다. 목록을 새로 고침하고 이동·합치기·삭제를 다시 선택해 연결 확인을 진행하세요. 백업에는 이 대화도 포함됩니다.".into());
        }
    }
    Ok(())
}

impl Adapter {
    pub fn connection_plan(&self, projects: &[String]) -> Result<Value, String> {
        let catalog = self.snapshot()?;
        let mut groups = Vec::new();
        let mut selected = std::collections::HashSet::new();
        for project in projects {
            if !selected.insert(project) {
                return Err("중복 프로젝트 선택입니다.".into());
            }
            let threads = missing_members(&catalog, project, projects)?;
            if threads.is_empty() {
                continue;
            }
            let group = catalog
                .projects
                .iter()
                .find(|g| g.project["id"] == *project)
                .unwrap();
            if group.project["legacy"] == true {
                return Err("이전 형식 프로젝트의 연결을 먼저 갱신해야 합니다. 프로젝트 이름 변경을 한 번 완료한 뒤 다시 진행하세요. 표시만 된 대화는 그대로 보존됩니다.".into());
            }
            groups.push(json!({"project":group.project,"threads":threads}));
        }
        Ok(
            json!({"projects":projects,"groups":groups,"revision":catalog_json(catalog)["revision"]}),
        )
    }

    pub fn confirm_connections(&mut self, plan: &Value, job: &Job) -> Result<Value, String> {
        job.check()?;
        self.guard()?;
        let projects = plan["projects"]
            .as_array()
            .ok_or("연결 확인 계획 누락")?
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or("프로젝트 ID 오류"))
            .collect::<Result<Vec<_>, _>>()?;
        let current = self.connection_plan(&projects)?;
        let interrupt = self.isolated_checkpoint(plan, "connection-assigned");
        let mut expected = plan.clone();
        if interrupt {
            expected
                .as_object_mut()
                .unwrap()
                .remove("__testInterruptAt");
        }
        if current != expected {
            return Err("연결 확인 후 목록이 변경됐습니다. 새로 고침하고 다시 확인하세요.".into());
        }
        let groups = current["groups"].as_array().unwrap();
        // Validate every native source before the first write. Display values
        // never substitute for the recorded source in this check or in undo.
        for group in groups {
            for t in group["threads"].as_array().unwrap() {
                job.check()?;
                let read = self.read_thread(t["id"].as_str().ok_or("대화 ID 오류")?)?;
                if !t["projectId"].is_null()
                    || !read["projectId"].is_null()
                    || read["status"]["type"] == "active"
                {
                    return Err(
                        "대화의 실제 연결이나 실행 상태가 변경됐습니다. 다시 확인하세요.".into(),
                    );
                }
            }
        }
        let mut operations = Vec::new();
        for group in groups {
            job.check()?;
            let target = group["project"]["id"].as_str().unwrap();
            let threads = group["threads"].as_array().unwrap();
            let op = self.begin(json!({"kind":"rust-assign","displayConfirmation":true,"sourceRevision":current["revision"],"threads":threads,"target":target,"resources":threads.iter().map(|t|format!("thread:{}",t["id"].as_str().unwrap())).chain(std::iter::once(target.to_owned())).collect::<Vec<_>>()}))?;
            let result = (|| {
                let mut assignments = HashMap::new();
                for t in threads {
                    job.check()?;
                    let id = t["id"].as_str().unwrap();
                    if !self.read_thread(id)?["projectId"].is_null() {
                        return Err("연결 중 대화 소속이 변경됐습니다.".into());
                    }
                    self.journal
                        .record(&op, "relinking", json!({"threadId":id}))?;
                    self.assign(id, Some(target))?;
                    if interrupt {
                        return Err("isolated connection interruption".into());
                    }
                    assignments.insert(id.to_owned(), Some(target.to_owned()));
                }
                self.sync(
                    std::slice::from_ref(&group["project"]),
                    &assignments,
                    &[],
                    &[],
                )
            })();
            self.finish(&op, result)?;
            operations.push(op);
        }
        Ok(json!({"operationIds":operations}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joint_explicit_project_selection_keeps_cross_project_dependencies() {
        let catalog = crate::organize_catalog(
            vec![json!({"id":"a"}), json!({"id":"b"})],
            vec![
                json!({"id":"parent","projectId":"a"}),
                json!({"id":"child","projectId":"b","parentThreadId":"parent"}),
            ],
        )
        .unwrap();
        assert!(require_connections(&catalog, &["a".into()]).is_err());
        require_connections(&catalog, &["a".into(), "b".into()]).unwrap();
    }
}

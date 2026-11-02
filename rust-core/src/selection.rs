use serde_json::Value;
use std::collections::HashSet;
pub fn expand_thread_ids(threads: &[Value], requested: &[String]) -> Result<Vec<String>, String> {
    let mut ids: HashSet<_> = requested.iter().cloned().collect();
    if ids.len() != requested.len() {
        return Err("중복 대화 선택입니다.".into());
    }
    let mut projects = HashSet::new();
    for id in requested {
        let thread = threads
            .iter()
            .find(|t| t["id"] == *id)
            .ok_or("선택한 대화가 바뀌었습니다.")?;
        if let Some(pid) = thread["projectId"].as_str() {
            projects.insert(pid.to_owned());
        }
    }
    loop {
        let mut changed = false;
        for thread in threads {
            if ["parentThreadId", "attachedThreadParentId"]
                .iter()
                .any(|key| thread[*key].as_str().is_some_and(|id| ids.contains(id)))
            {
                let id = thread["id"].as_str().ok_or("자식 대화 ID 오류")?;
                if ids.contains(id) {
                    continue;
                }
                if matches!(
                    thread["membershipReason"].as_str(),
                    Some(
                        "projectless"
                            | "other_host"
                            | "unresolved_assignment"
                            | "missing_project"
                            | "ambiguous"
                    )
                ) {
                    return Err("프로젝트 없음·다른 기기·불명확한 연결의 자식 대화가 있습니다. 내부 기록을 포함해 연결을 확인하고 필요한 대화를 명시적으로 함께 선택하세요.".into());
                }
                if thread["projectId"]
                    .as_str()
                    .is_some_and(|pid| !projects.contains(pid))
                {
                    return Err("다른 프로젝트의 자식 대화가 연결돼 있습니다. 해당 대화를 명시적으로 함께 선택하세요.".into());
                }
                changed |= ids.insert(id.to_owned());
            }
        }
        if !changed {
            break;
        }
    }
    let mut result = ids.into_iter().collect::<Vec<_>>();
    result.sort();
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn expands_unassigned_children_but_blocks_cross_project_children() {
        let mut threads = vec![
            json!({"id":"p","projectId":"A"}),
            json!({"id":"c","projectId":null,"parentThreadId":"p"}),
        ];
        assert_eq!(expand_thread_ids(&threads, &["p".into()]).unwrap().len(), 2);
        threads[1]["projectId"] = json!("B");
        assert!(expand_thread_ids(&threads, &["p".into()]).is_err());
        assert_eq!(
            expand_thread_ids(&threads, &["p".into(), "c".into()])
                .unwrap()
                .len(),
            2
        );
    }
}

#[cfg(test)]
mod protected_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn protected_child_links_require_explicit_selection_instead_of_automatic_materialization() {
        for reason in [
            "projectless",
            "other_host",
            "ambiguous",
            "missing_project",
            "unresolved_assignment",
        ] {
            let threads = vec![
                json!({"id":"p","projectId":"A"}),
                json!({"id":"child","projectId":null,"parentThreadId":"p","membershipReason":reason,"internal":true}),
            ];
            assert!(expand_thread_ids(&threads, &["p".into()]).is_err());
            assert_eq!(
                expand_thread_ids(&threads, &["p".into(), "child".into()])
                    .unwrap()
                    .len(),
                2
            );
        }
    }
}

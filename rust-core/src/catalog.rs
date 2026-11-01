use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct ProjectGroup {
    pub project: Value,
    pub threads: Vec<Value>,
}

pub struct Catalog {
    pub projects: Vec<ProjectGroup>,
    pub unassigned: Vec<Value>,
    pub unresolved: Vec<Value>,
}
pub(crate) enum LocalAlias {
    Resolved(String),
    Missing,
    Ambiguous,
}
pub(crate) fn local_alias(
    maps: Option<&Value>,
    host: &str,
    alias: &str,
    known: &HashSet<String>,
) -> LocalAlias {
    if let Some(id) = maps
        .and_then(|m| m.get(host))
        .and_then(|m| m.get(alias))
        .and_then(Value::as_str)
    {
        return LocalAlias::Resolved(id.to_owned());
    }
    let mut candidates = HashSet::new();
    if let Some(hosts) = maps.and_then(Value::as_object) {
        for (key, value) in hosts {
            if key.starts_with("local:") {
                if let Some(id) = value.get(alias).and_then(Value::as_str) {
                    if known.contains(id) {
                        candidates.insert(id.to_owned());
                    }
                }
            }
        }
    }
    match candidates.len() {
        0 => LocalAlias::Missing,
        1 => LocalAlias::Resolved(candidates.into_iter().next().unwrap()),
        _ => LocalAlias::Ambiguous,
    }
}

pub fn merge_legacy_projects(
    projects: &mut Vec<Value>,
    state: &Value,
    host: &str,
) -> Result<(), String> {
    if !state.is_object() {
        return Err("invalid desktop state".into());
    }
    let mut native = HashSet::new();
    for p in projects.iter() {
        let id = p
            .get("id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or("invalid project id")?;
        if !native.insert(id.to_owned()) {
            return Err("duplicate project id".into());
        }
    }
    let legacy = state.get("local-projects");
    let maps = state.get("app-server-project-id-by-legacy-project-id-by-host");
    if legacy.is_some_and(|v| !v.is_object()) || maps.is_some_and(|v| !v.is_object()) {
        return Err("invalid desktop project mappings".into());
    }
    let mut added = Vec::new();
    let mut added_ids = HashSet::new();
    if let Some(entries) = legacy.and_then(Value::as_object) {
        for (alias, project) in entries {
            if alias.is_empty() || !project.is_object() {
                return Err("invalid legacy project".into());
            }
            let canonical = match local_alias(maps, host, alias, &native) {
                LocalAlias::Resolved(id) => id,
                LocalAlias::Missing => alias.to_owned(),
                LocalAlias::Ambiguous => continue,
            };
            if canonical.is_empty() {
                return Err("invalid legacy project id".into());
            }
            if native.contains(&canonical) {
                continue;
            }
            if !added_ids.insert(canonical.clone()) {
                return Err("ambiguous legacy projects".into());
            }
            let name = project.get("name").and_then(Value::as_str).unwrap_or(alias);
            let roots = match project.get("rootPaths") {
                None => Vec::new(),
                Some(Value::Array(paths)) => paths
                    .iter()
                    .map(|p| {
                        p.as_str()
                            .map(|p| serde_json::json!({"path":p}))
                            .ok_or("invalid legacy project root")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => return Err("invalid legacy project roots".into()),
            };
            added.push(serde_json::json!({"id":canonical,"name":name,"roots":roots,"legacy":true}));
        }
    }
    projects.extend(added);
    Ok(())
}

pub fn merge_legacy_assignments(
    projects: &[Value],
    threads: &mut [Value],
    state: &Value,
    host: &str,
) -> Result<(), String> {
    if !state.is_object() {
        return Err("invalid desktop state".into());
    }
    let mut project_ids = HashSet::new();
    for project in projects {
        let id = project
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("invalid project id")?;
        if !project_ids.insert(id.to_owned()) {
            return Err("duplicate project id".into());
        }
    }
    let assignments = state.get("thread-project-assignments");
    let mappings = state.get("app-server-project-id-by-legacy-project-id-by-host");
    if assignments.is_some_and(|v| !v.is_object()) || mappings.is_some_and(|v| !v.is_object()) {
        return Err("invalid desktop project mappings".into());
    }
    let mut updated = threads.to_vec();
    for thread in &mut updated {
        let id = thread
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("invalid thread id")?;
        match thread.get("projectId") {
            Some(Value::String(pid)) if !pid.is_empty() => continue,
            Some(Value::Null) => {}
            _ => return Err("unknown project membership".into()),
        }
        let Some(assignment) = assignments.and_then(|a| a.get(id)) else {
            continue;
        };
        if assignment.is_null() {
            continue;
        }
        let kind = assignment
            .get("projectKind")
            .and_then(Value::as_str)
            .ok_or("invalid legacy project kind")?;
        let legacy = assignment
            .get("projectId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("invalid legacy project id")?;
        if kind != "local" {
            thread["legacyAssignmentUnresolved"] = Value::Bool(true);
            continue;
        }
        let chosen = match local_alias(mappings, host, legacy, &project_ids) {
            LocalAlias::Resolved(id) => Some(id),
            LocalAlias::Missing if project_ids.contains(legacy) => Some(legacy.to_owned()),
            _ => None,
        };
        if let Some(chosen) = chosen {
            thread["projectId"] = Value::String(chosen);
        } else {
            thread["legacyAssignmentUnresolved"] = Value::Bool(true);
        }
    }
    threads.clone_from_slice(&updated);
    Ok(())
}

pub fn organize_catalog(projects: Vec<Value>, threads: Vec<Value>) -> Result<Catalog, String> {
    let mut locations = HashMap::new();
    let mut grouped = Vec::new();
    for project in projects {
        let id = project
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("invalid project id")?
            .to_owned();
        if locations.insert(id, grouped.len()).is_some() {
            return Err("duplicate project id".into());
        }
        grouped.push(ProjectGroup {
            project,
            threads: Vec::new(),
        });
    }
    let mut unassigned = Vec::new();
    let mut unresolved = Vec::new();
    let mut seen = HashSet::new();
    for thread in threads {
        let id = thread
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("invalid thread id")?;
        if !seen.insert(id.to_owned()) {
            return Err("duplicate thread id".into());
        }
        if thread
            .get("legacyAssignmentUnresolved")
            .and_then(Value::as_bool)
            == Some(true)
        {
            unresolved.push(thread);
            continue;
        }
        match thread.get("projectId") {
            Some(Value::Null) => unassigned.push(thread),
            Some(Value::String(id)) if !id.is_empty() => {
                if let Some(index) = locations.get(id) {
                    grouped[*index].threads.push(thread);
                } else {
                    unresolved.push(thread);
                }
            }
            _ => return Err("unknown project membership".into()),
        }
    }
    Ok(Catalog {
        projects: grouped,
        unassigned,
        unresolved,
    })
}

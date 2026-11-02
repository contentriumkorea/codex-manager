//! Display-only membership. Never overwrite authoritative projectId.
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn path_key(path: &str) -> String {
    let mut path = path.replace('/', "\\").to_lowercase();
    if let Some(rest) = path.strip_prefix("\\\\?\\unc\\") {
        path = format!("\\\\{rest}");
    } else if let Some(rest) = path.strip_prefix("\\\\?\\") {
        path = rest.to_owned();
    }
    let unc = path.starts_with("\\\\");
    let mut parts = Vec::new();
    for part in path.split('\\') {
        match part {
            "" | "." => {}
            ".." if parts.len() > if unc { 2 } else { 1 } => {
                parts.pop();
            }
            ".." => {}
            _ => parts.push(part),
        }
    }
    format!(
        "{}{}",
        if unc {
            "\\\\"
        } else if path.starts_with('\\') {
            "\\"
        } else {
            ""
        },
        parts.join("\\")
    )
}

pub(crate) fn annotate(projects: &[Value], threads: &mut [Value]) -> Result<(), String> {
    let valid: HashSet<_> = projects.iter().filter_map(|p| p["id"].as_str()).collect();
    let roots: Vec<_> =
        projects
            .iter()
            .flat_map(|p| {
                p["roots"].as_array().into_iter().flatten().filter_map(|r| {
                    Some((path_key(r["path"].as_str()?), p["id"].as_str()?.to_owned()))
                })
            })
            .filter(|(r, _)| !r.is_empty())
            .collect();
    let mut indexes = HashMap::new();
    for (i, t) in threads.iter().enumerate() {
        let id = t["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("invalid thread id")?;
        if indexes.insert(id.to_owned(), i).is_some() {
            return Err("duplicate thread id".into());
        }
    }
    let parents: Vec<_> = threads
        .iter()
        .map(|t| {
            t["parentThreadId"]
                .as_str()
                .and_then(|id| indexes.get(id))
                .copied()
        })
        .collect();
    let mut resolved = vec![false; threads.len()];
    for (i, t) in threads.iter_mut().enumerate() {
        let (project, reason) = match t.get("projectId") {
            Some(Value::String(id)) if !id.is_empty() => {
                if valid.contains(id.as_str()) {
                    (Some(id.clone()), "explicit")
                } else {
                    (None, "missing_project")
                }
            }
            Some(Value::Null) => {
                if t["legacyAssignmentUnresolved"] == true {
                    (None, "unresolved_assignment")
                } else if t["explicitProjectless"] == true {
                    (None, "projectless")
                } else if t["otherHost"] == true {
                    (None, "other_host")
                } else if t["internal"] == true && parents[i].is_some() {
                    (None, "internal_parent")
                } else {
                    let cwd = path_key(t["cwd"].as_str().unwrap_or(""));
                    let matches: Vec<_> = roots
                        .iter()
                        .filter(|(root, _)| cwd == *root || cwd.starts_with(&format!("{root}\\")))
                        .collect();
                    if let Some(length) = matches.iter().map(|(r, _)| r.len()).max() {
                        let candidates: HashSet<_> = matches
                            .iter()
                            .filter(|(r, _)| r.len() == length)
                            .map(|(_, id)| id.as_str())
                            .collect();
                        if candidates.len() == 1 {
                            (
                                Some(candidates.into_iter().next().unwrap().to_owned()),
                                "folder",
                            )
                        } else {
                            (None, "ambiguous")
                        }
                    } else {
                        (None, "outside")
                    }
                }
            }
            _ => return Err("unknown project membership".into()),
        };
        t["displayProjectId"] = json!(project);
        t["membershipReason"] = json!(reason);
        resolved[i] = !matches!(reason, "folder" | "outside" | "internal_parent");
    }
    for start in 0..threads.len() {
        let mut chain = Vec::new();
        let mut positions = HashMap::new();
        let mut current = Some(start);
        while let Some(i) = current {
            if resolved[i] {
                break;
            }
            if let Some(&position) = positions.get(&i) {
                for &cyclic in &chain[position..] {
                    let t: &mut Value = &mut threads[cyclic];
                    t["displayProjectId"] = Value::Null;
                    t["membershipReason"] = json!("ambiguous");
                    resolved[cyclic] = true;
                }
                break;
            }
            positions.insert(i, chain.len());
            chain.push(i);
            current = parents[i];
        }
        for i in chain.into_iter().rev() {
            if resolved[i] {
                continue;
            }
            if let Some(parent) = parents[i]
                .and_then(|p| threads[p]["displayProjectId"].as_str())
                .map(str::to_owned)
            {
                let t = &mut threads[i];
                if t["internal"] != true
                    && t["membershipReason"] == "folder"
                    && t["displayProjectId"] != parent
                {
                    t["displayProjectId"] = Value::Null;
                    t["membershipReason"] = json!("ambiguous");
                } else if t["displayProjectId"].is_null()
                    && matches!(
                        t["membershipReason"].as_str(),
                        Some("outside" | "internal_parent")
                    )
                {
                    t["displayProjectId"] = json!(parent);
                    t["membershipReason"] = json!("parent");
                }
            }
            resolved[i] = true;
        }
    }
    Ok(())
}

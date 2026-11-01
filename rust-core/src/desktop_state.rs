use crate::{ensure_plain, write_json_new};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

pub fn sync_state(
    home: &Path,
    projects: &[Value],
    assignments: &HashMap<String, Option<String>>,
    removed: &[String],
    deleted: &[String],
    backup: &Path,
) -> Result<(), String> {
    let path = home.join(".codex-global-state.json");
    if !path.is_file() {
        return Ok(());
    }
    ensure_plain(&path)?;
    let before = fs::read(&path).map_err(|e| e.to_string())?;
    let mut state: Value = serde_json::from_slice(&before).map_err(|e| e.to_string())?;
    if !state.is_object() {
        return Err("Codex 앱 설정 형식이 바뀌었습니다.".into());
    }
    let host = format!("local:{}", home.display());
    for key in [
        "local-projects",
        "app-server-project-id-by-legacy-project-id-by-host",
        "thread-project-assignments",
    ] {
        if state.get(key).is_none() {
            state[key] = json!({});
        }
        if !state[key].is_object() {
            return Err("Codex 앱 프로젝트 설정 형식이 바뀌었습니다.".into());
        }
    }
    if state["app-server-project-id-by-legacy-project-id-by-host"]
        .get(&host)
        .is_none()
    {
        state["app-server-project-id-by-legacy-project-id-by-host"][&host] = json!({});
    }
    let mut mapping: HashMap<String, String> = serde_json::from_value(
        state["app-server-project-id-by-legacy-project-id-by-host"][&host].clone(),
    )
    .map_err(|e| e.to_string())?;
    let mut known: HashSet<String> = removed
        .iter()
        .cloned()
        .chain(
            projects
                .iter()
                .filter_map(|p| p["id"].as_str().map(str::to_owned)),
        )
        .collect();
    if let Some(database) = crate::resolve_store(home)?.database {
        known.extend(
            crate::read_database_catalog(&database)?
                .projects
                .into_iter()
                .filter_map(|g| g.project["id"].as_str().map(str::to_owned)),
        );
    }
    let maps = &state["app-server-project-id-by-legacy-project-id-by-host"];
    let mut keys = state["local-projects"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    for (key, value) in maps.as_object().unwrap() {
        if key.starts_with("local:") {
            if let Some(map) = value.as_object() {
                keys.extend(map.keys().cloned());
            }
        }
    }
    let mut aliases: HashSet<String> = removed.iter().cloned().collect();
    for alias in keys {
        if let crate::catalog::LocalAlias::Resolved(id) =
            crate::catalog::local_alias(Some(maps), &host, &alias, &known)
        {
            if removed.contains(&id) {
                aliases.insert(alias);
            } else {
                mapping.entry(alias).or_insert(id);
            }
        }
    }
    for (key, value) in state["app-server-project-id-by-legacy-project-id-by-host"]
        .as_object_mut()
        .unwrap()
    {
        if key.starts_with("local:") {
            if let Some(map) = value.as_object_mut() {
                map.retain(|_, id| {
                    !id.as_str()
                        .is_some_and(|id| removed.iter().any(|removed| removed == id))
                });
            }
        }
    }
    for alias in &aliases {
        mapping.remove(alias);
        state["local-projects"]
            .as_object_mut()
            .unwrap()
            .remove(alias);
    }
    let mut ui_ids = HashMap::new();
    for p in projects {
        let id = p["id"].as_str().ok_or("프로젝트 ID 누락")?;
        let alias = p["legacyAlias"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                mapping
                    .iter()
                    .filter(|(_, v)| *v == id)
                    .map(|(k, _)| k.clone())
                    .min()
            })
            .unwrap_or_else(|| id.to_owned());
        mapping.insert(alias.clone(), id.to_owned());
        ui_ids.insert(id.to_owned(), alias.clone());
        let mut entry = state["local-projects"]
            .get(&alias)
            .cloned()
            .unwrap_or(json!({}));
        if !entry.is_object() {
            return Err("Codex 프로젝트 설정 오류".into());
        }
        entry["id"] = json!(alias);
        entry["name"] = p["name"].clone();
        entry["rootPaths"] = json!(p["roots"]
            .as_array()
            .ok_or("루트 목록 오류")?
            .iter()
            .map(|r| r["path"].clone())
            .collect::<Vec<_>>());
        entry["updatedAt"] = json!(chrono::Utc::now().timestamp_millis());
        if entry.get("createdAt").is_none() {
            entry["createdAt"] = entry["updatedAt"].clone();
        }
        state["local-projects"][&alias] = entry;
    }
    state["app-server-project-id-by-legacy-project-id-by-host"][&host] = json!(mapping);
    for (tid, pid) in assignments {
        if let Some(pid) = pid {
            state["thread-project-assignments"][tid] =
                json!({"projectKind":"local","projectId":ui_ids.get(pid).unwrap_or(pid)});
        } else {
            state["thread-project-assignments"]
                .as_object_mut()
                .unwrap()
                .remove(tid);
        }
        if let Some(map) = state["thread-workspace-root-hints"].as_object_mut() {
            map.remove(tid);
        }
    }
    for tid in deleted {
        for key in [
            "thread-project-assignments",
            "thread-project-membership-host-ids",
            "thread-projectless-output-directories",
            "queued-follow-ups",
        ] {
            if let Some(map) = state[key].as_object_mut() {
                map.remove(tid);
            }
        }
        if let Some(ids) = state["projectless-thread-ids"].as_array_mut() {
            ids.retain(|x| x != tid);
        }
        if let Some(map) =
            state["electron-thread-read-state-v1"]["unreadByIdentity"].as_object_mut()
        {
            map.retain(|key, _| !key.contains(tid));
        }
    }
    if let Some(map) = state["sidebar-project-thread-orders"].as_object_mut() {
        for ids in map.values_mut().filter_map(Value::as_array_mut) {
            ids.retain(|x| {
                !assignments.contains_key(x.as_str().unwrap_or(""))
                    && !deleted.iter().any(|d| x == d)
            });
        }
    }
    for key in ["project-order", "pinned-project-ids"] {
        if let Some(ids) = state[key].as_array_mut() {
            ids.retain(|x| !aliases.contains(x.as_str().unwrap_or("")));
        }
    }
    for key in ["sidebar-project-thread-orders", "project-appearances"] {
        if let Some(map) = state[key].as_object_mut() {
            map.retain(|key, _| !aliases.contains(key));
        }
    }
    if aliases.contains(
        state["selected-project"]["projectId"]
            .as_str()
            .unwrap_or(""),
    ) {
        state.as_object_mut().unwrap().remove("selected-project");
    }
    fs::create_dir_all(backup).map_err(|e| e.to_string())?;
    let saved = backup.join(format!("desktop-state-{}.json", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(saved)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&before)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    let temporary = home.join(format!(".manager-{}.partial", uuid::Uuid::new_v4()));
    write_json_new(&temporary, &state)?;
    if fs::read(&path).map_err(|e| e.to_string())? != before {
        return Err("Codex 설정이 다른 프로세스에서 변경됐습니다. 복구 내역을 확인하세요.".into());
    }
    fs::rename(temporary, path).map_err(|e| e.to_string())
}

pub struct CatalogDeletion {
    databases: Vec<Connection>,
}
impl CatalogDeletion {
    pub fn lock(home: &Path) -> Result<Self, String> {
        let mut guard = Self {
            databases: Vec::new(),
        };
        for name in ["codex.db", "codex-dev.db"] {
            let path = home.join("sqlite").join(name);
            if !path.exists() {
                continue;
            }
            ensure_plain(&path)?;
            let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(|e| e.to_string())?;
            db.busy_timeout(std::time::Duration::from_secs(2))
                .map_err(|e| e.to_string())?;
            db.execute_batch("BEGIN IMMEDIATE")
                .map_err(|e| e.to_string())?;
            let checks = [
                (
                    "local_thread_catalog",
                    vec!["host_id", "thread_id", "source_kind", "missing_candidate"],
                ),
                ("local_thread_catalog_hosts", vec!["host_id", "host_kind"]),
                (
                    "local_thread_catalog_sync_state",
                    vec!["host_id", "observation_sequence"],
                ),
                (
                    "local_thread_catalog_metadata",
                    vec!["id", "catalog_revision"],
                ),
                ("local_thread_catalog_scan_checkpoints", vec!["host_id"]),
                (
                    "local_thread_catalog_scan_entries",
                    vec!["host_id", "thread_id", "removed"],
                ),
            ];
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='local_thread_catalog')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !exists {
                db.execute_batch("ROLLBACK").map_err(|e| e.to_string())?;
                continue;
            }
            for (table, fields) in checks {
                let mut q = db
                    .prepare(&format!("PRAGMA table_info({table})"))
                    .map_err(|e| e.to_string())?;
                let columns = q
                    .query_map([], |r| r.get::<_, String>(1))
                    .map_err(|e| e.to_string())?
                    .collect::<rusqlite::Result<HashSet<_>>>()
                    .map_err(|e| e.to_string())?;
                if !fields.iter().all(|f| columns.contains(*f)) {
                    return Err("Codex 대화 목록 형식이 변경되어 삭제를 차단했습니다.".into());
                }
            }
            let host: Option<String> = db
                .query_row(
                    "SELECT host_kind FROM local_thread_catalog_hosts WHERE host_id='local'",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if host.is_none() {
                db.execute_batch("ROLLBACK").map_err(|e| e.to_string())?;
                continue;
            }
            if host.as_deref() != Some("local") {
                return Err("로컬 대화 목록을 확인할 수 없습니다.".into());
            }
            for sql in [
                "SELECT count(*) FROM local_thread_catalog_sync_state WHERE host_id='local'",
                "SELECT count(*) FROM local_thread_catalog_metadata WHERE id=1",
            ] {
                let n: i64 = db
                    .query_row(sql, [], |r| r.get(0))
                    .map_err(|e| e.to_string())?;
                if n != 1 {
                    return Err("Codex 대화 목록 갱신 정보를 확인할 수 없습니다.".into());
                }
            }
            guard.databases.push(db);
        }
        Ok(guard)
    }
    pub fn remove(&mut self, ids: &[String]) -> Result<(), String> {
        for db in &mut self.databases {
            let checkpoint:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM local_thread_catalog_scan_checkpoints WHERE host_id='local')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
            let mut changed = false;
            for id in ids {
                let row:Option<(String,i64)>=db.query_row("SELECT source_kind,missing_candidate FROM local_thread_catalog WHERE host_id='local' AND thread_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
                if row.as_ref().is_some_and(|(kind, _)| kind == "chatgpt") {
                    continue;
                }
                db.execute("UPDATE local_thread_catalog_sync_state SET observation_sequence=observation_sequence+1 WHERE host_id='local'",[]).map_err(|e|e.to_string())?;
                if checkpoint {
                    db.execute("INSERT INTO local_thread_catalog_scan_entries(host_id,thread_id,removed) VALUES('local',?1,1) ON CONFLICT(host_id,thread_id) DO UPDATE SET removed=1",[id]).map_err(|e|e.to_string())?;
                }
                db.execute(
                    "DELETE FROM local_thread_catalog WHERE host_id='local' AND thread_id=?1",
                    [id],
                )
                .map_err(|e| e.to_string())?;
                changed |= row.is_some_and(|(_, missing)| missing == 0);
            }
            if changed {
                db.execute("UPDATE local_thread_catalog_metadata SET catalog_revision=catalog_revision+1 WHERE id=1",[]).map_err(|e|e.to_string())?;
            }
        }
        Ok(())
    }
    pub fn commit(&mut self) -> Result<(), String> {
        for db in &self.databases {
            db.execute_batch("COMMIT").map_err(|e| e.to_string())?;
        }
        self.databases.clear();
        Ok(())
    }
}
impl Drop for CatalogDeletion {
    fn drop(&mut self) {
        for db in &self.databases {
            let _ = db.execute_batch("ROLLBACK");
        }
    }
}
use rusqlite::OptionalExtension;
#[cfg(test)]
mod state_tests {
    use super::*;
    fn fixture(state: &Value) -> std::path::PathBuf {
        let home = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-artifacts")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&home).unwrap();
        write_json_new(&home.join(".codex-global-state.json"), state).unwrap();
        home
    }
    #[test]
    fn removed_native_project_does_not_reappear_from_a_previous_local_host() {
        let state = json!({"local-projects":{"alias":{"id":"alias","name":"A","rootPaths":[]}},"app-server-project-id-by-legacy-project-id-by-host":{"local:old-home":{"alias":"native"},"remote:other":{"alias":"native"}},"thread-project-assignments":{}});
        let home = fixture(&state);
        sync_state(
            &home,
            &[],
            &HashMap::new(),
            &["native".into()],
            &[],
            &home.join("recovery"),
        )
        .unwrap();
        let state: Value =
            serde_json::from_slice(&fs::read(home.join(".codex-global-state.json")).unwrap())
                .unwrap();
        assert!(state["local-projects"].get("alias").is_none());
        assert!(
            state["app-server-project-id-by-legacy-project-id-by-host"]["local:old-home"]
                .get("alias")
                .is_none()
        );
        assert_eq!(
            state["app-server-project-id-by-legacy-project-id-by-host"]["remote:other"]["alias"],
            "native"
        );
        let mut projects = Vec::new();
        crate::merge_legacy_projects(&mut projects, &state, &format!("local:{}", home.display()))
            .unwrap();
        assert!(projects.is_empty());
    }
    #[test]
    fn conflicting_alias_for_a_surviving_project_is_not_deleted() {
        let state = json!({"local-projects":{"alias":{"id":"alias","name":"Keep","rootPaths":[]}},"app-server-project-id-by-legacy-project-id-by-host":{"local:old":{"alias":"removed"},"local:other":{"alias":"keep"}},"thread-project-assignments":{}});
        let home = fixture(&state);
        sync_state(
            &home,
            &[json!({"id":"keep","name":"Keep","roots":[]})],
            &HashMap::new(),
            &["removed".into()],
            &[],
            &home.join("recovery"),
        )
        .unwrap();
        let state: Value =
            serde_json::from_slice(&fs::read(home.join(".codex-global-state.json")).unwrap())
                .unwrap();
        assert!(state["local-projects"].get("alias").is_some());
        assert_eq!(
            state["app-server-project-id-by-legacy-project-id-by-host"]["local:other"]["alias"],
            "keep"
        );
    }
}

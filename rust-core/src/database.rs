use crate::{merge_legacy_assignments, merge_legacy_projects, organize_catalog, Catalog};
use rusqlite::{
    backup::{Backup, StepResult},
    Connection, OpenFlags,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

fn columns(db: &Connection, table: &str) -> Result<HashSet<String>, String> {
    let mut stmt = db
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
}

fn require(columns: &HashSet<String>, fields: &[&str]) -> Result<(), String> {
    if fields.iter().all(|f| columns.contains(*f)) {
        Ok(())
    } else {
        Err("unsupported Codex database schema".into())
    }
}

fn optional<'a>(cols: &HashSet<String>, col: &'a str, default: &'a str) -> &'a str {
    if cols.contains(col) {
        col
    } else {
        default
    }
}

fn read_raw(path: &Path) -> Result<(Vec<Value>, Vec<Value>), String> {
    if !path.is_absolute() || !path.is_file() {
        return Err("invalid database path".into());
    }
    for component in path.ancestors() {
        let meta = std::fs::symlink_metadata(component).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            return Err("linked database path".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("linked database path".into());
            }
        }
    }
    let source = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    source
        .busy_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    let mut db = Connection::open_in_memory().map_err(|e| e.to_string())?;
    {
        let backup = Backup::new(&source, &mut db).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match backup.step(256).map_err(|e| e.to_string())? {
                StepResult::Done => break,
                StepResult::More => {}
                StepResult::Busy | StepResult::Locked => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => return Err("unexpected SQLite backup status".into()),
            }
            if Instant::now() > deadline {
                return Err("database snapshot timeout".into());
            }
        }
    }
    let projects_cols = columns(&db, "projects")?;
    let roots_cols = columns(&db, "project_roots")?;
    let thread_cols = columns(&db, "threads")?;
    require(&projects_cols, &["id", "name"])?;
    require(&roots_cols, &["project_id", "path"])?;
    require(&thread_cols, &["id", "project_id", "cwd"])?;
    let order = optional(&projects_cols, "position", "rowid");
    let mut stmt = db
        .prepare(&format!("SELECT id,name FROM projects ORDER BY {order}"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut projects = Vec::new();
    let root_order = optional(&roots_cols, "position", "rowid");
    for row in rows {
        let (id, name) = row.map_err(|e| e.to_string())?;
        let mut roots_stmt = db
            .prepare(&format!(
                "SELECT path FROM project_roots WHERE project_id=?1 ORDER BY {root_order}"
            ))
            .map_err(|e| e.to_string())?;
        let roots = roots_stmt
            .query_map([&id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| json!({"path":p}))
            .collect::<Vec<_>>();
        projects.push(json!({"id":id,"name":name,"roots":roots}));
    }
    let mut parents = HashMap::new();
    let edge_cols = columns(&db, "thread_spawn_edges")?;
    if !edge_cols.is_empty() {
        require(&edge_cols, &["parent_thread_id", "child_thread_id"])?;
        let mut edges = db
            .prepare("SELECT parent_thread_id,child_thread_id FROM thread_spawn_edges")
            .map_err(|e| e.to_string())?;
        let rows = edges
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (p, c) = row.map_err(|e| e.to_string())?;
            if parents.insert(c, p).is_some() {
                return Err("ambiguous thread parent".into());
            }
        }
    }
    let attachment_cols = columns(&db, "thread_attachments")?;
    let mut attachment_counts = HashMap::new();
    if !attachment_cols.is_empty() {
        require(&attachment_cols, &["thread_id"])?;
        let mut att = db
            .prepare("SELECT thread_id,count(*) FROM thread_attachments GROUP BY thread_id")
            .map_err(|e| e.to_string())?;
        let rows = att
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, count) = row.map_err(|e| e.to_string())?;
            attachment_counts.insert(id, count);
        }
    }
    let fields = [
        "name",
        "title",
        "rollout_path",
        "archived",
        "has_user_event",
        "attached_thread_parent_id",
        "updated_at",
        "source",
        "thread_source",
    ];
    let expression = fields
        .iter()
        .map(|f| optional(&thread_cols, f, "NULL"))
        .collect::<Vec<_>>()
        .join(",");
    let mut stmt = db
        .prepare(&format!(
            "SELECT id,project_id,cwd,{expression} FROM threads"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<String>>(11)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut threads = Vec::<Value>::new();
    for row in rows {
        let (
            id,
            project,
            cwd,
            name,
            title,
            path,
            archived,
            has_event,
            attached,
            updated,
            source,
            thread_source,
        ) = row.map_err(|e| e.to_string())?;
        let count = if attachment_cols.is_empty() {
            None
        } else {
            Some(*attachment_counts.get(&id).unwrap_or(&0))
        };
        let source: Value = source
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        let parent = parents
            .get(&id)
            .map(String::as_str)
            .or_else(|| source["subagent"]["thread_spawn"]["parent_thread_id"].as_str());
        let internal = source["subagent"].is_object()
            || matches!(
                thread_source.as_deref(),
                Some("subagent" | "guardian_review")
            );
        threads.push(json!({"id":id,"projectId":project,"cwd":cwd,"name":name.filter(|x|!x.is_empty()).or(title),
            "path":path,"archived":archived.map(|n|n!=0),"hasUserEvent":has_event.map(|n|n!=0),
            "parentThreadId":parent,"attachedThreadParentId":attached,"attachmentCount":count,"updatedAt":updated,"internal":internal}));
    }
    Ok((projects, threads))
}

pub fn read_database_catalog(path: &Path) -> Result<Catalog, String> {
    let (projects, threads) = read_raw(path)?;
    organize_catalog(projects, threads)
}

pub fn read_database_catalog_with_state(
    path: &Path,
    state: &Value,
    host: &str,
) -> Result<Catalog, String> {
    let (mut projects, mut threads) = read_raw(path)?;
    merge_legacy_projects(&mut projects, state, host)?;
    merge_legacy_assignments(&projects, &mut threads, state, host)?;
    organize_catalog(projects, threads)
}

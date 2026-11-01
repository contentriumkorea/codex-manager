use crate::ensure_plain;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Journal {
    pub path: PathBuf,
}
impl Journal {
    pub fn operation(&self, id: &str) -> Result<Option<Value>, String> {
        let db = self.db()?;
        let row=db.query_row("SELECT id,state,payload,(SELECT min(created_at) FROM events WHERE operation_id=operations.id) FROM operations WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?))).optional().map_err(|e|e.to_string())?;
        row.map(|(id, state, payload, at)| {
            serde_json::from_str::<Value>(&payload)
                .map(|payload| json!({"id":id,"state":state,"payload":payload,"createdAt":at}))
                .map_err(|e| e.to_string())
        })
        .transpose()
    }
    pub fn claim(&self, id: &str, resources: &[String]) -> Result<(), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for resource in resources {
            tx.execute(
                "INSERT OR IGNORE INTO locks(resource,operation_id) VALUES(?1,?2)",
                params![resource, id],
            )
            .map_err(|e| e.to_string())?;
            let owner: String = tx
                .query_row(
                    "SELECT operation_id FROM locks WHERE resource=?1",
                    [resource],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if owner != id {
                return Err("다른 미완료 작업이 있습니다. 해당 작업을 먼저 복구하세요.".into());
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn open(path: &Path) -> Result<Self, String> {
        ensure_plain(path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let journal = Self {
            path: path.to_owned(),
        };
        journal.db()?.execute_batch("CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY,state TEXT,payload TEXT);CREATE TABLE IF NOT EXISTS events(operation_id TEXT,phase TEXT,payload TEXT,created_at TEXT DEFAULT CURRENT_TIMESTAMP);CREATE TABLE IF NOT EXISTS locks(resource TEXT PRIMARY KEY,operation_id TEXT);").map_err(|e|e.to_string())?;
        Ok(journal)
    }
    fn db(&self) -> Result<Connection, String> {
        ensure_plain(&self.path)?;
        let db = Connection::open(&self.path).map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        Ok(db)
    }
    pub fn begin(&self, payload: &Value) -> Result<String, String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO operations VALUES (?1,'planned',?2)",
            params![id, payload.to_string()],
        )
        .map_err(|e| e.to_string())?;
        if let Some(resources) = payload["resources"].as_array() {
            for r in resources {
                tx.execute(
                    "INSERT INTO locks VALUES (?1,?2)",
                    params![r.as_str().ok_or("잘못된 작업 잠금")?, id],
                )
                .map_err(|_| {
                    "같은 저장소의 미완료 작업이 있습니다. 작업 내역에서 복구하세요.".to_owned()
                })?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }
    pub fn record(&self, id: &str, phase: &str, payload: Value) -> Result<(), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO events(operation_id,phase,payload) VALUES (?1,?2,?3)",
            params![id, phase, payload.to_string()],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE operations SET state=?1 WHERE id=?2",
            params![phase, id],
        )
        .map_err(|e| e.to_string())?;
        if ["completed", "cancelled"].contains(&phase) {
            tx.execute("DELETE FROM locks WHERE operation_id=?1", [id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn history(&self) -> Result<Vec<Value>, String> {
        let db = self.db()?;
        let mut q=db.prepare("SELECT id,state,payload,(SELECT min(created_at) FROM events WHERE operation_id=operations.id) FROM operations WHERE state NOT IN ('completed','cancelled') OR rowid IN (SELECT rowid FROM operations ORDER BY rowid DESC LIMIT 500) ORDER BY rowid DESC").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            let (id, state, payload, at) = row.map_err(|e| e.to_string())?;
            let payload: Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
            result.push(json!({"id":id,"state":state,"payload":payload,"createdAt":at}));
        }
        Ok(result)
    }
    pub fn events(&self, id: &str) -> Result<Vec<Value>, String> {
        let db = self.db()?;
        let mut q = db
            .prepare("SELECT phase,payload FROM events WHERE operation_id=?1 ORDER BY rowid")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            let (phase, payload) = row.map_err(|e| e.to_string())?;
            result.push(json!({"phase":phase,"payload":serde_json::from_str::<Value>(&payload).map_err(|e|e.to_string())?}));
        }
        Ok(result)
    }
}

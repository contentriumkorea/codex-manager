use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Instant;

#[derive(Clone)]
pub struct Job(Arc<Inner>);
struct Inner {
    cancelled: AtomicBool,
    started: Instant,
    progress: Mutex<Value>,
}
impl Default for Job {
    fn default() -> Self {
        Self::new()
    }
}
impl Job {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            cancelled: AtomicBool::new(false),
            started: Instant::now(),
            progress: Mutex::new(
                json!({"state":"running","phase":"준비","done":0,"total":0,"bytes":0}),
            ),
        }))
    }
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn check(&self) -> Result<(), String> {
        if self.0.cancelled.load(Ordering::SeqCst) {
            Err("작업을 취소했습니다. 처리된 항목은 작업 내역에서 확인하세요.".into())
        } else {
            Ok(())
        }
    }
    pub fn phase(&self, name: &str, total: u64) {
        self.phase_unit(name, total, "bytes");
    }
    pub fn phase_items(&self, name: &str, total: u64) {
        self.phase_unit(name, total, "items");
    }
    fn phase_unit(&self, name: &str, total: u64, unit: &str) {
        let mut p = self.0.progress.lock().unwrap();
        p["phase"] = json!(name);
        p["total"] = json!(total);
        p["done"] = json!(0);
        p["phaseStarted"] = json!(self.0.started.elapsed().as_secs_f64());
        p["phaseBytes"] = json!(0);
        p["unit"] = json!(unit);
    }
    pub fn advance(&self, done: u64, path: &str, bytes: u64) {
        let mut p = self.0.progress.lock().unwrap();
        p["done"] = json!(done);
        p["path"] = json!(path);
        p["bytes"] = json!(p["bytes"].as_u64().unwrap_or(0).saturating_add(bytes));
        p["phaseBytes"] = json!(p["phaseBytes"].as_u64().unwrap_or(0).saturating_add(bytes));
    }
    pub fn finish(&self, result: Result<Value, String>) {
        let mut p = self.0.progress.lock().unwrap();
        p["finalElapsed"] = json!(self.0.started.elapsed().as_secs_f64());
        match result {
            Ok(value) => {
                p["state"] = json!("completed");
                p["result"] = value;
            }
            Err(error) => {
                p["state"] = json!(if self.0.cancelled.load(Ordering::SeqCst) {
                    "cancelled"
                } else {
                    "failed"
                });
                p["error"] = json!(error);
            }
        }
    }
    pub fn snapshot(&self) -> Value {
        let mut p = self.0.progress.lock().unwrap().clone();
        let elapsed = p["finalElapsed"]
            .as_f64()
            .unwrap_or_else(|| self.0.started.elapsed().as_secs_f64());
        let seconds = elapsed - p["phaseStarted"].as_f64().unwrap_or(0.0);
        let speed = p["phaseBytes"].as_u64().unwrap_or(0) as f64 / seconds.max(0.001);
        p["elapsed"] = json!(elapsed);
        p["speed"] = json!(speed);
        let left = p["total"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(p["done"].as_u64().unwrap_or(0));
        p["remaining"] = if speed > 0.0 {
            json!(left as f64 / speed)
        } else {
            Value::Null
        };
        p
    }
}

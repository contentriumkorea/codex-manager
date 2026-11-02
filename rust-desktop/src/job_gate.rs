use codex_manager_core::Job;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

struct Active {
    id: String,
    job: Job,
    automatic: bool,
}
#[derive(Default)]
struct State {
    active: Option<Active>,
    waiting: usize,
    stopped: bool,
}
#[derive(Default)]
pub struct JobGate {
    state: Mutex<State>,
    changed: Condvar,
}
impl JobGate {
    pub fn begin(&self, id: &str, job: &Job, automatic: bool) -> Result<(), String> {
        let mut s = self.state.lock().map_err(|_| "작업 잠금 오류")?;
        if s.stopped {
            return Err("프로그램 종료·업데이트 준비 중입니다.".into());
        }
        if automatic && (s.waiting > 0 || s.active.is_some()) {
            return Err("자동 보관을 다음 주기로 미룹니다.".into());
        }
        if !automatic {
            s.waiting += 1;
            let deadline = Instant::now() + Duration::from_secs(30);
            while let Some(active) = &s.active {
                if !active.automatic {
                    s.waiting -= 1;
                    return Err("진행 중인 작업을 완료하거나 취소하세요.".into());
                }
                active.job.cancel();
                let (next, _) = self
                    .changed
                    .wait_timeout(s, Duration::from_millis(100))
                    .map_err(|_| "작업 잠금 오류")?;
                s = next;
                if Instant::now() >= deadline {
                    s.waiting -= 1;
                    return Err(
                        "자동 보관을 안전하게 중단 중입니다. 잠시 뒤 다시 시도하세요.".into(),
                    );
                }
            }
            s.waiting -= 1;
            if s.stopped {
                return Err("프로그램 종료·업데이트 준비 중입니다.".into());
            }
        }
        s.active = Some(Active {
            id: id.into(),
            job: job.clone(),
            automatic,
        });
        Ok(())
    }
    pub fn finish(&self, id: &str) {
        if let Ok(mut s) = self.state.lock() {
            if s.active.as_ref().is_some_and(|a| a.id == id) {
                s.active = None;
            }
            self.changed.notify_all();
        }
    }
    pub fn stop(&self) -> Result<(), String> {
        let job = Job::new();
        self.begin("shutdown", &job, false)?;
        self.state.lock().map_err(|_| "작업 잠금 오류")?.stopped = true;
        self.finish("shutdown");
        Ok(())
    }
    pub fn resume(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.stopped = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_and_exit_cancel_automatic_and_wait_for_terminal() {
        let gate = std::sync::Arc::new(JobGate::default());
        for stopping in [false, true] {
            let auto = Job::new();
            gate.begin("auto", &auto, true).unwrap();
            let g = gate.clone();
            let j = auto.clone();
            let worker = std::thread::spawn(move || {
                while j.check().is_ok() {
                    std::thread::yield_now();
                }
                j.finish(Err("cancelled".into()));
                g.finish("auto");
            });
            if stopping {
                gate.stop().unwrap();
            } else {
                gate.begin("manual", &Job::new(), false).unwrap();
            }
            assert_ne!(auto.snapshot()["state"], "running");
            worker.join().unwrap();
            assert!(gate.begin("auto2", &Job::new(), true).is_err());
            if !stopping {
                assert!(gate.stop().is_err());
                gate.finish("manual");
            }
        }
    }
}

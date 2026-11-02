use codex_manager_core::{runtime_roots, runtime_roots_with_job, Job};
use serde_json::json;
use std::{fs, path::PathBuf};
#[test]
fn reverse_scan_crosses_large_lines_and_explicit_empty_roots_win() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&root).unwrap();
    let path = root.join("rollout.jsonl");
    let first = json!({"type":"session_meta","payload":{"id":"00000000-0000-4000-8000-000000000099","runtime_workspace_roots":[root]}});
    let large = json!({"type":"response_item","payload":{"content":"x".repeat(300_000)}});
    fs::write(&path, format!("{first}\n{large}\n")).unwrap();
    assert_eq!(runtime_roots(&path).unwrap(), vec![root]);
    let last = json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"runtime_workspace_roots":[]}}});
    fs::write(&path, format!("{first}\n{large}\n{last}\n")).unwrap();
    assert!(runtime_roots(&path).unwrap().is_empty());
    let job = Job::new();
    job.cancel();
    assert!(runtime_roots_with_job(&path, &job).is_err());
}

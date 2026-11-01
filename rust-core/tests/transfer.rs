use codex_manager_core::{copy_tree, resolve_store, scan_tree, Job};
use std::{fs, path::PathBuf};
fn fixture() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn copy_verifies_nested_files_and_empty_folders_and_never_overwrites() {
    let p = fixture();
    let src = p.join("src");
    let dst = p.join("dst");
    fs::create_dir_all(src.join("한글/empty")).unwrap();
    fs::write(src.join("한글/file.txt"), "contents").unwrap();
    let job = Job::new();
    let tree = scan_tree(&src, &job).unwrap();
    copy_tree(&src, &dst, &tree, &job).unwrap();
    assert_eq!(fs::read(dst.join("한글/file.txt")).unwrap(), b"contents");
    assert!(dst.join("한글/empty").is_dir());
    assert!(copy_tree(&src, &dst, &tree, &Job::new()).is_err());
    assert!(copy_tree(&src, &src.join("nested"), &tree, &Job::new()).is_err());
}
#[test]
fn cancellation_and_source_change_keep_original_and_do_not_report_completion() {
    let p = fixture();
    let src = p.join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("x"), "old").unwrap();
    let tree = scan_tree(&src, &Job::new()).unwrap();
    fs::write(src.join("x"), "changed").unwrap();
    assert!(copy_tree(&src, &p.join("dst"), &tree, &Job::new()).is_err());
    let job = Job::new();
    job.cancel();
    assert!(scan_tree(&src, &job).is_err());
    assert_eq!(fs::read(src.join("x")).unwrap(), b"changed");
    assert_ne!(job.snapshot()["state"], "completed");
}
#[test]
fn sqlite_home_and_latest_numeric_database_are_respected_without_writes() {
    let p = fixture();
    let db = p.join("database");
    fs::create_dir(&db).unwrap();
    fs::write(
        p.join("config.toml"),
        format!("sqlite_home = {}", serde_json::to_string(&db).unwrap()),
    )
    .unwrap();
    fs::write(db.join("state_9.sqlite"), []).unwrap();
    fs::write(db.join("state_10.sqlite"), []).unwrap();
    let store = resolve_store(&p).unwrap();
    assert_eq!(store.database.unwrap(), db.join("state_10.sqlite"));
    fs::write(p.join("config.toml"), "sqlite_home = 'relative'").unwrap();
    assert!(resolve_store(&p).is_err());
}

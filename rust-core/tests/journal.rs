use codex_manager_core::Journal;
use serde_json::json;
#[test]
fn pending_operation_stays_recoverable_after_five_hundred_newer_records() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    let journal = Journal::open(&root.join("journal.sqlite")).unwrap();
    let old = journal
        .begin(&json!({"kind":"rust-import","resources":["home:old"]}))
        .unwrap();
    let mut c = rusqlite::Connection::open(&journal.path).unwrap();
    let tx = c.transaction().unwrap();
    for n in 0..505 {
        tx.execute(
            "INSERT INTO operations VALUES(?1,'completed','{}')",
            [format!("new-{n}")],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(journal.operation(&old).unwrap().unwrap()["id"], old);
    assert!(journal.history().unwrap().iter().any(|r| r["id"] == old));
}

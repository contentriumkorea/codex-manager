use codex_manager_core::{Adapter, Job, Journal, Store};
use std::{fs, path::PathBuf};
fn setup() -> (Adapter, PathBuf) {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(uuid::Uuid::new_v4().to_string());
    let home = p.join("home");
    let workspace = p.join("workspace");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir(&workspace).unwrap();
    let db = home.join("state_5.sqlite");
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch("CREATE TABLE projects(id TEXT,name TEXT);CREATE TABLE project_roots(project_id TEXT,path TEXT);CREATE TABLE threads(id TEXT,project_id TEXT,cwd TEXT);INSERT INTO projects VALUES('p','P');").unwrap();
    c.execute(
        "INSERT INTO project_roots VALUES('p',?1)",
        [workspace.to_str().unwrap()],
    )
    .unwrap();
    drop(c);
    let adapter = Adapter::isolated(
        Store {
            home: home.clone(),
            sqlite_home: home,
            database: Some(db),
        },
        p.join("unused.exe"),
        Journal::open(&p.join("manager/journal.sqlite")).unwrap(),
    )
    .unwrap();
    (adapter, workspace)
}
#[test]
fn file_rename_copy_delete_and_undo_preserve_originals_and_new_files() {
    let (mut adapter, root) = setup();
    let source = root.join("한글.txt");
    fs::write(&source, "original").unwrap();
    let renamed = root.join("이름.txt");
    let op = adapter
        .edit_files(
            "rename",
            std::slice::from_ref(&source),
            &root,
            Some("이름.txt"),
            &Job::new(),
        )
        .unwrap();
    assert!(renamed.exists());
    adapter.undo(&op).unwrap();
    assert!(source.exists());
    let target = root.join("target");
    fs::create_dir(&target).unwrap();
    let op = adapter
        .edit_files(
            "copy",
            std::slice::from_ref(&source),
            &target,
            None,
            &Job::new(),
        )
        .unwrap();
    assert!(target.join("한글.txt").exists());
    adapter.undo(&op).unwrap();
    assert!(!target.join("한글.txt").exists());
    let op = adapter
        .edit_files(
            "delete",
            std::slice::from_ref(&source),
            &root,
            None,
            &Job::new(),
        )
        .unwrap();
    assert!(!source.exists());
    fs::write(&source, "new").unwrap();
    assert!(adapter.undo(&op).is_err());
    assert_eq!(fs::read(&source).unwrap(), b"new");
}
#[test]
fn project_root_and_parent_child_selection_cannot_be_changed_as_files() {
    let (adapter, root) = setup();
    assert!(adapter
        .edit_files(
            "delete",
            std::slice::from_ref(&root),
            &root,
            None,
            &Job::new()
        )
        .is_err());
    let folder = root.join("folder");
    fs::create_dir(&folder).unwrap();
    let file = folder.join("x");
    fs::write(&file, []).unwrap();
    assert!(adapter
        .edit_files("delete", &[folder, file], &root, None, &Job::new())
        .is_err());
}
#[test]
fn new_folder_is_undoable_but_a_new_file_inside_blocks_undo() {
    let (mut adapter, root) = setup();
    let op = adapter.mkdir(&root, "새 폴더", &Job::new()).unwrap();
    let folder = root.join("새 폴더");
    assert!(folder.is_dir());
    fs::write(folder.join("new.txt"), "new").unwrap();
    assert!(adapter.undo(&op).is_err());
    assert!(folder.join("new.txt").is_file());
}
#[test]
fn secondary_runtime_workspace_blocks_directory_changes() {
    let (adapter, root) = setup();
    let secondary = root.join("secondary");
    fs::create_dir(&secondary).unwrap();
    let outside = root.parent().unwrap().join("other-workspace");
    fs::create_dir(&outside).unwrap();
    let rollout = root.parent().unwrap().join("thread.jsonl");
    fs::write(&rollout,format!("{}\n",serde_json::json!({"type":"session_meta","payload":{"id":"00000000-0000-4000-8000-000000000099","runtime_workspace_roots":[secondary]}}))).unwrap();
    let db = rusqlite::Connection::open(adapter.store.database.as_ref().unwrap()).unwrap();
    db.execute_batch("ALTER TABLE threads ADD COLUMN rollout_path TEXT;")
        .unwrap();
    db.execute(
        "INSERT INTO threads VALUES('thread',NULL,?1,?2)",
        rusqlite::params![outside.to_str().unwrap(), rollout.to_str().unwrap()],
    )
    .unwrap();
    drop(db);
    for kind in ["delete", "rename", "move"] {
        assert!(adapter
            .edit_files(
                kind,
                std::slice::from_ref(&secondary),
                &outside,
                Some("renamed"),
                &Job::new()
            )
            .is_err());
    }
    assert!(secondary.is_dir());
}
#[test]
fn cancelled_undo_uses_the_callers_job_without_changing_files() {
    let (mut adapter, root) = setup();
    let op = adapter.mkdir(&root, "keep", &Job::new()).unwrap();
    let job = Job::new();
    job.cancel();
    assert!(adapter.undo_with_job(&op, &job).is_err());
    assert!(root.join("keep").is_dir());
    assert_eq!(
        adapter.journal.operation(&op).unwrap().unwrap()["state"],
        "completed"
    );
}

#[test]
fn external_file_copy_move_and_undo_ignore_unrelated_corrupt_history() {
    let (mut adapter, root) = setup();
    let db = rusqlite::Connection::open(adapter.store.database.as_ref().unwrap()).unwrap();
    db.execute_batch("ALTER TABLE threads ADD COLUMN rollout_path TEXT;")
        .unwrap();
    let bad = root.parent().unwrap().join("bad.jsonl");
    fs::write(&bad, b"malformed\n").unwrap();
    db.execute(
        "INSERT INTO threads VALUES('unrelated',NULL,?1,?2)",
        rusqlite::params![root.parent().unwrap().to_str(), bad.to_str()],
    )
    .unwrap();
    drop(db);
    let destination = root.parent().unwrap().join("unregistered archive");
    fs::create_dir(&destination).unwrap();
    let source = root.join("document.txt");
    fs::write(&source, "keep exact bytes").unwrap();
    assert!(adapter
        .edit_files("copy", &[source.clone()], &destination, None, &Job::new())
        .is_err());
    for kind in ["copy", "move"] {
        let op = adapter
            .edit_files_to_folder(kind, &[source.clone()], &destination, &Job::new())
            .unwrap();
        assert_eq!(
            fs::read(destination.join("document.txt")).unwrap(),
            b"keep exact bytes"
        );
        assert_eq!(source.exists(), kind == "copy");
        adapter.undo(&op).unwrap();
        assert!(source.exists());
        assert!(!destination.join("document.txt").exists());
    }
}

#[test]
fn sidecar_hidden_and_not_editable_as_ordinary_files() {
    let (adapter, root) = setup();
    let sidecar = root.join(".codex-manager");
    fs::create_dir(&sidecar).unwrap();
    let file = sidecar.join("current.json");
    fs::write(&file, "keep").unwrap();
    assert!(codex_manager_core::list_files(&root, &[root.clone()])
        .unwrap()
        .is_empty());
    for source in [&sidecar, &file] {
        for kind in ["copy", "move", "rename", "delete"] {
            assert!(adapter
                .edit_files(kind, &[source.clone()], &root, Some("changed"), &Job::new())
                .is_err());
        }
    }
    assert!(adapter.mkdir(&root, ".CODEX-MANAGER", &Job::new()).is_err());
}

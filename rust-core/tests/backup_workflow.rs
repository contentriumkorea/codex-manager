use codex_manager_core::{
    backup_project, discover_bundles, load_verified_bundle, read_database_catalog, Job,
};
use rusqlite::Connection;
use serde_json::json;
use std::{fs, path::PathBuf};
#[test]
fn export_is_python_format_compatible_and_parent_discovery_finds_the_bundle() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(uuid::Uuid::new_v4().to_string());
    let workspace = p.join("workspace");
    fs::create_dir_all(workspace.join("empty")).unwrap();
    fs::write(workspace.join("한글.txt"), "보존").unwrap();
    let rollout = p.join("chat.jsonl");
    let id = "00000000-0000-4000-8000-000000000099";
    fs::write(&rollout,format!("{}\n",json!({"type":"session_meta","payload":{"id":id,"cwd":workspace,"runtime_workspace_roots":[workspace]}}))).unwrap();
    let database = p.join("state.sqlite");
    let c = Connection::open(&database).unwrap();
    c.execute_batch("CREATE TABLE projects(id TEXT,name TEXT);CREATE TABLE project_roots(project_id TEXT,path TEXT);CREATE TABLE threads(id TEXT,project_id TEXT,cwd TEXT,rollout_path TEXT,name TEXT,archived INTEGER);").unwrap();
    c.execute("INSERT INTO projects VALUES('project','테스트')", [])
        .unwrap();
    c.execute(
        "INSERT INTO project_roots VALUES('project',?1)",
        [workspace.to_str().unwrap()],
    )
    .unwrap();
    c.execute(
        "INSERT INTO threads VALUES(?1,'project',?2,?3,'대화',0)",
        rusqlite::params![id, workspace.to_str().unwrap(), rollout.to_str().unwrap()],
    )
    .unwrap();
    drop(c);
    let backup = p.join("backups/project");
    backup_project(
        &read_database_catalog(&database).unwrap(),
        "project",
        &backup,
        &Job::new(),
    )
    .unwrap();
    let m = load_verified_bundle(&backup, &Job::new()).unwrap();
    assert_eq!(m["project"]["name"], "테스트");
    assert_eq!(m["conversations"][0]["id"], id);
    assert_eq!(m["inventory"][0]["relative_path"], "한글.txt");
    assert_eq!(
        discover_bundles(&p.join("backups"), &Job::new())
            .unwrap()
            .len(),
        1
    );
    fs::write(backup.join("files/root-01/한글.txt"), "손상").unwrap();
    assert!(load_verified_bundle(&backup, &Job::new()).is_err());
}

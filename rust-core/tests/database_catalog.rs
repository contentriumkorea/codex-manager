use codex_manager_core::{read_database_catalog, read_database_catalog_with_state};
use rusqlite::Connection;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn reads_empty_and_assigned_threads_without_folder_guessing() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(format!("sqlite-{nonce}"));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("state_5.sqlite");
    {
        let c = Connection::open(&db).unwrap();
        c.execute_batch("
            CREATE TABLE projects (id TEXT PRIMARY KEY,name TEXT,position INTEGER);
            CREATE TABLE project_roots (project_id TEXT,position INTEGER,path TEXT);
            CREATE TABLE threads (id TEXT PRIMARY KEY,project_id TEXT,cwd TEXT,rollout_path TEXT,
                                  name TEXT,title TEXT,archived INTEGER,has_user_event INTEGER);
            INSERT INTO projects VALUES ('a','A',0),('b','B',1);
            INSERT INTO project_roots VALUES ('a',0,'C:/shared'),('a',1,'D:/other'),('b',0,'C:/shared');
            INSERT INTO threads VALUES ('assigned','b','C:/shared',NULL,'Conversation','',0,1);
            INSERT INTO threads VALUES ('empty','a','C:/shared',NULL,NULL,'',0,0);
            INSERT INTO threads VALUES ('unknown','missing','C:/shared',NULL,NULL,'',0,1);
            INSERT INTO threads VALUES ('loose',NULL,'C:/shared',NULL,NULL,'',0,1);
        ").unwrap();
    }
    let catalog = read_database_catalog(&db).unwrap();
    assert_eq!(catalog.projects.len(), 2);
    assert_eq!(
        catalog.projects[0].project["roots"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(catalog.projects[0].threads[0]["id"], "empty");
    assert_eq!(catalog.projects[1].threads[0]["id"], "assigned");
    assert_eq!(catalog.unassigned[0]["id"], "loose");
    assert_eq!(catalog.unresolved[0]["id"], "unknown");
    let c = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}

#[test]
fn unknown_database_schema_is_an_error_instead_of_an_empty_catalog() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(format!("unknown-{nonce}"));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("unknown.sqlite");
    Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated (id TEXT)")
        .unwrap();
    assert!(read_database_catalog(&db).is_err());
}

#[test]
#[ignore = "requires a generated isolated 0.166 Codex database fixture"]
fn current_codex_database_keeps_threads_omitted_from_ordinary_listing() {
    let db = std::path::PathBuf::from(
        std::env::var_os("CODEX_DATABASE_FIXTURE").expect("fixture required"),
    );
    let catalog = read_database_catalog(&db).unwrap();
    assert_eq!(catalog.projects.len(), 1);
    assert_eq!(
        catalog.projects[0].project["roots"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(catalog.projects[0].threads.len(), 1);
    assert_eq!(catalog.projects[0].threads[0]["hasUserEvent"], false);
    assert_eq!(
        catalog.projects[0].threads[0]["id"],
        "00000000-0000-4000-8000-000000000012"
    );
}

#[test]
fn legacy_desktop_membership_is_restored_without_changing_database() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(format!("legacy-{nonce}"));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("state.sqlite");
    {
        let c = Connection::open(&db).unwrap();
        c.execute_batch("CREATE TABLE projects (id TEXT,name TEXT); CREATE TABLE project_roots(project_id TEXT,path TEXT);
          CREATE TABLE threads(id TEXT,project_id TEXT,cwd TEXT);
          INSERT INTO projects VALUES ('native','Project');
          INSERT INTO project_roots VALUES ('native','C:/root');
          INSERT INTO threads VALUES ('legacy-thread',NULL,'C:/root');").unwrap();
    }
    let state = serde_json::json!({"thread-project-assignments":{"legacy-thread":{"projectKind":"local","projectId":"old"}},
       "app-server-project-id-by-legacy-project-id-by-host":{"local:D:/store":{"old":"native"}}});
    let catalog = read_database_catalog_with_state(&db, &state, "local:D:/store").unwrap();
    assert!(catalog.unassigned.is_empty());
    assert_eq!(catalog.projects[0].threads[0]["id"], "legacy-thread");
    let c = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        c.query_row("SELECT project_id FROM threads", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
}

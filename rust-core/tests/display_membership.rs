use codex_manager_core::{
    all_threads, merge_legacy_assignments, organize_catalog, read_database_catalog,
};
use serde_json::{json, Value};

fn thread(id: &str, cwd: &str) -> Value {
    json!({"id":id,"projectId":null,"cwd":cwd,"hasUserEvent":false})
}

#[test]
fn display_inference_keeps_raw_membership_and_protected_reasons() {
    let projects = vec![
        json!({"id":"a","roots":[{"path":"C:/Work"}]}),
        json!({"id":"b","roots":[{"path":"c:/work/deep"}]}),
        json!({"id":"c","roots":[{"path":"C:/shared"}]}),
        json!({"id":"d","roots":[{"path":"c:/SHARED"}]}),
        json!({"id":"unc","roots":[{"path":"\\\\server\\share\\workspace"}]}),
    ];
    let mut threads = vec![
        thread("folder", "\\\\?\\C:\\WORK\\deep\\.\\child"),
        thread("boundary", "C:/worker"),
        thread("ambiguous", "C:/shared/a"),
        thread("projectless", "C:/work"),
        thread("remote", "C:/work"),
        thread("unc", "\\\\?\\UNC\\Server\\Share\\workspace\\child"),
        thread("legacy", "C:/other"),
    ];
    threads.push(json!({"id":"explicit","projectId":"a","cwd":"C:/work/deep"}));
    threads.push(json!({"id":"missing","projectId":"gone","cwd":"C:/work/deep"}));
    let state = json!({"projectless-thread-ids":["projectless"],"thread-project-membership-host-ids":{"remote":"remote:other"},
        "thread-project-assignments":{"legacy":{"projectKind":"local","projectId":"a"}}});
    merge_legacy_assignments(&projects, &mut threads, &state, "local:test").unwrap();
    let catalog = organize_catalog(projects, threads).unwrap();
    let all = all_threads(&catalog);
    let get = |id: &str| all.iter().find(|t| t["id"] == id).unwrap();
    assert_eq!(get("folder")["displayProjectId"], "b");
    assert!(get("folder")["projectId"].is_null());
    assert_eq!(get("folder")["membershipReason"], "folder");
    assert_eq!(get("folder")["hasUserEvent"], false);
    assert_eq!(get("explicit")["displayProjectId"], "a");
    assert_eq!(get("unc")["displayProjectId"], "unc");
    assert_eq!(get("legacy")["projectId"], "a");
    assert!(get("legacy")["nativeProjectId"].is_null());
    for (id, reason) in [
        ("boundary", "outside"),
        ("ambiguous", "ambiguous"),
        ("projectless", "projectless"),
        ("remote", "other_host"),
        ("missing", "missing_project"),
    ] {
        assert!(get(id)["displayProjectId"].is_null());
        assert_eq!(get(id)["membershipReason"], reason);
    }
}

#[test]
fn parent_precedes_internal_folder_and_conflicts_and_cycles_do_not_guess() {
    let projects = vec![
        json!({"id":"a","roots":[{"path":"C:/a"}]}),
        json!({"id":"b","roots":[{"path":"C:/b"}]}),
    ];
    let mut ts = vec![json!({"id":"p","projectId":"a","cwd":"C:/a"})];
    for (id, cwd, parent, internal) in [
        ("child", "C:/b", "p", true),
        ("normal", "C:/b", "p", false),
        ("outside", "C:/other", "child", false),
        ("x", "C:/a", "y", true),
        ("y", "C:/a", "x", true),
    ] {
        let mut t = thread(id, cwd);
        t["parentThreadId"] = json!(parent);
        t["internal"] = json!(internal);
        ts.push(t);
    }
    let mut protected = thread("protected", "C:/a");
    protected["parentThreadId"] = json!("p");
    protected["explicitProjectless"] = json!(true);
    ts.push(protected);
    // Long reverse-ordered ancestry proves iterative resolution is stack-safe.
    for i in (0..12000).rev() {
        let mut t = thread(&format!("deep-{i}"), "C:/other");
        t["internal"] = json!(true);
        t["parentThreadId"] = if i == 0 {
            json!("p")
        } else {
            json!(format!("deep-{}", i - 1))
        };
        ts.push(t);
    }
    let all = all_threads(&organize_catalog(projects, ts).unwrap());
    for id in ["child", "outside", "deep-11999"] {
        let t = all.iter().find(|t| t["id"] == id).unwrap();
        assert_eq!(t["displayProjectId"], "a");
        assert_eq!(t["membershipReason"], "parent");
        assert!(t["projectId"].is_null());
    }
    for id in ["normal", "x", "y"] {
        let t = all.iter().find(|t| t["id"] == id).unwrap();
        assert!(t["displayProjectId"].is_null());
        assert_eq!(t["membershipReason"], "ambiguous");
    }
    assert_eq!(
        all.iter().find(|t| t["id"] == "protected").unwrap()["membershipReason"],
        "projectless"
    );
}

#[test]
fn database_internal_source_fallback_is_independent_of_archiving_and_user_event() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(format!("classification-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("state.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE projects(id TEXT,name TEXT); CREATE TABLE project_roots(project_id TEXT,path TEXT); CREATE TABLE threads(id TEXT,project_id TEXT,cwd TEXT,source TEXT,thread_source TEXT,has_user_event INTEGER,archived INTEGER);
        INSERT INTO threads VALUES('normal',NULL,'C:/none','\"cli\"','cli',0,1),('guardian',NULL,'C:/none',NULL,'guardian_review',0,0),('sub',NULL,'C:/none',NULL,'subagent',0,0),('object',NULL,'C:/none','{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"normal\"}}}',NULL,0,0);").unwrap();
    let all = all_threads(&read_database_catalog(&path).unwrap());
    for t in &all {
        assert_eq!(t["internal"], t["id"] != "normal");
        assert_eq!(t["hasUserEvent"], false);
    }
    assert_eq!(
        all.iter().find(|t| t["id"] == "normal").unwrap()["archived"],
        true
    );
}

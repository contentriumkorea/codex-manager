use codex_manager_core::{resolve_store, Adapter, Job, Journal, Store};
use serde_json::json;
use std::{fs, path::PathBuf};
#[test]
#[ignore = "requires installed Codex executable and a fresh isolated home"]
fn current_server_rename_assign_merge_delete_and_undo_keep_full_catalog() {
    let root = PathBuf::from(std::env::var_os("CODEX_TEST_HOME").unwrap()).join("management");
    assert!(!root.exists());
    fs::create_dir_all(&root).unwrap();
    let home = root.join("codex-home");
    fs::create_dir_all(&home).unwrap();
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("sample.txt"), "keep").unwrap();
    let binary = PathBuf::from(std::env::var_os("CODEX_TEST_BINARY").unwrap());
    let store = Store {
        home: home.clone(),
        sqlite_home: home.clone(),
        database: None,
    };
    let journal = Journal::open(&root.join("manager/journal.sqlite")).unwrap();
    let mut adapter = Adapter::isolated(store, binary, journal).unwrap();
    let a = adapter
        .create("A", std::slice::from_ref(&workspace))
        .unwrap();
    let b = adapter.create("B", &[]).unwrap();
    let id = "00000000-0000-4000-8000-000000000088";
    let rollout = root.join("test.jsonl");
    let meta = json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":workspace,"originator":"codex_cli_rs","cli_version":"0.0.0","source":"cli","model_provider":"openai"}});
    let event = json!({"timestamp":"2026-11-02T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"fixture","images":[],"local_images":[],"text_elements":[]}});
    fs::write(&rollout, format!("{meta}\n{event}\n")).unwrap();
    adapter
        .import_thread(
            &rollout,
            id,
            &workspace,
            std::slice::from_ref(&workspace),
            a["id"].as_str().unwrap(),
            "Original",
        )
        .unwrap();
    let op = adapter
        .rename_threads(&[id.into()], &["Renamed".into()])
        .unwrap();
    assert_eq!(adapter.read_thread(id).unwrap()["name"], "Renamed");
    adapter.undo(&op).unwrap();
    assert_eq!(adapter.read_thread(id).unwrap()["name"], "Original");
    adapter
        .merge(
            &[a["id"].as_str().unwrap().into()],
            b["id"].as_str().unwrap(),
        )
        .unwrap();
    assert_eq!(adapter.read_thread(id).unwrap()["projectId"], b["id"]);
    let delete = adapter
        .delete(&[b["id"].as_str().unwrap().into()], &[], false, &Job::new())
        .unwrap();
    assert!(adapter.snapshot().unwrap().projects.is_empty());
    assert!(workspace.join("sample.txt").is_file());
    adapter.undo(&delete).unwrap();
    assert_eq!(adapter.snapshot().unwrap().projects.len(), 1);
    assert!(resolve_store(&home).unwrap().database.is_some());
    adapter.release_server();
    let project = adapter.snapshot().unwrap().projects[0].project.clone();
    let bundle = root.join("backup");
    codex_manager_core::backup_project(
        &adapter.snapshot().unwrap(),
        project["id"].as_str().unwrap(),
        &bundle,
        &Job::new(),
    )
    .unwrap();
    let second = root.join("second-computer");
    fs::create_dir(&second).unwrap();
    let mut target = Adapter::isolated(
        Store {
            home: second.clone(),
            sqlite_home: second.clone(),
            database: None,
        },
        adapter.binary.clone(),
        Journal::open(&second.join("manager/journal.sqlite")).unwrap(),
    )
    .unwrap();
    let destination = root.join("imported-workspace");
    target
        .import_backup(
            &bundle,
            std::slice::from_ref(&destination),
            None,
            false,
            None,
            &Job::new(),
        )
        .unwrap();
    assert_eq!(fs::read(destination.join("sample.txt")).unwrap(), b"keep");
    let restored = target.read_thread(id).unwrap();
    assert_eq!(restored["name"], "Original");
    assert_eq!(
        codex_manager_core::clean_path(std::path::Path::new(restored["cwd"].as_str().unwrap())),
        destination
    );
    assert_eq!(target.snapshot().unwrap().projects[0].threads.len(), 1);
    let pid = target.snapshot().unwrap().projects[0].project["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let moved = root.join("moved-workspace");
    let transfer = target
        .transfer_project(&pid, std::slice::from_ref(&moved), true, &Job::new())
        .unwrap();
    assert!(!destination.exists());
    assert_eq!(
        target.journal.operation(&transfer).unwrap().unwrap()["state"],
        "completed"
    );
    fs::write(moved.join("sample.txt"), "later work").unwrap();
    assert!(target.undo(&transfer).is_err());
    assert_eq!(fs::read(moved.join("sample.txt")).unwrap(), b"later work");
    fs::write(moved.join("sample.txt"), "keep").unwrap();
    assert_eq!(fs::read(moved.join("sample.txt")).unwrap(), b"keep");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("new-work.txt"), "preserve").unwrap();
    assert!(target.undo(&transfer).is_err());
    assert_eq!(
        target.journal.operation(&transfer).unwrap().unwrap()["state"],
        "completed"
    );
    assert_eq!(
        fs::read(destination.join("new-work.txt")).unwrap(),
        b"preserve"
    );
    fs::remove_file(destination.join("new-work.txt")).unwrap();
    fs::remove_dir(&destination).unwrap();
    let extra = root.join("later-workspace");
    fs::create_dir(&extra).unwrap();
    let current = codex_manager_core::all_threads(&target.snapshot().unwrap())
        .into_iter()
        .find(|t| t["id"] == id)
        .unwrap();
    target
        .relocate(&current, &moved, &[moved.clone(), extra])
        .unwrap();
    assert!(target
        .undo(&transfer)
        .unwrap_err()
        .contains("보조 작업 폴더"));
    assert!(!destination.exists());
    target
        .relocate(&current, &moved, std::slice::from_ref(&moved))
        .unwrap();
    target.undo(&transfer).unwrap();
    assert!(destination.join("sample.txt").is_file());
    assert_eq!(
        codex_manager_core::clean_path(std::path::Path::new(
            target.read_thread(id).unwrap()["cwd"].as_str().unwrap()
        )),
        destination
    );
    let mut pending_payload =
        target.journal.operation(&transfer).unwrap().unwrap()["payload"].clone();
    let mut pending_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(pending_payload["recovery"].as_str().unwrap()).unwrap())
            .unwrap();
    let partial = root.join("partial-output");
    fs::create_dir(&partial).unwrap();
    pending_manifest["roots"][0]["destination"] = json!(partial);
    let plan = root.join("partial-transfer-plan.json");
    fs::write(&plan, pending_manifest.to_string()).unwrap();
    pending_payload["recovery"] = json!(plan);
    pending_payload["transferPlanHash"] =
        json!(codex_manager_core::digest(&plan, &Job::new()).unwrap());
    pending_payload["afterProjects"][0]["roots"] = json!([{"path":partial}]);
    pending_payload["destinations"] = json!([partial]);
    let pending_copy = target.journal.begin(&pending_payload).unwrap();
    target.undo(&pending_copy).unwrap();
    assert_eq!(
        target.journal.operation(&pending_copy).unwrap().unwrap()["state"],
        "completed"
    );
    assert!(partial.is_dir());
    assert_eq!(fs::read(destination.join("sample.txt")).unwrap(), b"keep");
    let second_id = "00000000-0000-4000-8000-000000000089";
    let mut second_meta = meta.clone();
    second_meta["payload"]["id"] = json!(second_id);
    second_meta["payload"]["cwd"] = json!(destination);
    let raw = root.join("second-thread.jsonl");
    fs::write(&raw, format!("{second_meta}\n{event}\n")).unwrap();
    target
        .import_thread(
            &raw,
            second_id,
            &destination,
            std::slice::from_ref(&destination),
            &pid,
            "Remaining",
        )
        .unwrap();
    let cancelled = Job::new();
    cancelled.cancel();
    assert!(target.delete(&[], &[id.into()], true, &cancelled).is_err());
    assert_eq!(
        codex_manager_core::all_threads(&target.snapshot().unwrap()).len(),
        2
    );
    let planned_threads = codex_manager_core::all_threads(&target.snapshot().unwrap());
    let pending=target.journal.begin(&json!({"kind":"rust-delete","home":target.store.home,"projects":[],"threads":planned_threads,"permanent":true,"saved":[],"restoreKeys":{},"resources":[format!("home:{}",target.store.home.display())]})).unwrap();
    target.release_server();
    target
        .call("thread/delete", json!({"threadId":id}))
        .unwrap();
    target
        .journal
        .record(&pending, "deleting", json!({"threadId":id}))
        .unwrap();
    let report = target.finalize_delete(&pending, &Job::new()).unwrap();
    assert_eq!(report["deleted"].as_array().unwrap().len(), 1);
    assert_eq!(report["remainingThreads"].as_array().unwrap().len(), 1);
    assert_eq!(
        target.journal.operation(&pending).unwrap().unwrap()["state"],
        "cancelled"
    );
    target
        .rename_threads(&[second_id.into()], &["Continue".into()])
        .unwrap();
    assert_eq!(target.read_thread(second_id).unwrap()["name"], "Continue");
    target.release_server();
    let legacy_path = second.join(".codex-global-state.json");
    fs::write(&legacy_path,json!({"local-projects":{"legacy-fixture":{"id":"legacy-fixture","name":"기존 프로젝트","rootPaths":[destination]}},"app-server-project-id-by-legacy-project-id-by-host":{},"thread-project-assignments":{}}).to_string()).unwrap();
    assert!(target
        .snapshot()
        .unwrap()
        .projects
        .iter()
        .any(|g| g.project["id"] == "legacy-fixture"));
    let aliases = target
        .materialize_legacy(&["legacy-fixture".into()], &Job::new())
        .unwrap();
    let native = aliases.get("legacy-fixture").unwrap();
    target.rename_project(native, "기존 프로젝트 변경").unwrap();
    let catalog = target.snapshot().unwrap();
    assert!(!catalog
        .projects
        .iter()
        .any(|g| g.project["id"] == "legacy-fixture"));
    assert_eq!(
        catalog
            .projects
            .iter()
            .filter(|g| g.project["id"] == *native)
            .count(),
        1
    );
    // Simulate termination after native creation/assignment, before alias publication.
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(&legacy_path).unwrap()).unwrap();
    state["local-projects"]["interrupted-legacy"] =
        json!({"id":"interrupted-legacy","name":"중단된 연결","rootPaths":[destination]});
    state["thread-project-assignments"][second_id] =
        json!({"projectKind":"local","projectId":"interrupted-legacy"});
    fs::write(&legacy_path, state.to_string()).unwrap();
    let before = target.read_thread(second_id).unwrap()["projectId"].clone();
    let old = json!({"id":"interrupted-legacy","name":"중단된 연결","roots":[{"path":destination}],"legacy":true});
    let key = uuid::Uuid::new_v4().to_string();
    let operation=target.journal.begin(&json!({"kind":"rust-legacy","home":second,"project":old,"threads":[{"id":second_id,"nativeBefore":before}],"projectKey":key,"resources":[format!("home:{}",second.display())]})).unwrap();
    let created = target
        .call(
            "project/create",
            json!({"idempotencyKey":key,"name":"중단된 연결","roots":[{"path":destination}]}),
        )
        .unwrap()["project"]
        .clone();
    target
        .call(
            "thread/metadata/update",
            json!({"threadId":second_id,"projectId":created["id"]}),
        )
        .unwrap();
    assert!(target
        .materialize_legacy(&["interrupted-legacy".into()], &Job::new())
        .unwrap_err()
        .contains("다시 선택"));
    assert_eq!(
        target.read_thread(second_id).unwrap()["projectId"],
        created["id"]
    );
    assert_eq!(
        target.journal.operation(&operation).unwrap().unwrap()["state"],
        "completed"
    );
    assert!(!target
        .snapshot()
        .unwrap()
        .projects
        .iter()
        .any(|g| g.project["id"] == "interrupted-legacy"));

    let protected = target
        .create("보호 경로", std::slice::from_ref(&second))
        .unwrap();
    assert!(target
        .transfer_project(
            protected["id"].as_str().unwrap(),
            &[root.join("blocked-move")],
            true,
            &Job::new()
        )
        .unwrap_err()
        .contains("Codex 저장소"));
    assert!(target
        .export_backup(
            protected["id"].as_str().unwrap(),
            &root.join("blocked-backup"),
            &Job::new()
        )
        .is_err());
    assert!(!root.join("blocked-move").exists());
    assert!(!root.join("blocked-backup").exists());
}

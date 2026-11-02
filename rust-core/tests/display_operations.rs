use codex_manager_core::*;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

#[test]
#[ignore = "installed SDK with isolated home only"]
fn inferred_backup_confirm_move_merge_delete_and_original_membership_recovery() {
    let root =
        PathBuf::from(std::env::var_os("CODEX_TEST_HOME").unwrap()).join("display-operations");
    assert!(!root.exists());
    fs::create_dir_all(&root).unwrap();
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    fs::write(home.join(".codex-global-state.json"), b"{}").unwrap();
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("keep.txt"), "original bytes").unwrap();
    let journal = Journal::open(&root.join("manager/journal.sqlite")).unwrap();
    let mut adapter = Adapter::isolated(
        resolve_store(&home).unwrap(),
        PathBuf::from(std::env::var_os("CODEX_TEST_BINARY").unwrap()),
        journal,
    )
    .unwrap();
    let project = adapter
        .create("Source", std::slice::from_ref(&workspace))
        .unwrap();
    let mut pid = project["id"].as_str().unwrap().to_owned();
    let ids = [
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ];
    for (i, id) in ids.iter().enumerate() {
        let source = if i == 0 {
            json!("cli")
        } else {
            json!({"subagent":{"thread_spawn":{"parent_thread_id":ids[0],"depth":1,"agent_nickname":"fixture","agent_role":"default"}}})
        };
        let meta = json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":workspace,"runtime_workspace_roots":[workspace],"originator":"codex_cli_rs","cli_version":"0.0.0","source":source,"model_provider":"openai"}});
        let event = json!({"timestamp":"2026-11-02T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"fictional fixture","images":[],"local_images":[],"text_elements":[]}});
        let file = root.join(format!("{id}.jsonl"));
        fs::write(&file, format!("{meta}\n{event}\n")).unwrap();
        adapter
            .import_thread(
                &file,
                id,
                &workspace,
                std::slice::from_ref(&workspace),
                &pid,
                "Fixture",
            )
            .unwrap();
        adapter
            .call(
                "thread/metadata/update",
                json!({"threadId":id,"projectId":""}),
            )
            .unwrap();
    }
    adapter.release_server();
    // Fixture setup only: remove the explicit legacy assignments created by import.
    let state_path = home.join(".codex-global-state.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    for id in &ids {
        state["thread-project-assignments"]
            .as_object_mut()
            .unwrap()
            .remove(id);
    }
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    let snapshot = adapter.snapshot().unwrap();
    assert_eq!(snapshot.projects[0].threads.len(), 2);
    assert_eq!(
        all_threads(&snapshot)
            .iter()
            .filter(|t| t["internal"] == true)
            .count(),
        1
    );
    assert!(all_threads(&snapshot)
        .iter()
        .all(|t| t["projectId"].is_null()));
    let before_state = fs::read(&state_path).unwrap();
    let backup = root.join("display-backup");
    adapter.export_backup(&pid, &backup, &Job::new()).unwrap();
    let manifest = load_verified_bundle(&backup, &Job::new()).unwrap();
    assert_eq!(manifest["conversations"].as_array().unwrap().len(), 2);
    assert!(manifest["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["project_id"].is_null()));
    assert_eq!(fs::read(&state_path).unwrap(), before_state);
    assert!(adapter
        .delete(std::slice::from_ref(&pid), &[], false, &Job::new())
        .is_err());
    assert!(adapter
        .transfer_project(&pid, &[root.join("blocked")], true, &Job::new())
        .is_err());
    assert!(!root.join("blocked").exists());
    let plan = adapter.connection_plan(std::slice::from_ref(&pid)).unwrap();
    assert_eq!(plan["groups"][0]["threads"].as_array().unwrap().len(), 2);
    let mut stale = plan.clone();
    stale["revision"] = json!("stale");
    assert!(adapter.confirm_connections(&stale, &Job::new()).is_err());
    let cancelled = Job::new();
    cancelled.cancel();
    assert!(adapter.confirm_connections(&plan, &cancelled).is_err());
    let mut interrupted = plan.clone();
    interrupted["__testInterruptAt"] = json!("connection-assigned");
    assert!(adapter
        .confirm_connections(&interrupted, &Job::new())
        .unwrap_err()
        .contains("interruption"));
    let pending = adapter
        .journal
        .history()
        .unwrap()
        .into_iter()
        .find(|r| r["payload"]["displayConfirmation"] == true)
        .unwrap();
    assert_eq!(pending["state"], "needs_recovery");
    adapter.undo(pending["id"].as_str().unwrap()).unwrap();
    assert!(all_threads(&adapter.snapshot().unwrap())
        .iter()
        .all(|t| t["projectId"].is_null()));

    for action in ["move", "merge", "delete"] {
        let plan = adapter.connection_plan(std::slice::from_ref(&pid)).unwrap();
        let result = adapter.confirm_connections(&plan, &Job::new()).unwrap();
        let connection = result["operationIds"][0].as_str().unwrap();
        for id in &ids {
            assert_eq!(adapter.read_thread(id).unwrap()["projectId"], pid);
        }
        let op = match action {
            "move" => adapter
                .transfer_project(&pid, &[root.join("moved")], true, &Job::new())
                .unwrap(),
            "merge" => {
                let target = adapter.create("Target", &[]).unwrap();
                adapter
                    .merge(std::slice::from_ref(&pid), target["id"].as_str().unwrap())
                    .unwrap()
            }
            _ => adapter
                .delete(std::slice::from_ref(&pid), &[], false, &Job::new())
                .unwrap(),
        };
        assert!(
            adapter.undo(connection).is_err(),
            "must undo later operation first"
        );
        adapter.undo(&op).unwrap();
        pid = adapter.read_thread(&ids[0]).unwrap()["projectId"]
            .as_str()
            .unwrap()
            .to_owned();
        adapter.undo(connection).unwrap();
        for id in &ids {
            assert!(adapter.read_thread(id).unwrap()["projectId"].is_null());
        }
        assert_eq!(
            fs::read(workspace.join("keep.txt")).unwrap(),
            b"original bytes"
        );
        // Merge undo restores the empty target; its shared roots must not affect
        // subsequent inference (the original target had no roots).
        let source = adapter
            .snapshot()
            .unwrap()
            .projects
            .into_iter()
            .find(|g| g.project["id"] == pid)
            .unwrap();
        assert_eq!(source.threads.len(), 2);
    }
    // Cross-store preview includes inferred records without changing the source;
    // transfer pins the original null membership and both store revisions.
    adapter.release_server();
    let target_home = root.join("target-home");
    fs::create_dir(&target_home).unwrap();
    let mut target = Adapter::isolated(
        resolve_store(&target_home).unwrap(),
        adapter.binary.clone(),
        adapter.journal.clone(),
    )
    .unwrap();
    let mut request = json!({"kind":"projects","ids":[pid],"destination":root.join("cross-destination"),"copy":false});
    let preview = cross_store_plan(&adapter, &target, &request, &Job::new()).unwrap();
    assert_eq!(
        preview["projects"][0]["threads"].as_array().unwrap().len(),
        2
    );
    assert!(preview["projects"][0]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["projectId"].is_null()));
    request["expectedRevision"] = preview["sourceRevision"].clone();
    request["targetRevision"] = preview["targetRevision"].clone();
    let result = cross_store_transfer(&mut adapter, &mut target, &request, &Job::new()).unwrap();
    assert_eq!(
        adapter
            .journal
            .operation(result["operationId"].as_str().unwrap())
            .unwrap()
            .unwrap()["state"],
        "completed"
    );
    assert!(all_threads(&adapter.snapshot().unwrap()).is_empty());
    let received = all_threads(&target.snapshot().unwrap());
    assert_eq!(received.len(), 2);
    assert_eq!(received.iter().filter(|t| t["internal"] == true).count(), 1);
    assert!(received.iter().all(|t| t["projectId"].is_string()));
}

use codex_manager_core::*;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

fn fixture_adapter(label: &str) -> (PathBuf, Adapter) {
    let root = PathBuf::from(std::env::var_os("CODEX_TEST_HOME").unwrap()).join(label);
    assert!(!root.exists());
    fs::create_dir_all(root.join("home")).unwrap();
    let adapter = Adapter::isolated(
        resolve_store(&root.join("home")).unwrap(),
        PathBuf::from(std::env::var_os("CODEX_TEST_BINARY").unwrap()),
        Journal::open(&root.join("manager/journal.sqlite")).unwrap(),
    )
    .unwrap();
    (root, adapter)
}

fn fixture_thread(
    adapter: &mut Adapter,
    root: &std::path::Path,
    workspace: &std::path::Path,
    id: &str,
    project: &str,
    parent: Option<&str>,
) {
    fs::create_dir_all(workspace).unwrap();
    fs::write(workspace.join("keep.txt"), "fixture bytes").unwrap();
    let source = parent.map(|parent|json!({"subagent":{"thread_spawn":{"parent_thread_id":parent,"depth":1,"agent_nickname":"fixture","agent_role":"default"}}})).unwrap_or(json!("cli"));
    let meta = json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":workspace,"runtime_workspace_roots":[workspace],"originator":"codex_cli_rs","cli_version":"0.0.0","source":source,"model_provider":"openai"}});
    let event = json!({"timestamp":"2026-11-02T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"fictional fixture","images":[],"local_images":[],"text_elements":[]}});
    let path = root.join(format!("{id}.jsonl"));
    fs::write(&path, format!("{meta}\n{event}\n")).unwrap();
    adapter
        .import_thread(
            &path,
            id,
            workspace,
            &[workspace.to_owned()],
            project,
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

#[test]
#[ignore = "installed SDK with isolated home only"]
fn inferred_thread_actions_do_not_authorize_legacy_project_materialization() {
    for action in ["rename", "delete", "move"] {
        let (root, mut a) = fixture_adapter(&format!("legacy-selection-{action}"));
        let workspace = root.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let temporary = a.create("Fixture native", &[]).unwrap();
        let target = a.create("Move target", &[]).unwrap();
        let explicit = "00000000-0000-4000-8000-000000000011";
        let inferred = "00000000-0000-4000-8000-000000000012";
        for tid in [explicit, inferred] {
            fixture_thread(
                &mut a,
                &root,
                &workspace,
                tid,
                temporary["id"].as_str().unwrap(),
                None,
            );
        }
        a.call("project/delete", json!({"projectId":temporary["id"]}))
            .unwrap();
        a.release_server();
        let state_path = a.store.home.join(".codex-global-state.json");
        let state = json!({"local-projects":{"L":{"id":"L","name":"Legacy","rootPaths":[workspace]}},"thread-project-assignments":{explicit:{"projectKind":"local","projectId":"L"}}});
        fs::write(&state_path, state.to_string()).unwrap();
        let initial = a.snapshot().unwrap();
        assert_eq!(
            all_threads(&initial)
                .iter()
                .find(|t| t["id"] == inferred)
                .unwrap()["displayProjectId"],
            "L"
        );
        let mut selected = vec![inferred.to_owned()];
        if action == "move" {
            selected.push(target["id"].as_str().unwrap().to_owned());
        }
        assert!(a
            .materialize_legacy(&selected, &Job::new())
            .unwrap()
            .is_empty());
        match action {
            "rename" => {
                a.rename_threads(&[inferred.into()], &["Changed title".into()])
                    .unwrap();
            }
            "delete" => {
                a.delete(&[], &[inferred.into()], false, &Job::new())
                    .unwrap();
            }
            _ => {
                a.move_threads(&[inferred.into()], target["id"].as_str().unwrap())
                    .unwrap();
            }
        }
        assert!(a.read_thread(explicit).unwrap()["projectId"].is_null());
        let snapshot = a.snapshot().unwrap();
        assert_eq!(
            snapshot.projects.len(),
            2,
            "thread-only action must not create a native L"
        );
        assert!(snapshot
            .projects
            .iter()
            .any(|g| g.project["id"] == "L" && g.project["legacy"] == true));
        assert!(!a
            .journal
            .history()
            .unwrap()
            .iter()
            .any(|r| r["payload"]["kind"] == "rust-legacy"));
        let after: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
        assert_eq!(after["local-projects"]["L"], state["local-projects"]["L"]);
        assert_eq!(
            after["thread-project-assignments"][explicit],
            state["thread-project-assignments"][explicit]
        );
        // Selecting L itself or its authoritative E still authorizes conversion.
        let trigger = if action == "rename" { "L" } else { explicit };
        let mapping = a
            .materialize_legacy(&[trigger.into()], &Job::new())
            .unwrap();
        assert_eq!(a.read_thread(explicit).unwrap()["projectId"], mapping["L"]);
        let legacy = a
            .journal
            .history()
            .unwrap()
            .into_iter()
            .find(|r| r["payload"]["kind"] == "rust-legacy")
            .unwrap();
        assert_eq!(legacy["payload"]["threads"].as_array().unwrap().len(), 1);
        assert_eq!(legacy["payload"]["threads"][0]["id"], explicit);
        if action == "rename" {
            assert!(a.read_thread(inferred).unwrap()["projectId"].is_null());
        }
        if action == "move" {
            assert_eq!(a.read_thread(inferred).unwrap()["projectId"], target["id"]);
        }
    }
}

#[test]
#[ignore = "installed SDK with isolated home only"]
fn jointly_selected_protected_child_moves_parent_first_and_resumes_partial_cleanup() {
    for interrupted in [false, true] {
        let (root, mut a) = fixture_adapter(if interrupted {
            "protected-child-resume"
        } else {
            "protected-child-complete"
        });
        let temporary = a.create("Temporary", &[]).unwrap();
        let parent = "00000000-0000-4000-8000-000000000101";
        let child = "00000000-0000-4000-8000-000000000102";
        let other = "00000000-0000-4000-8000-000000000103";
        assert!(parent < child);
        fixture_thread(
            &mut a,
            &root,
            &root.join("parent-workspace"),
            parent,
            temporary["id"].as_str().unwrap(),
            None,
        );
        fixture_thread(
            &mut a,
            &root,
            &root.join("child-workspace"),
            child,
            temporary["id"].as_str().unwrap(),
            Some(parent),
        );
        fixture_thread(
            &mut a,
            &root,
            &root.join("other-workspace"),
            other,
            temporary["id"].as_str().unwrap(),
            None,
        );
        a.call("project/delete", json!({"projectId":temporary["id"]}))
            .unwrap();
        a.release_server();
        fs::write(
            a.store.home.join(".codex-global-state.json"),
            json!({"projectless-thread-ids":[child]}).to_string(),
        )
        .unwrap();
        fs::create_dir(root.join("target-home")).unwrap();
        let mut b = Adapter::isolated(
            resolve_store(&root.join("target-home")).unwrap(),
            a.binary.clone(),
            a.journal.clone(),
        )
        .unwrap();
        let snapshot = all_threads(&a.snapshot().unwrap());
        assert_eq!(
            snapshot.iter().find(|t| t["id"] == child).unwrap()["membershipReason"],
            "projectless"
        );
        let omitted =
            json!({"kind":"threads","ids":[parent],"destination":root.join("output"),"copy":false});
        assert!(cross_store_plan(&a, &b, &omitted, &Job::new())
            .unwrap_err()
            .contains("명시적으로"));
        let mut request = omitted;
        request["ids"] = json!([parent, child, other]);
        if interrupted {
            request["__testInterruptAt"] = json!("source-delete-next");
        }
        let preview = cross_store_plan(&a, &b, &request, &Job::new()).unwrap();
        assert_eq!(preview["threads"], json!([parent, child, other]));
        request["expectedRevision"] = preview["sourceRevision"].clone();
        request["targetRevision"] = preview["targetRevision"].clone();
        let result = cross_store_transfer(&mut a, &mut b, &request, &Job::new());
        if interrupted {
            assert!(result.unwrap_err().contains("source-delete-next"));
            let central = a
                .journal
                .history()
                .unwrap()
                .into_iter()
                .find(|r| r["payload"]["kind"] == "rust-cross-store")
                .unwrap();
            let op = central["id"].as_str().unwrap();
            let remaining = all_threads(&a.snapshot().unwrap());
            assert_eq!(remaining.len(), 1);
            assert_eq!(remaining[0]["id"], other);
            let child_journal =
                Journal::open(&root.join(format!("manager/cross-{op}-source.sqlite"))).unwrap();
            let deletion = child_journal
                .history()
                .unwrap()
                .into_iter()
                .find(|r| r["payload"]["kind"] == "rust-delete")
                .unwrap();
            assert_eq!(deletion["payload"]["threads"].as_array().unwrap().len(), 3);
            assert_ne!(deletion["state"], "completed");
            let target_file = PathBuf::from(preview["roots"][2]["destination"].as_str().unwrap())
                .join("keep.txt");
            fs::write(&target_file, "changed before second source delete").unwrap();
            assert!(cross_store_resume(&mut a, &mut b, op, &Job::new()).is_err());
            assert_eq!(
                all_threads(&a.snapshot().unwrap()).len(),
                1,
                "changed target must preserve remaining selected source record"
            );
            fs::write(&target_file, "fixture bytes").unwrap();
            // A new unapproved descendant must not be swept into the retry's
            // complete selected set. These writes are isolated fixture changes.
            let late = "00000000-0000-4000-8000-000000000104";
            let temporary = a.create("Late fixture", &[]).unwrap();
            fixture_thread(
                &mut a,
                &root,
                &root.join("late-workspace"),
                late,
                temporary["id"].as_str().unwrap(),
                Some(other),
            );
            a.call("project/delete", json!({"projectId":temporary["id"]}))
                .unwrap();
            a.release_server();
            fs::write(
                a.store.home.join(".codex-global-state.json"),
                json!({"projectless-thread-ids":[child]}).to_string(),
            )
            .unwrap();
            assert!(cross_store_resume(&mut a, &mut b, op, &Job::new())
                .unwrap_err()
                .contains("승인하지 않은"));
            assert_eq!(all_threads(&a.snapshot().unwrap()).len(), 2);
            a.call("thread/delete", json!({"threadId":late})).unwrap();
            a.release_server();
            cross_store_resume(&mut a, &mut b, op, &Job::new()).unwrap();
            cross_store_resume(&mut a, &mut b, op, &Job::new()).unwrap();
            assert_eq!(
                a.journal.operation(op).unwrap().unwrap()["state"],
                "completed"
            );
        } else {
            result.unwrap();
        }
        assert!(all_threads(&a.snapshot().unwrap()).is_empty());
        let received = all_threads(&b.snapshot().unwrap());
        assert_eq!(received.len(), 3);
        let t = received.iter().find(|t| t["id"] == child).unwrap();
        assert_eq!(t["parentThreadId"], parent);
        assert_eq!(t["internal"], true);
        assert!(received.iter().all(|t| t["projectId"].is_string()));
    }
}

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

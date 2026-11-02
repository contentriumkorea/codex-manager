use codex_manager_core::*;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};
fn fixture() -> (PathBuf, Adapter, Adapter) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(format!("cross-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let journal = Journal::open(&root.join("manager/journal.sqlite")).unwrap();
    let make = |name: &str| {
        let home = root.join(name);
        fs::create_dir(&home).unwrap();
        let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE projects(id TEXT,name TEXT); CREATE TABLE project_roots(project_id TEXT,path TEXT); CREATE TABLE threads(id TEXT,project_id TEXT,cwd TEXT,rollout_path TEXT,name TEXT,archived INTEGER);").unwrap();
        let workspace = root.join(format!("{name}-workspace"));
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("한글.txt"), "original").unwrap();
        db.execute("INSERT INTO projects VALUES('p', 'Project')", [])
            .unwrap();
        db.execute(
            "INSERT INTO project_roots VALUES('p', ?1)",
            [workspace.to_str().unwrap()],
        )
        .unwrap();
        drop(db);
        Adapter::isolated(
            resolve_store(&home).unwrap(),
            root.join("missing.exe"),
            journal.clone(),
        )
        .unwrap()
    };
    let a = make("source");
    let b = make("target");
    (root, a, b)
}
fn request(root: &std::path::Path) -> Value {
    json!({"kind":"projects","ids":["p"],"destination":root.join("destination"),"copy":false})
}
#[test]
fn preview_is_readonly_and_checks_aliases_and_protected_paths() {
    let (root, a, b) = fixture();
    let req = request(&root);
    let plan = cross_store_plan(&a, &b, &req, &Job::new()).unwrap();
    assert_eq!(plan["projects"].as_array().unwrap().len(), 1);
    assert_eq!(plan["roots"][0]["retainSource"], false);
    assert!(!root.join("destination").exists());
    assert!(a.journal.history().unwrap().is_empty());
    assert!(!a.store.home.join(".codex-manager-write.lock").exists());
    assert!(cross_store_plan(&a, &a, &req, &Job::new()).is_err());
    let mut req = req;
    req["destination"] = json!(a.store.home);
    assert!(cross_store_plan(&a, &b, &req, &Job::new()).is_err());
}
#[test]
fn files_validate_both_owners_nested_selection_and_no_overwrite() {
    let (root, a, b) = fixture();
    let file = root.join("source-workspace/한글.txt");
    let destination = root.join("target-workspace");
    let mut req = json!({"kind":"files","ids":[file],"destination":destination,"copy":true});
    assert!(cross_store_plan(&a, &b, &req, &Job::new()).is_err()); // duplicate filename
    fs::rename(destination.join("한글.txt"), destination.join("other.txt")).unwrap();
    assert!(cross_store_plan(&a, &b, &req, &Job::new()).is_ok());
    req["ids"] = json!([root.join("source-workspace"), file]);
    assert!(cross_store_plan(&a, &b, &req, &Job::new()).is_err());
    req["ids"] = json!([file]);
    req["destination"] = json!(root);
    assert!(cross_store_plan(&a, &b, &req, &Job::new()).is_err());
}
fn native_pair(label: &str) -> (PathBuf, Adapter, Adapter, Value, String, String) {
    let root = PathBuf::from(std::env::var_os("CODEX_TEST_HOME").expect("isolated fixture root"))
        .join(label);
    assert!(!root.exists());
    fs::create_dir_all(&root).unwrap();
    let journal = Journal::open(&root.join("manager/journal.sqlite")).unwrap();
    let binary = PathBuf::from(std::env::var_os("CODEX_TEST_BINARY").unwrap());
    let make = |name: &str| {
        let home = root.join(name);
        fs::create_dir(&home).unwrap();
        Adapter::isolated(
            resolve_store(&home).unwrap(),
            binary.clone(),
            journal.clone(),
        )
        .unwrap()
    };
    let mut source = make("source");
    let target = make("target");
    let workspace = root.join("workspace-한글");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("bytes.txt"), b"verified workspace bytes").unwrap();
    let project = source
        .create("원본 이름", std::slice::from_ref(&workspace))
        .unwrap();
    let mut add = |title: &str| {
        let id = uuid::Uuid::new_v4().to_string();
        let path = root.join(format!("{id}.jsonl"));
        let meta = json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":workspace,"runtime_workspace_roots":[workspace],"originator":"codex_cli_rs","cli_version":"0.0.0","source":"cli","model_provider":"openai"}});
        let message = json!({"timestamp":"2026-11-02T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"actual content 한글"}]}});
        fs::write(&path, format!("{meta}\n{message}\n")).unwrap();
        source
            .import_thread(
                &path,
                &id,
                &workspace,
                std::slice::from_ref(&workspace),
                project["id"].as_str().unwrap(),
                title,
            )
            .unwrap();
        id
    };
    let first = add("첫 대화");
    let second = add("남은 대화");
    source.release_server();
    (root, source, target, project, first, second)
}
fn planned(a: &Adapter, b: &Adapter, mut request: Value) -> Value {
    let plan = cross_store_plan(a, b, &request, &Job::new()).unwrap();
    request["expectedRevision"] = plan["sourceRevision"].clone();
    request["targetRevision"] = plan["targetRevision"].clone();
    request
}
#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_copy_partial_move_merge_and_full_project_move() {
    for (label, copy, partial) in [
        ("copy", true, false),
        ("partial", false, true),
        ("full", false, false),
    ] {
        let (root, mut a, mut b, p, first, second) = native_pair(label);
        if copy {
            fn old_runtime_shape(value: &mut Value) {
                match value {
                    Value::Object(map) => {
                        map.remove("runtime_workspace_roots");
                        for v in map.values_mut() {
                            old_runtime_shape(v);
                        }
                    }
                    Value::Array(items) => {
                        for v in items {
                            old_runtime_shape(v);
                        }
                    }
                    _ => {}
                }
            }
            let thread = all_threads(&a.snapshot().unwrap())
                .into_iter()
                .find(|t| t["id"] == first)
                .unwrap();
            let path = thread["path"].as_str().unwrap();
            let rewritten = fs::read_to_string(path)
                .unwrap()
                .lines()
                .map(|line| {
                    let mut v: Value = serde_json::from_str(line).unwrap();
                    old_runtime_shape(&mut v);
                    v.to_string() + "\n"
                })
                .collect::<String>();
            fs::write(path, rewritten).unwrap();
            assert!(runtime_roots(std::path::Path::new(path))
                .unwrap()
                .is_empty());
        }
        let target = if partial {
            {
                let existing = root.join("existing-target");
                fs::create_dir(&existing).unwrap();
                fs::write(existing.join("unrelated.txt"), "keep existing").unwrap();
                Some(b.create("대상 이름 유지", &[existing]).unwrap())
            }
        } else {
            None
        };
        b.release_server();
        let request = planned(
            &a,
            &b,
            json!({"kind":if partial{"threads"}else{"projects"},"ids":if partial{json!([first])}else{json!([p["id"]])},"destination":if partial{root.join("existing-target")}else{root.join("output")},"targetProject":target.as_ref().map(|p|&p["id"]),"copy":copy}),
        );
        let preview = cross_store_plan(&a, &b, &request, &Job::new()).unwrap();
        fs::write(
            root.join("workspace-한글/bytes.txt"),
            "changed after preview",
        )
        .unwrap();
        assert!(cross_store_transfer(&mut a, &mut b, &request, &Job::new())
            .unwrap_err()
            .contains("미리보기 이후"));
        assert!(a.journal.history().unwrap().is_empty());
        fs::write(
            root.join("workspace-한글/bytes.txt"),
            b"verified workspace bytes",
        )
        .unwrap();
        let result = cross_store_transfer(&mut a, &mut b, &request, &Job::new()).unwrap();
        assert_eq!(
            a.journal
                .operation(result["operationId"].as_str().unwrap())
                .unwrap()
                .unwrap()["state"],
            "completed"
        );
        assert!(!root.join("manager/recovery").exists());
        if !copy {
            assert!(result["retainedRecovery"]["bytes"].as_u64().unwrap() > 0);
            assert!(
                PathBuf::from(result["retainedRecovery"]["path"].as_str().unwrap())
                    .starts_with(PathBuf::from(preview["stagingBase"].as_str().unwrap()))
            );
        }
        let destination = PathBuf::from(preview["roots"][0]["destination"].as_str().unwrap());
        assert_eq!(
            fs::read(destination.join("bytes.txt")).unwrap(),
            b"verified workspace bytes"
        );
        let read = b.read_thread(&first).unwrap();
        assert_eq!(read["name"], "첫 대화");
        assert_eq!(
            clean_path(std::path::Path::new(read["cwd"].as_str().unwrap())),
            destination
        );
        let catalog = b.snapshot().unwrap();
        assert_eq!(catalog.projects.len(), 1);
        assert_eq!(read["projectId"], catalog.projects[0].project["id"]);
        if partial {
            assert_eq!(catalog.projects[0].project["name"], "대상 이름 유지");
            assert_eq!(
                roots(&catalog.projects[0].project).unwrap(),
                vec![root.join("existing-target")]
            );
            assert_eq!(
                fs::read(root.join("existing-target/unrelated.txt")).unwrap(),
                b"keep existing"
            );
            let exported = backup_project(
                &catalog,
                catalog.projects[0].project["id"].as_str().unwrap(),
                &root.join("re-export"),
                &Job::new(),
            )
            .unwrap();
            assert!(exported["dependencies"].as_array().unwrap().is_empty());
        }
        let imported = all_threads(&catalog)
            .into_iter()
            .find(|t| t["id"] == first)
            .unwrap();
        assert!(fs::read_to_string(imported["path"].as_str().unwrap())
            .unwrap()
            .contains("actual content 한글"));
        assert_eq!(all_threads(&catalog).len(), if partial { 1 } else { 2 });
        b.release_server();
        let source_threads = all_threads(&a.snapshot().unwrap());
        assert_eq!(source_threads.iter().any(|t| t["id"] == first), copy);
        assert_eq!(
            source_threads.iter().any(|t| t["id"] == second),
            copy || partial
        );
        assert_eq!(root.join("workspace-한글").exists(), copy || partial);
        if partial {
            assert_eq!(
                fs::read(root.join("workspace-한글/bytes.txt")).unwrap(),
                b"verified workspace bytes"
            );
        }
        if copy {
            for mode in [true, false] {
                let mut conflicting = request.clone();
                conflicting["copy"] = json!(mode);
                conflicting["destination"] = json!(root.join("other"));
                assert!(cross_store_plan(&a, &b, &conflicting, &Job::new())
                    .unwrap_err()
                    .contains("같은 ID"));
            }
        }
    }
}
#[test]
fn preview_revision_covers_bytes_and_shared_roots_are_retained() {
    let (root, a, b) = fixture();
    let request = request(&root);
    let before = cross_store_plan(&a, &b, &request, &Job::new()).unwrap();
    fs::write(root.join("source-workspace/한글.txt"), "changed").unwrap();
    let after = cross_store_plan(&a, &b, &request, &Job::new()).unwrap();
    assert_ne!(before["sourceRevision"], after["sourceRevision"]);
    let db = rusqlite::Connection::open(a.store.database.as_ref().unwrap()).unwrap();
    db.execute("INSERT INTO projects VALUES('shared','Shared')", [])
        .unwrap();
    db.execute(
        "INSERT INTO project_roots VALUES('shared',?1)",
        [root.join("source-workspace").to_str().unwrap()],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        cross_store_plan(&a, &b, &request, &Job::new()).unwrap()["roots"][0]["retainSource"],
        true
    );
    let mut alias =
        Adapter::isolated(b.store.clone(), b.binary.clone(), b.journal.clone()).unwrap();
    alias.store.sqlite_home = a.store.sqlite_home.clone();
    assert!(cross_store_plan(&a, &alias, &request, &Job::new()).is_err());
}
#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_interrupted_transfer_resumes_without_duplicates_and_checks_changed_inputs() {
    for stop in ["bundle-created", "imported", "target-verified", "root-held"] {
        let (root, mut a, mut b, p, first, _) = native_pair(stop);
        let req = planned(
            &a,
            &b,
            json!({"kind":"projects","ids":[p["id"]],"destination":root.join("output"),"copy":false,"__testInterruptAt":stop}),
        );
        assert!(cross_store_transfer(&mut a, &mut b, &req, &Job::new())
            .unwrap_err()
            .contains("Isolated fixture interruption"));
        let history = a.journal.history().unwrap();
        assert_eq!(history.len(), 1);
        let op = history[0]["id"].as_str().unwrap();
        assert!(a.keep_current(op, &Job::new()).is_err());
        assert!(a.undo(op).is_err());
        let plan = &history[0]["payload"]["plan"];
        let dst = PathBuf::from(plan["roots"][0]["destination"].as_str().unwrap());
        if stop == "target-verified" {
            fs::write(dst.join("bytes.txt"), "edited target").unwrap();
            assert!(cross_store_resume(&mut a, &mut b, op, &Job::new()).is_err());
            assert!(all_threads(&a.snapshot().unwrap())
                .iter()
                .any(|t| t["id"] == first));
            fs::write(dst.join("bytes.txt"), b"verified workspace bytes").unwrap();
            fs::write(root.join("workspace-한글/bytes.txt"), "edited source").unwrap();
            assert!(cross_store_resume(&mut a, &mut b, op, &Job::new()).is_err());
            fs::write(
                root.join("workspace-한글/bytes.txt"),
                b"verified workspace bytes",
            )
            .unwrap();
        }
        if stop == "root-held" {
            let hold = root.join(format!(".codex-manager-transfer-{op}-0-hold"));
            assert!(hold.is_dir());
            // Simulates process loss after the first verified file was removed.
            fs::remove_file(hold.join("bytes.txt")).unwrap();
        }
        cross_store_resume(&mut a, &mut b, op, &Job::new()).unwrap();
        cross_store_resume(&mut a, &mut b, op, &Job::new()).unwrap();
        assert_eq!(b.snapshot().unwrap().projects.len(), 1);
        assert_eq!(all_threads(&b.snapshot().unwrap()).len(), 2);
        assert!(all_threads(&a.snapshot().unwrap()).is_empty());
        assert!(!root.join("workspace-한글").exists());
        assert_eq!(
            fs::read(dst.join("bytes.txt")).unwrap(),
            b"verified workspace bytes"
        );
        for e in a.journal.events(op).unwrap() {
            if e["phase"] == "bundle-ready" {
                assert!(!PathBuf::from(e["payload"]["path"].as_str().unwrap())
                    .join("files/root-01")
                    .exists());
            }
        }
    }
}
#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_files_and_legacy_empty_projects() {
    let (root, mut a, mut b, _, _, _) = native_pair("files-legacy");
    let destination = root.join("target-files");
    fs::create_dir(&destination).unwrap();
    b.create("Files", std::slice::from_ref(&destination))
        .unwrap();
    b.release_server();
    let req = planned(
        &a,
        &b,
        json!({"kind":"files","ids":[root.join("workspace-한글/bytes.txt")],"destination":destination,"copy":false,"__testInterruptAt":"target-verified"}),
    );
    assert!(cross_store_transfer(&mut a, &mut b, &req, &Job::new()).is_err());
    let op = a.journal.history().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let cancelled = Job::new();
    cancelled.cancel();
    assert!(cross_store_resume(&mut a, &mut b, &op, &cancelled).is_err());
    cross_store_resume(&mut a, &mut b, &op, &Job::new()).unwrap();
    assert!(!root.join("workspace-한글/bytes.txt").exists());
    assert_eq!(
        fs::read(destination.join("bytes.txt")).unwrap(),
        b"verified workspace bytes"
    );
    let legacy_root = root.join("legacy-root");
    fs::create_dir(&legacy_root).unwrap();
    fs::write(legacy_root.join("legacy.txt"), "legacy bytes").unwrap();
    let source_state = a.store.home.join(".codex-global-state.json");
    let mut state: Value = fs::read(&source_state)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(json!({}));
    state["local-projects"]["old-source"] =
        json!({"id":"old-source","name":"Legacy source","rootPaths":[legacy_root]});
    fs::write(&source_state, state.to_string()).unwrap();
    let target_state = b.store.home.join(".codex-global-state.json");
    let mut state: Value = fs::read(&target_state)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(json!({}));
    state["local-projects"]["old-target"] =
        json!({"id":"old-target","name":"Legacy target","rootPaths":[]});
    fs::write(&target_state, state.to_string()).unwrap();
    let req = planned(
        &a,
        &b,
        json!({"kind":"projects","ids":["old-source"],"targetProject":"old-target","destination":root.join("legacy-out"),"copy":false}),
    );
    cross_store_transfer(&mut a, &mut b, &req, &Job::new()).unwrap();
    assert!(!legacy_root.exists());
    assert!(b
        .snapshot()
        .unwrap()
        .projects
        .iter()
        .any(|g| g.project["name"] == "Legacy target" && !roots(&g.project).unwrap().is_empty()));
}
#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_legacy_conversations_materialize_and_move_idempotently() {
    let (root, mut a, mut b, p, first, second) = native_pair("legacy-conversations");
    for tid in [&first, &second] {
        a.call(
            "thread/metadata/update",
            json!({"threadId":tid,"projectId":""}),
        )
        .unwrap();
    }
    a.call("project/delete", json!({"projectId":p["id"]}))
        .unwrap();
    a.release_server();
    fs::write(a.store.home.join(".codex-global-state.json"),json!({"local-projects":{"legacy":{"id":"legacy","name":"Legacy real conversations","rootPaths":[root.join("workspace-한글")]}},"thread-project-assignments":{first.clone():{"projectKind":"local","projectId":"legacy"},second.clone():{"projectKind":"local","projectId":"legacy"}},"app-server-project-id-by-legacy-project-id-by-host":{}}).to_string()).unwrap();
    let req = planned(
        &a,
        &b,
        json!({"kind":"projects","ids":["legacy"],"destination":root.join("legacy-destination"),"copy":false,"__testInterruptAt":"target-verified"}),
    );
    assert!(a
        .snapshot()
        .unwrap()
        .projects
        .iter()
        .any(|g| g.project["id"] == "legacy" && g.threads.len() == 2));
    assert!(cross_store_transfer(&mut a, &mut b, &req, &Job::new()).is_err());
    let op = a.journal.history().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    cross_store_resume(&mut a, &mut b, &op, &Job::new()).unwrap();
    assert!(a.snapshot().unwrap().projects.is_empty());
    assert!(!root.join("workspace-한글").exists());
    assert_eq!(b.snapshot().unwrap().projects.len(), 1);
    assert_eq!(all_threads(&b.snapshot().unwrap()).len(), 2);
    assert_eq!(b.read_thread(&first).unwrap()["name"], "첫 대화");
}
#[test]
fn target_store_references_protect_original_roots_and_file_directories() {
    let (root, a, b) = fixture();
    let workspace = root.join("source-workspace");
    let directory = workspace.join("nested");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("keep.txt"), "target still uses this").unwrap();
    let db = rusqlite::Connection::open(b.store.database.as_ref().unwrap()).unwrap();
    db.execute(
        "INSERT INTO projects VALUES('shared-target','Shared target')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO project_roots VALUES('shared-target',?1)",
        [workspace.to_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        cross_store_plan(&a, &b, &request(&root), &Job::new()).unwrap()["roots"][0]["retainSource"],
        true
    );
    db.execute(
        "DELETE FROM project_roots WHERE project_id='shared-target'",
        [],
    )
    .unwrap();
    let rollout = b.store.home.join("target-reference.jsonl");
    let id = "00000000-0000-4000-8000-000000000121";
    let files = json!({"kind":"files","ids":[directory],"destination":root.join("target-workspace"),"copy":false});
    for secondary in [false, true] {
        let cwd = if secondary {
            root.join("target-workspace")
        } else {
            directory.clone()
        };
        let runtime = if secondary {
            vec![cwd.clone(), directory.clone()]
        } else {
            vec![cwd.clone()]
        };
        fs::write(&rollout,format!("{}\n",json!({"type":"session_meta","payload":{"id":id,"cwd":cwd,"runtime_workspace_roots":runtime}}))).unwrap();
        db.execute("DELETE FROM threads WHERE id=?1", [id]).unwrap();
        db.execute(
            "INSERT INTO threads VALUES(?1,'shared-target',?2,?3,'Target reference',0)",
            rusqlite::params![id, cwd.to_str().unwrap(), rollout.to_str().unwrap()],
        )
        .unwrap();
        assert_eq!(
            cross_store_plan(&a, &b, &request(&root), &Job::new()).unwrap()["roots"][0]
                ["retainSource"],
            true
        );
        assert!(cross_store_plan(&a, &b, &files, &Job::new()).is_err());
        let mut copy = files.clone();
        copy["copy"] = json!(true);
        assert!(cross_store_plan(&a, &b, &copy, &Job::new()).is_ok());
    }
}

#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_other_store_references_preserve_source_on_move_and_resume() {
    for boundary in [
        "existing",
        "target-verified",
        "root-held",
        "file-target-verified",
    ] {
        let (root, mut a, mut b, p, _, _) = native_pair(&format!("target-protection-{boundary}"));
        let workspace = root.join("workspace-한글");
        let files = boundary == "file-target-verified";
        let original = if files {
            let path = workspace.join("nested");
            fs::create_dir(&path).unwrap();
            fs::write(path.join("bytes.txt"), b"verified workspace bytes").unwrap();
            path
        } else {
            workspace.clone()
        };
        let unrelated_root = root.join("unrelated-target");
        fs::create_dir(&unrelated_root).unwrap();
        let other = b
            .create("Unrelated target", std::slice::from_ref(&unrelated_root))
            .unwrap();
        let other_id = uuid::Uuid::new_v4().to_string();
        let rollout = root.join("unrelated-target.jsonl");
        fs::write(&rollout,format!("{}\n",json!({"type":"session_meta","timestamp":"2026-11-02T00:00:00Z","payload":{"id":other_id,"timestamp":"2026-11-02T00:00:00Z","cwd":unrelated_root,"runtime_workspace_roots":[unrelated_root],"originator":"codex_cli_rs","cli_version":"0.0.0","source":"cli","model_provider":"openai"}}))).unwrap();
        b.import_thread(
            &rollout,
            &other_id,
            &unrelated_root,
            std::slice::from_ref(&unrelated_root),
            other["id"].as_str().unwrap(),
            "Unrelated conversation",
        )
        .unwrap();
        b.release_server();
        if boundary == "existing" {
            b.set_roots(
                other["id"].as_str().unwrap(),
                std::slice::from_ref(&workspace),
            )
            .unwrap();
            b.release_server();
        }
        let req = planned(
            &a,
            &b,
            json!({"kind":if files{"files"}else{"projects"},"ids":if files{json!([original])}else{json!([p["id"]])},"destination":if files{unrelated_root.clone()}else{root.join("output")},"copy":false,"__testInterruptAt":if boundary=="existing"{Value::Null}else if files{json!("target-verified")}else{json!(boundary)}}),
        );
        if boundary == "existing" {
            assert_eq!(
                cross_store_plan(&a, &b, &req, &Job::new()).unwrap()["roots"][0]["retainSource"],
                true
            );
            cross_store_transfer(&mut a, &mut b, &req, &Job::new()).unwrap();
        } else {
            assert!(cross_store_transfer(&mut a, &mut b, &req, &Job::new())
                .unwrap_err()
                .contains("Isolated fixture interruption"));
            let op = a
                .journal
                .history()
                .unwrap()
                .into_iter()
                .find(|r| r["payload"]["kind"] == "rust-cross-store")
                .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
            // Add a runtime reference after preview/target verification. The
            // existing target project's own roots and imported project are unchanged.
            let thread = all_threads(&b.snapshot().unwrap())
                .into_iter()
                .find(|t| t["id"] == other_id)
                .unwrap();
            let target_rollout = PathBuf::from(thread["path"].as_str().unwrap());
            use std::io::Write;
            writeln!(fs::OpenOptions::new().append(true).open(target_rollout).unwrap(),"{}",json!({"type":"turn_context","payload":{"runtime_workspace_roots":[unrelated_root,original]}})).unwrap();
            assert!(cross_store_resume(&mut a, &mut b, &op, &Job::new()).is_err());
        }
        assert_eq!(
            fs::read(original.join("bytes.txt")).unwrap(),
            b"verified workspace bytes"
        );
        assert!(all_threads(&b.snapshot().unwrap())
            .iter()
            .any(|t| t["id"] == other_id));
    }
}

#[test]
fn long_unicode_project_title_keeps_display_name_and_creatable_generated_components() {
    let (root, a, b) = fixture();
    let title = "한글🌱".repeat(85) + "끝";
    assert_eq!(title.chars().count(), 256);
    assert_eq!(valid_name(&title).unwrap(), title);
    let original = root.join("source-workspace");
    let long_root = root.join("긴🌱".repeat(80));
    fs::rename(&original, &long_root).unwrap();
    let db = rusqlite::Connection::open(a.store.home.join("state_5.sqlite")).unwrap();
    db.execute("UPDATE projects SET name=?1 WHERE id='p'", [&title])
        .unwrap();
    db.execute(
        "UPDATE project_roots SET path=?1 WHERE project_id='p'",
        [long_root.to_str().unwrap()],
    )
    .unwrap();
    drop(db);
    let plan = cross_store_plan(&a, &b, &request(&root), &Job::new()).unwrap();
    assert_eq!(plan["projects"][0]["project"]["name"], title);
    let destination = PathBuf::from(plan["roots"][0]["destination"].as_str().unwrap());
    for component in destination
        .strip_prefix(root.join("destination"))
        .unwrap()
        .components()
    {
        // Windows components are limited by UTF-16 units, not scalar count.
        assert!(
            component
                .as_os_str()
                .to_string_lossy()
                .encode_utf16()
                .count()
                <= 255
        );
    }
    fs::create_dir_all(&destination).unwrap();
    assert_eq!(
        fs::read_to_string(long_root.join("한글.txt")).unwrap(),
        "original"
    );
    fs::write(destination.join("실제 파일.txt"), "creatable").unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("실제 파일.txt")).unwrap(),
        "creatable"
    );
}

#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_unicode_multi_root_project_move_preserves_both_runtime_roots() {
    let (root, mut a, mut b, old_project, _first, _second) = native_pair("unicode-multi-root");
    let original = root.join("workspace-한글");
    let secondary = root.join("보조 작업-🌱");
    fs::create_dir(&secondary).unwrap();
    fs::write(secondary.join("연결 자료.txt"), "두 번째 작업 폴더 내용").unwrap();
    // Recreate one selected project with two roots and one actual conversation.
    a.delete(
        &[old_project["id"].as_str().unwrap().to_owned()],
        &[],
        true,
        &Job::new(),
    )
    .unwrap();
    let project = a
        .create(
            "두 작업 폴더 프로젝트",
            &[original.clone(), secondary.clone()],
        )
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let rollout = root.join("multi-root.jsonl");
    let meta = json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":original,"runtime_workspace_roots":[original,secondary],"originator":"codex_cli_rs","cli_version":"0.0.0","source":"cli","model_provider":"openai"}});
    let message = json!({"timestamp":"2026-11-02T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"두 폴더의 파일과 대화를 함께 이동"}]}});
    fs::write(&rollout, format!("{meta}\n{message}\n")).unwrap();
    a.import_thread(
        &rollout,
        &id,
        &original,
        &[original.clone(), secondary.clone()],
        project["id"].as_str().unwrap(),
        "한글 대화 제목 🌱",
    )
    .unwrap();
    a.release_server();
    let req = planned(
        &a,
        &b,
        json!({"kind":"projects","ids":[project["id"]],"destination":root.join("대상 폴더"),"copy":false}),
    );
    let preview = cross_store_plan(&a, &b, &req, &Job::new()).unwrap();
    assert_eq!(preview["roots"].as_array().unwrap().len(), 2);
    let mapped = |source: &PathBuf| {
        PathBuf::from(
            preview["roots"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| std::path::Path::new(r["source"].as_str().unwrap()) == source)
                .unwrap()["destination"]
                .as_str()
                .unwrap(),
        )
    };
    let main_target = mapped(&original);
    let secondary_target = mapped(&secondary);
    cross_store_transfer(&mut a, &mut b, &req, &Job::new()).unwrap();
    assert_eq!(
        fs::read(main_target.join("bytes.txt")).unwrap(),
        b"verified workspace bytes"
    );
    assert_eq!(
        fs::read_to_string(secondary_target.join("연결 자료.txt")).unwrap(),
        "두 번째 작업 폴더 내용"
    );
    let read = b.read_thread(&id).unwrap();
    assert_eq!(read["name"], "한글 대화 제목 🌱");
    assert_eq!(
        clean_path(std::path::Path::new(read["cwd"].as_str().unwrap())),
        main_target
    );
    let catalog = b.snapshot().unwrap();
    assert_eq!(catalog.projects.len(), 1);
    assert_eq!(catalog.projects[0].project["name"], "두 작업 폴더 프로젝트");
    assert_eq!(read["projectId"], catalog.projects[0].project["id"]);
    let target_roots = roots(&catalog.projects[0].project).unwrap();
    assert_eq!(target_roots.len(), 2);
    assert!(target_roots.contains(&main_target));
    assert!(target_roots.contains(&secondary_target));
    let imported = all_threads(&catalog)
        .into_iter()
        .find(|t| t["id"] == id)
        .unwrap();
    let imported_path = std::path::Path::new(imported["path"].as_str().unwrap());
    let runtime = runtime_roots(imported_path)
        .unwrap()
        .into_iter()
        .map(|p| clean_path(&p))
        .collect::<Vec<_>>();
    assert_eq!(runtime.len(), 2);
    assert!(runtime.contains(&main_target));
    assert!(runtime.contains(&secondary_target));
    assert!(fs::read_to_string(imported_path)
        .unwrap()
        .contains("두 폴더의 파일과 대화를 함께 이동"));
    b.release_server();
    assert!(!all_threads(&a.snapshot().unwrap())
        .iter()
        .any(|t| t["id"] == id));
    assert!(a.snapshot().unwrap().projects.is_empty());
    assert!(!original.exists());
    assert!(!secondary.exists());
}

fn transfer_id(a: &Adapter) -> String {
    a.journal
        .history()
        .unwrap()
        .into_iter()
        .find(|r| r["payload"]["kind"] == "rust-cross-store")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn conclude(a: &mut Adapter, b: &mut Adapter, id: &str) -> Value {
    let before = digest(&a.journal.path, &Job::new()).unwrap();
    let preview = cross_store_conclude_plan(a, b, id, &Job::new()).unwrap();
    assert_eq!(
        before,
        digest(&a.journal.path, &Job::new()).unwrap(),
        "preview must be readonly"
    );
    let result = cross_store_conclude(a, b, id, &preview["revision"], &Job::new()).unwrap();
    assert_eq!(result["safeConclusion"], true);
    assert_eq!(result["transferCompleted"], false);
    assert_eq!(
        a.journal.operation(id).unwrap().unwrap()["state"],
        "cancelled"
    );
    let event_count = a.journal.events(id).unwrap().len();
    assert_eq!(
        cross_store_conclude(a, b, id, &Value::Null, &Job::new()).unwrap(),
        result
    );
    assert_eq!(a.journal.events(id).unwrap().len(), event_count);
    assert!(cross_store_resume(a, b, id, &Job::new()).is_err());
    result
}
fn harmless_both(root: &std::path::Path, a: &mut Adapter, b: &mut Adapter) {
    for (side, adapter) in [("source", a), ("target", b)] {
        let workspace = root.join(format!("after-{side}"));
        fs::create_dir(&workspace).unwrap();
        let p = adapter
            .create("After safe conclusion", &[workspace])
            .unwrap();
        adapter.release_server();
        let op = adapter
            .rename_project(p["id"].as_str().unwrap(), "Harmless journaled rename")
            .unwrap();
        adapter.release_server();
        assert_eq!(
            adapter.journal.operation(&op).unwrap().unwrap()["state"],
            "completed"
        );
    }
}
#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_safe_conclusion_preserves_conflicts_and_releases_both_homes() {
    for case in [
        "source-edit",
        "target-edit",
        "source-reference",
        "complete-hold",
        "partial-hold",
        "referenced-hold",
    ] {
        let (root, mut a, mut b, p, first, _) = native_pair(&format!("conclude-{case}"));
        let workspace = root.join("workspace-한글");
        fs::write(workspace.join("extra.txt"), b"second file for partial hold").unwrap();
        let boundary = if case.ends_with("hold") {
            "root-held"
        } else {
            "target-verified"
        };
        let req = planned(
            &a,
            &b,
            json!({"kind":"projects","ids":[p["id"]],"destination":root.join("output"),"copy":false,"__testInterruptAt":boundary}),
        );
        assert!(cross_store_transfer(&mut a, &mut b, &req, &Job::new()).is_err());
        let id = transfer_id(&a);
        let record = a.journal.operation(&id).unwrap().unwrap();
        let destination = PathBuf::from(
            record["payload"]["plan"]["roots"][0]["destination"]
                .as_str()
                .unwrap(),
        );
        let hold = root.join(format!(".codex-manager-transfer-{id}-0-hold"));
        match case {
            "source-edit" | "target-edit" => {
                let adapter = if case == "source-edit" { &a } else { &b };
                let t = all_threads(&adapter.snapshot().unwrap())
                    .into_iter()
                    .find(|t| t["id"] == first)
                    .unwrap();
                use std::io::Write;
                writeln!(fs::OpenOptions::new().append(true).open(t["path"].as_str().unwrap()).unwrap(),"{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"legitimate new content\"}}]}}}}").unwrap();
                fs::write(
                    if case == "source-edit" {
                        workspace.join("bytes.txt")
                    } else {
                        destination.join("bytes.txt")
                    },
                    b"legitimate edited workspace",
                )
                .unwrap();
            }
            "source-reference" => {
                a.create(
                    "New unrelated source reference",
                    std::slice::from_ref(&workspace),
                )
                .unwrap();
                a.release_server();
            }
            "partial-hold" => {
                fs::remove_file(hold.join("bytes.txt")).unwrap();
                fs::write(
                    destination.join("bytes.txt"),
                    b"changed target after partial cleanup",
                )
                .unwrap();
            }
            "referenced-hold" => {
                a.create("New reference to held files", std::slice::from_ref(&hold))
                    .unwrap();
                a.release_server();
                fs::write(
                    destination.join("bytes.txt"),
                    b"changed target with hold in use",
                )
                .unwrap();
            }
            "complete-hold" => {
                fs::write(destination.join("bytes.txt"), b"changed target after hold").unwrap();
            }
            _ => unreachable!(),
        }
        assert!(cross_store_resume(&mut a, &mut b, &id, &Job::new()).is_err());
        let preview = cross_store_conclude_plan(&a, &b, &id, &Job::new()).unwrap();
        if case == "source-edit" {
            fs::write(workspace.join("after-preview.txt"), b"new legitimate file").unwrap();
            assert!(
                cross_store_conclude(&mut a, &mut b, &id, &preview["revision"], &Job::new())
                    .unwrap_err()
                    .contains("미리보기")
            );
        }
        if case == "complete-hold" {
            assert_eq!(preview["roots"][0]["action"], "restore-complete-hold");
        }
        if case == "partial-hold" {
            assert_eq!(preview["roots"][0]["sourceUnavailable"], true);
        }
        if case == "complete-hold" {
            // Model success-before-final-recording in BOTH private journals.
            for side in ["source", "target"] {
                let journal =
                    Journal::open(&root.join(format!("manager/cross-{id}-{side}.sqlite"))).unwrap();
                for r in journal.history().unwrap() {
                    journal
                        .record(
                            r["id"].as_str().unwrap(),
                            "needs_recovery",
                            json!({"isolatedCrashAfterAppliedOutcome":true}),
                        )
                        .unwrap();
                }
            }
        }
        let result = conclude(&mut a, &mut b, &id);
        if case == "complete-hold" {
            for side in ["source", "target"] {
                let journal =
                    Journal::open(&root.join(format!("manager/cross-{id}-{side}.sqlite"))).unwrap();
                assert!(journal
                    .history()
                    .unwrap()
                    .iter()
                    .all(|r| ["completed", "cancelled"].contains(&r["state"].as_str().unwrap())));
            }
        }
        if case == "partial-hold" || case == "referenced-hold" {
            assert!(!workspace.exists());
            assert_eq!(
                fs::read(hold.join("extra.txt")).unwrap(),
                b"second file for partial hold"
            );
            assert_eq!(result["retained"]["roots"][0]["sourceUnavailable"], true);
        } else {
            assert!(workspace.join("extra.txt").exists());
        }
        if case == "source-edit" || case == "target-edit" {
            let adapter = if case == "source-edit" { &a } else { &b };
            let t = all_threads(&adapter.snapshot().unwrap())
                .into_iter()
                .find(|t| t["id"] == first)
                .unwrap();
            assert!(fs::read_to_string(t["path"].as_str().unwrap())
                .unwrap()
                .contains("legitimate new content"));
            assert_eq!(
                fs::read(if case == "source-edit" {
                    workspace.join("bytes.txt")
                } else {
                    destination.join("bytes.txt")
                })
                .unwrap(),
                b"legitimate edited workspace"
            );
        }
        if case == "source-reference" {
            assert!(a
                .snapshot()
                .unwrap()
                .projects
                .iter()
                .any(|g| g.project["name"] == "New unrelated source reference"));
        }
        harmless_both(&root, &mut a, &mut b);
    }
}
#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_interrupted_exports_are_bounded_before_retry_and_eventually_complete() {
    let (root, mut a, mut b, p, first, _) = native_pair("bounded-exports");
    let workspace = root.join("workspace-한글");
    fs::write(workspace.join("a-large.bin"), vec![19u8; 2 * 1024 * 1024]).unwrap();
    fs::write(workspace.join("z-large.bin"), vec![23u8; 2 * 1024 * 1024]).unwrap();
    let req = planned(
        &a,
        &b,
        json!({"kind":"projects","ids":[p["id"]],"destination":root.join("output"),"copy":false,"__testInterruptAt":"export-file-cancel","__testCancelAttempts":3}),
    );
    assert!(cross_store_transfer(&mut a, &mut b, &req, &Job::new()).is_err());
    let id = transfer_id(&a);
    for attempt in 1..=3 {
        let events = a.journal.events(&id).unwrap();
        let bundles = events
            .iter()
            .filter(|e| e["phase"] == "bundle-intent")
            .map(|e| PathBuf::from(e["payload"]["path"].as_str().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(bundles.len(), attempt);
        let live = bundles.iter().filter(|p| p.exists()).collect::<Vec<_>>();
        assert_eq!(
            live.len(),
            1,
            "old attempts must be retired before replacement export"
        );
        let tree = scan_tree(live[0], &Job::new()).unwrap();
        assert_eq!(
            tree.files.len(),
            1,
            "cancel after one completed file, before bundle ready"
        );
        assert_eq!(tree_size(&tree).unwrap(), 2 * 1024 * 1024);
        if attempt < 3 {
            assert!(cross_store_resume(&mut a, &mut b, &id, &Job::new()).is_err());
        }
    }
    cross_store_resume(&mut a, &mut b, &id, &Job::new()).unwrap();
    assert!(all_threads(&b.snapshot().unwrap())
        .iter()
        .any(|t| t["id"] == first));
    assert!(!all_threads(&a.snapshot().unwrap())
        .iter()
        .any(|t| t["id"] == first));
    assert!(!workspace.exists());
}
#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_unknown_changed_and_replaced_export_bytes_require_safe_conclusion() {
    for case in [
        "unknown-partial",
        "unreceipted-complete",
        "changed",
        "replaced",
    ] {
        let (root, mut a, mut b, p, _, _) = native_pair(&format!("export-{case}"));
        let req = planned(
            &a,
            &b,
            json!({"kind":"projects","ids":[p["id"]],"destination":root.join("output"),"copy":false,"__testInterruptAt":"export-file-cancel"}),
        );
        assert!(cross_store_transfer(&mut a, &mut b, &req, &Job::new()).is_err());
        let id = transfer_id(&a);
        let bundle = PathBuf::from(
            a.journal
                .events(&id)
                .unwrap()
                .iter()
                .find(|e| e["phase"] == "bundle-intent")
                .unwrap()["payload"]["path"]
                .as_str()
                .unwrap(),
        );
        let file = bundle.join("files/root-01/bytes.txt");
        let protected = if case == "unknown-partial" {
            let unknown = bundle.join("files/root-01/.manager-hard-crash.partial");
            fs::write(&unknown, b"unknown crash bytes").unwrap();
            unknown
        } else if case == "unreceipted-complete" {
            let unknown = bundle.join("files/root-01/completed-before-receipt.txt");
            fs::write(&unknown, b"completed copy with no ownership receipt").unwrap();
            unknown
        } else if case == "changed" {
            fs::write(&file, b"legitimate edited staging").unwrap();
            file.clone()
        } else {
            fs::rename(&file, bundle.join("retained-original.txt")).unwrap();
            fs::write(&file, b"verified workspace bytes").unwrap();
            file.clone()
        };
        let before = fs::read(&protected).unwrap();
        for _ in 0..2 {
            assert!(cross_store_resume(&mut a, &mut b, &id, &Job::new()).is_err());
        }
        assert_eq!(
            a.journal
                .events(&id)
                .unwrap()
                .iter()
                .filter(|e| e["phase"] == "bundle-intent")
                .count(),
            1
        );
        conclude(&mut a, &mut b, &id);
        assert_eq!(fs::read(&protected).unwrap(), before);
        assert!(root.join("workspace-한글/bytes.txt").exists());
        harmless_both(&root, &mut a, &mut b);
    }
}

#[test]
fn explicitly_selected_projectless_thread_keeps_equivalent_windows_runtime_root() {
    let (root, a, b) = fixture();
    let tid = "00000000-0000-4000-8000-000000000199";
    let workspace = root.join("source-workspace");
    let rollout = root.join("synthetic.jsonl");
    fs::write(&rollout,format!("{}\n",json!({"type":"session_meta","payload":{"id":tid,"cwd":workspace,"runtime_workspace_roots":[workspace]}}))).unwrap();
    let db = rusqlite::Connection::open(a.store.database.as_ref().unwrap()).unwrap();
    db.execute(
        "INSERT INTO threads VALUES(?1,NULL,?2,?3,'Synthetic',0)",
        rusqlite::params![
            tid,
            workspace.canonicalize().unwrap().to_str().unwrap(),
            rollout.to_str().unwrap()
        ],
    )
    .unwrap();
    fs::write(
        a.store.home.join(".codex-global-state.json"),
        json!({"projectless-thread-ids":[tid]}).to_string(),
    )
    .unwrap();
    let plan = cross_store_plan(&a,&b,&json!({"kind":"threads","ids":[tid],"destination":root.join("synthetic-output"),"copy":false}),&Job::new()).unwrap();
    assert_eq!(plan["projects"][0]["synthetic"], true);
    assert_eq!(plan["roots"].as_array().unwrap().len(), 1);
    assert_eq!(plan["roots"][0]["source"], json!(clean_path(&workspace)));
}

#[test]
#[ignore = "real installed Codex, distinct isolated stores only"]
fn native_compressed_backup_import_and_recovery_preserve_proofs() {
    let (root, mut source, mut target, project, first, second) = native_pair("compressed-native");
    source
        .call("thread/archive", json!({"threadId":first}))
        .unwrap();
    source.release_server();
    let original = all_threads(&source.snapshot().unwrap())
        .into_iter()
        .find(|t| t["id"] == first)
        .unwrap();
    let path = PathBuf::from(original["path"].as_str().unwrap());
    let bytes = fs::read(&path).unwrap();
    let proof = messages_digest(&path, &Job::new()).unwrap();
    let zst = path.with_extension("jsonl.zst");
    fs::write(&zst, zstd::stream::encode_all(bytes.as_slice(), 1).unwrap()).unwrap();
    fs::remove_file(&path).unwrap(); // isolated fixture models Codex compression; DB remains .jsonl
    assert_eq!(messages_digest(&path, &Job::new()).unwrap(), proof);
    assert_ne!(digest(&zst, &Job::new()).unwrap(), proof);
    let backup = root.join("backup");
    source
        .export_backup(project["id"].as_str().unwrap(), &backup, &Job::new())
        .unwrap();
    let manifest = load_verified_bundle(&backup, &Job::new()).unwrap();
    let saved = manifest["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == first)
        .unwrap();
    assert_eq!(
        fs::read(backup.join(saved["rollout"].as_str().unwrap())).unwrap(),
        fs::read(&zst).unwrap()
    );
    let destination = root.join("relocated-한글");
    target
        .import_backup(
            &backup,
            std::slice::from_ref(&destination),
            None,
            false,
            None,
            &Job::new(),
        )
        .unwrap();
    target.release_server();
    let restored = all_threads(&target.snapshot().unwrap());
    assert_eq!(restored.len(), 2);
    for (id, name, archived) in [(&first, "첫 대화", true), (&second, "남은 대화", false)] {
        let t = restored.iter().find(|t| t["id"] == *id).unwrap();
        assert_eq!(t["name"], name);
        assert_eq!(t["archived"], archived);
        assert_eq!(
            clean_path(std::path::Path::new(t["cwd"].as_str().unwrap())),
            destination
        );
        let file = PathBuf::from(t["path"].as_str().unwrap());
        assert_eq!(runtime_roots(&file).unwrap(), vec![destination.clone()]);
        if id == &first {
            assert_eq!(messages_digest(&file, &Job::new()).unwrap(), proof);
        }
    }
    assert_eq!(
        fs::read(destination.join("bytes.txt")).unwrap(),
        b"verified workspace bytes"
    );
    assert!(zst.exists());
    let deletion = source
        .delete(&[], std::slice::from_ref(&first), false, &Job::new())
        .unwrap();
    let event = source.journal.operation(&deletion).unwrap().unwrap();
    let recovery = PathBuf::from(event["payload"]["saved"][0]["path"].as_str().unwrap());
    assert_eq!(
        fs::read(&recovery).unwrap(),
        zstd::stream::encode_all(bytes.as_slice(), 1).unwrap()
    );
    source.undo(&deletion).unwrap();
    source.release_server();
    let recovered = all_threads(&source.snapshot().unwrap())
        .into_iter()
        .find(|t| t["id"] == first)
        .unwrap();
    assert_eq!(recovered["name"], "첫 대화");
    assert_eq!(recovered["archived"], true);
    assert_eq!(
        messages_digest(
            std::path::Path::new(recovered["path"].as_str().unwrap()),
            &Job::new()
        )
        .unwrap(),
        proof
    );
}

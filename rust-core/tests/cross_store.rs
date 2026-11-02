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

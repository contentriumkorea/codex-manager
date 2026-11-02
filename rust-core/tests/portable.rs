use codex_manager_core::*;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};
const ID: &str = "00000000-0000-4000-8000-000000000099";
fn fixture() -> (PathBuf, PathBuf, Catalog) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(uuid::Uuid::new_v4().to_string());
    let anchor = base.join("작업 폴더");
    fs::create_dir_all(&anchor).unwrap();
    fs::write(anchor.join("ordinary.txt"), b"ordinary bytes").unwrap();
    let source = base.join("source.jsonl");
    fs::write(&source,format!("{}\n{}\n",json!({"type":"session_meta","payload":{"id":ID,"cwd":anchor,"runtime_workspace_roots":[anchor]}}),json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"history"}]}}))).unwrap();
    let catalog = Catalog {
        projects: vec![ProjectGroup {
            project: json!({"id":"project-one","name":"Project","roots":[{"path":anchor}]}),
            threads: vec![
                json!({"id":ID,"projectId":"project-one","name":"Title","cwd":anchor,"path":source,"archived":false,"parentThreadId":null}),
            ],
        }],
        unassigned: vec![],
        unresolved: vec![],
    };
    (anchor, source, catalog)
}
fn snapshot(anchor: &std::path::Path, catalog: &Catalog) -> Value {
    portable_snapshot(catalog, "project-one", anchor, &Job::new()).unwrap()
}
#[test]
fn portable_reuses_blobs_isolates_projects_and_never_copies_ordinary_files() {
    let (anchor, _, mut c) = fixture();
    let a = snapshot(&anchor, &c);
    let ns = a["namespace"].as_str().unwrap();
    assert_eq!(a["complete"], true);
    assert_eq!(a["copiedBlobs"], 1);
    let b = snapshot(&anchor, &c);
    assert_eq!(b["copiedBlobs"], 0);
    assert_eq!(b["reusedBlobs"], 1);
    let inspected = portable_inspect(&anchor, ns, &Job::new()).unwrap();
    assert_eq!(inspected["conversations"].as_array().unwrap().len(), 1);
    assert!(!anchor
        .join(".codex-manager/projects")
        .join(ns)
        .join("ordinary.txt")
        .exists());
    c.projects[0].project["id"] = json!("project-two");
    c.projects[0].threads[0]["projectId"] = json!("project-two");
    let other = portable_snapshot(&c, "project-two", &anchor, &Job::new()).unwrap();
    assert_ne!(a["namespace"], other["namespace"]);
    assert_eq!(
        portable_discover(anchor.parent().unwrap(), 4, 100, &Job::new()).unwrap()["projects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn cancellation_corruption_and_dependencies_preserve_latest_good() {
    let (anchor, source, mut c) = fixture();
    let a = snapshot(&anchor, &c);
    let ns = a["namespace"].as_str().unwrap();
    let job = Job::new();
    job.cancel();
    assert!(portable_snapshot(&c, "project-one", &anchor, &job).is_err());
    fs::write(&source, b"bad json\n").unwrap();
    assert!(portable_snapshot(&c, "project-one", &anchor, &Job::new()).is_err());
    assert_eq!(
        portable_status(&anchor, ns).unwrap()["snapshotId"],
        a["snapshotId"]
    );
    c.projects[0].project["roots"]
        .as_array_mut()
        .unwrap()
        .push(json!({"path":anchor.parent().unwrap().join("external")}));
    assert!(portable_snapshot(&c, "project-one", &anchor, &Job::new()).is_err());
    assert_eq!(
        portable_status(&anchor, ns).unwrap()["snapshotId"],
        a["snapshotId"]
    );
    assert!(portable_inspect(&anchor, "../escape", &Job::new()).is_err());
}
#[test]
fn changed_compressed_history_and_bounded_retention() {
    let (anchor, source, c) = fixture();
    let raw = fs::read(&source).unwrap();
    fs::write(
        source.with_extension("jsonl.zst"),
        zstd::stream::encode_all(raw.as_slice(), 1).unwrap(),
    )
    .unwrap();
    fs::remove_file(&source).unwrap();
    let first = snapshot(&anchor, &c);
    let ns = first["namespace"].as_str().unwrap();
    for i in 0..5 {
        let text = format!(
            "{}{}\n",
            String::from_utf8(raw.clone()).unwrap(),
            json!({"type":"event_msg","payload":{"type":"test","i":i}})
        );
        fs::write(
            source.with_extension("jsonl.zst"),
            zstd::stream::encode_all(text.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        snapshot(&anchor, &c);
    }
    let dir = anchor.join(".codex-manager/projects").join(ns);
    assert_eq!(fs::read_dir(dir.join("manifests")).unwrap().count(), 2);
    assert_eq!(fs::read_dir(dir.join("blobs")).unwrap().count(), 2);
    let inspected = portable_inspect(&anchor, ns, &Job::new()).unwrap();
    let hash = inspected["conversations"][0]["sha256"].as_str().unwrap();
    fs::write(dir.join("blobs").join(hash), b"tampered").unwrap();
    assert!(portable_inspect(&anchor, ns, &Job::new()).is_err());
}
#[test]
#[ignore = "installed native SDK; isolated fixture homes only"]
fn native_folder_move_restore_preserves_graph_and_conflicts() {
    let base = PathBuf::from(std::env::var_os("CODEX_TEST_HOME").unwrap()).join("portable-native");
    assert!(!base.exists());
    fs::create_dir_all(&base).unwrap();
    let binary = PathBuf::from(std::env::var_os("CODEX_TEST_BINARY").unwrap());
    let journal = Journal::open(&base.join("manager/journal.sqlite")).unwrap();
    let make = |name: &str| {
        let home = base.join(name);
        fs::create_dir(&home).unwrap();
        Adapter::isolated(
            resolve_store(&home).unwrap(),
            binary.clone(),
            journal.clone(),
        )
        .unwrap()
    };
    let mut source = make("source");
    let mut target = make("target");
    let anchor = base.join("원본 폴더");
    fs::create_dir(&anchor).unwrap();
    fs::write(anchor.join("normal.txt"), b"normal original bytes").unwrap();
    let project = source
        .create("Portable project", std::slice::from_ref(&anchor))
        .unwrap();
    let parent = uuid::Uuid::new_v4().to_string();
    let child = uuid::Uuid::new_v4().to_string();
    for (id, title, is_child) in [(&parent, "Parent", false), (&child, "Child", true)] {
        let path = base.join(format!("{id}.jsonl"));
        let origin = if is_child {
            json!({"subagent":{"thread_spawn":{"parent_thread_id":parent,"depth":1}}})
        } else {
            json!("cli")
        };
        let meta = json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":anchor,"runtime_workspace_roots":[anchor],"originator":"codex_cli_rs","cli_version":"0.0.0","source":origin,"model_provider":"openai"}});
        let msg = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Historical bytes unchanged"}]}});
        fs::write(&path, format!("{meta}\n{msg}\n")).unwrap();
        source
            .import_thread(
                &path,
                id,
                &anchor,
                std::slice::from_ref(&anchor),
                project["id"].as_str().unwrap(),
                title,
            )
            .unwrap();
    }
    source
        .call("thread/archive", json!({"threadId":child}))
        .unwrap();
    source.release_server();
    let threads = all_threads(&source.snapshot().unwrap());
    let child_thread = threads.iter().find(|t| t["id"] == child).unwrap();
    assert_eq!(child_thread["parentThreadId"], parent);
    assert_eq!(child_thread["internal"], true);
    let p = PathBuf::from(child_thread["path"].as_str().unwrap());
    let raw = fs::read(&p).unwrap();
    fs::write(
        p.with_extension("jsonl.zst"),
        zstd::stream::encode_all(raw.as_slice(), 1).unwrap(),
    )
    .unwrap();
    fs::remove_file(p).unwrap();
    let saved = source
        .portable_snapshot(project["id"].as_str().unwrap(), &anchor, &Job::new())
        .unwrap();
    let ns = saved["namespace"].as_str().unwrap();
    let moved = base.join("이동 폴더");
    fs::rename(&anchor, &moved).unwrap();
    let before = digest(&moved.join("normal.txt"), &Job::new()).unwrap();
    let plan = target
        .portable_restore_plan(&moved, ns, None, &Job::new())
        .unwrap();
    assert_eq!(plan["conflicts"], json!([]));
    let cancelled = Job::new();
    let observer = cancelled.clone();
    let watch = std::thread::spawn(move || {
        for _ in 0..10000 {
            if observer.snapshot()["phase"] == "Portable conversations restore" {
                observer.cancel();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });
    let interrupted = target.portable_restore(&moved, ns, None, None, &cancelled);
    watch.join().unwrap();
    assert!(interrupted.is_err(), "Cancellation must interrupt restore");
    let pending = target
        .journal
        .history()
        .unwrap()
        .into_iter()
        .find(|o| o["payload"]["kind"] == "rust-portable-restore" && o["state"] == "needs_recovery")
        .unwrap();
    let result = target
        .portable_restore(&moved, ns, None, pending["id"].as_str(), &Job::new())
        .unwrap();
    target.release_server();
    assert_eq!(
        digest(&moved.join("normal.txt"), &Job::new()).unwrap(),
        before
    );
    let restored = all_threads(&target.snapshot().unwrap());
    assert_eq!(restored.len(), 2);
    for old in &threads {
        let t = restored.iter().find(|t| t["id"] == old["id"]).unwrap();
        assert_eq!(t["name"], old["name"]);
        assert_eq!(t["archived"], old["archived"]);
        assert_eq!(t["parentThreadId"], old["parentThreadId"]);
        assert_eq!(t["internal"], old["internal"]);
        assert_eq!(
            clean_path(std::path::Path::new(t["cwd"].as_str().unwrap())),
            moved
        );
        assert_eq!(
            runtime_roots(std::path::Path::new(t["path"].as_str().unwrap())).unwrap(),
            vec![moved.clone()]
        );
        assert_eq!(
            messages_digest(
                std::path::Path::new(t["path"].as_str().unwrap()),
                &Job::new()
            )
            .unwrap(),
            messages_digest(
                std::path::Path::new(old["path"].as_str().unwrap()),
                &Job::new()
            )
            .unwrap()
        );
    }
    let pid = result["projectId"].as_str().unwrap();
    let repeat = target
        .portable_restore_plan(&moved, ns, Some(pid), &Job::new())
        .unwrap();
    assert_eq!(repeat["skipped"].as_array().unwrap().len(), 2);
    target
        .portable_restore(&moved, ns, Some(pid), None, &Job::new())
        .unwrap();
    target.release_server();
    let changed = restored.iter().find(|t| t["id"] == parent).unwrap();
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(changed["path"].as_str().unwrap())
        .unwrap();
    writeln!(
        f,
        "{}",
        json!({"type":"event_msg","payload":{"type":"tamper","text":"divergence"}})
    )
    .unwrap();
    drop(f);
    let conflict = target
        .portable_restore_plan(&moved, ns, Some(pid), &Job::new())
        .unwrap();
    assert_eq!(conflict["conflicts"].as_array().unwrap().len(), 1);
    assert!(target
        .portable_restore(&moved, ns, Some(pid), None, &Job::new())
        .is_err());
}
#[test]
fn missing_internal_parent_and_event_attachments_are_not_portable_success() {
    let (anchor, source, mut c) = fixture();
    c.projects[0].threads[0]["parentThreadId"] = json!("00000000-0000-4000-8000-000000000066");
    assert!(portable_snapshot(&c, "project-one", &anchor, &Job::new()).is_err());
    c.projects[0].threads[0]["parentThreadId"] = Value::Null;
    use std::io::Write;
    let mut f = fs::OpenOptions::new().append(true).open(source).unwrap();
    writeln!(f,"{}",json!({"type":"event_msg","payload":{"type":"user_message","message":"image","local_images":["C:/external/image.png"]}})).unwrap();
    drop(f);
    assert!(portable_snapshot(&c, "project-one", &anchor, &Job::new()).is_err());
}
#[test]
fn symlink_sidecar_and_anchor_dependencies_are_rejected_and_discovery_is_bounded() {
    let (anchor, _, c) = fixture();
    let external = anchor.parent().unwrap().join("outside");
    fs::create_dir(&external).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;
        if symlink_dir(&external, anchor.join("linked")).is_ok() {
            assert!(portable_snapshot(&c, "project-one", &anchor, &Job::new()).is_err());
        }
    }
    let result = portable_discover(anchor.parent().unwrap(), 10, 1, &Job::new()).unwrap();
    assert_eq!(result["truncated"], true);
}

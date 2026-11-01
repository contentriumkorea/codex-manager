use codex_manager_core::{
    collect_pages, merge_legacy_assignments, merge_legacy_projects, organize_catalog,
    read_response, thread_list_params, valid_bundle_path, CodexProcess, ProjectFilter, RpcClient,
    TimedReader,
};
use serde_json::json;
use std::io::Cursor;

#[test]
fn ignores_notifications_and_returns_matching_response() {
    let mut input = Cursor::new(
        b"{\"method\":\"notice\",\"params\":{}}\n{\"id\":7,\"result\":{\"data\":[1]}}\n",
    );
    assert_eq!(read_response(&mut input, 7).unwrap(), json!({"data":[1]}));
}

#[test]
fn reports_server_error_instead_of_empty_list() {
    let mut input = Cursor::new(b"{\"id\":7,\"error\":{\"code\":-1,\"message\":\"denied\"}}\n");
    assert!(read_response(&mut input, 7).unwrap_err().contains("denied"));
}

#[test]
fn stalled_app_server_times_out_without_waiting_for_stdout() {
    struct SlowReader;
    impl std::io::Read for SlowReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(std::time::Duration::from_millis(250));
            Ok(0)
        }
    }
    let mut reader = TimedReader::new(std::io::BufReader::new(SlowReader));
    let started = std::time::Instant::now();
    let error = reader
        .response(5, std::time::Duration::from_millis(20))
        .unwrap_err();
    assert!(error.contains("timeout"));
    assert!(started.elapsed() < std::time::Duration::from_millis(180));
}

#[test]
fn timed_reader_handles_notifications_and_corresponding_response() {
    let input = Cursor::new(b"{\"method\":\"notice\"}\n{\"id\":42,\"result\":{\"data\":[]}}\n");
    let mut reader = TimedReader::new(input);
    assert_eq!(
        reader
            .response(42, std::time::Duration::from_secs(1))
            .unwrap(),
        json!({"data":[]})
    );
}

#[test]
fn preserves_multiple_roots_and_workspace_metadata_across_pages() {
    let mut calls = 0;
    let result = collect_pages(|cursor| {
        calls += 1;
        match cursor {
            None => Ok(json!({"data":[{"id":"p","roots":[{"path":"C:/a"},{"path":"D:/b"}]}],"nextCursor":"next"})),
            Some("next") => Ok(json!({"data":[{"id":"t","runtimeWorkspaceRoots":["C:/a","D:/b"],"futureField":true}],"nextCursor":null})),
            _ => panic!("unexpected cursor"),
        }
    }).unwrap();
    assert_eq!(calls, 2);
    assert_eq!(result[0]["roots"].as_array().unwrap().len(), 2);
    assert_eq!(
        result[1]["runtimeWorkspaceRoots"].as_array().unwrap().len(),
        2
    );
    assert_eq!(result[1]["futureField"], true);
}

#[test]
fn repeated_cursor_is_rejected() {
    let result = collect_pages(|_| Ok(json!({"data":[],"nextCursor":"loop"})));
    assert!(result.unwrap_err().contains("cursor"));
}

#[test]
fn malformed_page_is_rejected() {
    assert!(collect_pages(|_| Ok(json!({"data":"not an array"}))).is_err());
}

#[test]
fn nonexistent_codex_process_returns_error_instead_of_panicking() {
    let home = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(CodexProcess::start(&home.join("nonexistent-codex-executable"), home, None).is_err());
}

#[test]
fn explicit_membership_not_folder_similarity_controls_project_grouping() {
    let projects = vec![
        json!({"id":"a","name":"A","roots":[{"path":"C:/shared"},{"path":"D:/other"}]}),
        json!({"id":"b","name":"B","roots":[{"path":"C:/shared"}]}),
    ];
    let threads = vec![
        json!({"id":"t1","projectId":"b","cwd":"C:/shared","environments":null}),
        json!({"id":"t2","projectId":null,"cwd":"C:/shared","environments":[]}),
        json!({"id":"t3","projectId":"missing","cwd":"C:/shared"}),
    ];
    let catalog = organize_catalog(projects, threads).unwrap();
    assert!(catalog.projects[0].threads.is_empty());
    assert_eq!(
        catalog.projects[0].project["roots"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(catalog.projects[1].threads[0]["id"], "t1");
    assert_eq!(catalog.unassigned[0]["id"], "t2");
    assert_eq!(catalog.unresolved[0]["id"], "t3");
}

#[test]
fn unknown_or_duplicate_membership_is_not_silently_marked_unassigned() {
    assert!(organize_catalog(vec![], vec![json!({"id":"t","cwd":"C:/a"})]).is_err());
    assert!(organize_catalog(vec![json!({"id":"p"}), json!({"id":"p"})], vec![]).is_err());
    assert!(organize_catalog(
        vec![],
        vec![
            json!({"id":"t","projectId":null}),
            json!({"id":"t","projectId":null})
        ]
    )
    .is_err());
}

#[test]
fn legacy_assignment_uses_exact_host_and_preserves_native_membership() {
    let projects = vec![json!({"id":"p1"}), json!({"id":"p2"})];
    let mut threads = vec![
        json!({"id":"legacy","projectId":null}),
        json!({"id":"native","projectId":"p2"}),
    ];
    let state = json!({
        "thread-project-assignments":{"legacy":{"projectKind":"local","projectId":"old"},"native":{"projectKind":"local","projectId":"old"}},
        "app-server-project-id-by-legacy-project-id-by-host":{"local:D:/here":{"old":"p1"},"local:D:/elsewhere":{"old":"p2"}}
    });
    merge_legacy_assignments(&projects, &mut threads, &state, "local:D:/here").unwrap();
    assert_eq!(threads[0]["projectId"], "p1");
    assert_eq!(threads[1]["projectId"], "p2");
}

#[test]
fn ambiguous_and_nonlocal_legacy_assignments_stay_unresolved() {
    let projects = vec![json!({"id":"p1"}), json!({"id":"p2"})];
    let mut threads = vec![
        json!({"id":"ambiguous","projectId":null}),
        json!({"id":"remote","projectId":null}),
    ];
    let state = json!({
        "thread-project-assignments":{"ambiguous":{"projectKind":"local","projectId":"old"},"remote":{"projectKind":"remote","projectId":"p1"}},
        "app-server-project-id-by-legacy-project-id-by-host":{"local:D:/one":{"old":"p1"},"local:D:/two":{"old":"p2"}}
    });
    merge_legacy_assignments(&projects, &mut threads, &state, "local:D:/new").unwrap();
    let catalog = organize_catalog(projects, threads).unwrap();
    assert!(catalog.unassigned.is_empty());
    assert_eq!(catalog.unresolved.len(), 2);
    assert!(catalog.projects.iter().all(|p| p.threads.is_empty()));
}

#[test]
fn legacy_only_projects_remain_visible_and_can_receive_explicit_assignment() {
    let mut projects = vec![];
    let mut threads = vec![json!({"id":"t","projectId":null})];
    let state = json!({"local-projects":{"old":{"id":"old","name":"Legacy","rootPaths":["D:/a","D:/b"]}},
                     "thread-project-assignments":{"t":{"projectKind":"local","projectId":"old"}}});
    merge_legacy_projects(&mut projects, &state, "local:D:/here").unwrap();
    merge_legacy_assignments(&projects, &mut threads, &state, "local:D:/here").unwrap();
    let catalog = organize_catalog(projects, threads).unwrap();
    assert_eq!(catalog.projects.len(), 1);
    assert_eq!(catalog.projects[0].project["name"], "Legacy");
    assert_eq!(
        catalog.projects[0].project["roots"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(catalog.projects[0].threads[0]["id"], "t");
}

#[test]
fn legacy_alias_to_native_project_does_not_create_duplicate() {
    let mut projects = vec![json!({"id":"native","name":"Native","roots":[]})];
    let state = json!({"local-projects":{"old":{"id":"old","name":"Legacy","rootPaths":["D:/a"]}},
      "app-server-project-id-by-legacy-project-id-by-host":{"local:D:/here":{"old":"native"}}});
    merge_legacy_projects(&mut projects, &state, "local:D:/here").unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["name"], "Native");
}

#[test]
fn backup_relative_paths_reject_windows_escapes_and_devices() {
    for path in [
        "../secret",
        "files/../x",
        "/absolute",
        "C:/escape",
        "files/evil\\name",
        "files/NUL.txt",
        "files/name.",
        "files//x",
        "files/./x",
        "files/a:b",
    ] {
        assert!(!valid_bundle_path(path), "unsafe path accepted: {path}");
    }
    assert!(valid_bundle_path("files/root-01/한글 문서.txt"));
    assert!(valid_bundle_path(
        "conversations/00000000-0000-4000-8000-000000000001.jsonl"
    ));
}

#[test]
fn thread_filter_distinguishes_all_from_unassigned_and_specific_project() {
    let all = thread_list_params(ProjectFilter::All, false);
    assert!(all.get("projectId").is_none());
    assert_eq!(all["useStateDbOnly"], true);
    let unassigned = thread_list_params(ProjectFilter::Unassigned, false);
    assert!(unassigned.get("projectId").unwrap().is_null());
    let project = thread_list_params(ProjectFilter::Project("p-42"), true);
    assert_eq!(project["projectId"], "p-42");
    assert_eq!(project["archived"], true);
}

#[test]
fn initializes_then_reads_projects_and_threads_with_correct_filters() {
    let input = Cursor::new(concat!(
        "{\"id\":1,\"result\":{}}\n",
        "{\"method\":\"notice\",\"params\":{}}\n",
        "{\"id\":2,\"result\":{\"data\":[{\"id\":\"p\"}],\"nextCursor\":null}}\n",
        "{\"id\":3,\"result\":{\"data\":[{\"id\":\"t\",\"projectId\":\"p\"}],\"nextCursor\":null}}\n"
    ));
    let mut output = Vec::new();
    let mut client = RpcClient::new(input, &mut output);
    client.initialize().unwrap();
    let projects = client.list_projects().unwrap();
    let threads = client.list_threads(ProjectFilter::All, false).unwrap();
    assert_eq!(projects[0]["id"], "p");
    assert_eq!(threads[0]["projectId"], "p");
    drop(client);
    let messages: Vec<serde_json::Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages[0]["method"], "initialize");
    assert_eq!(
        messages[0]["params"]["capabilities"]["experimentalApi"],
        true
    );
    assert_eq!(messages[1]["method"], "initialized");
    assert!(messages[1].get("id").is_none());
    assert_eq!(messages[2]["method"], "project/list");
    assert_eq!(messages[3]["method"], "thread/list");
    assert!(messages[3]["params"].get("projectId").is_none());
    assert_eq!(messages[3]["params"]["useStateDbOnly"], true);
}

use codex_manager_core::{read_response, CodexProcess, ProjectFilter, RpcClient};
use serde_json::json;
use std::io::{BufReader, Write};
use std::process::{Child, Command, Stdio};

struct OwnedServer(Child);
impl Drop for OwnedServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires installed Codex executable and a fresh isolated home"]
fn installed_app_server_handshake_and_lists_work_in_isolated_home() {
    let binary =
        std::env::var_os("CODEX_TEST_BINARY").expect("installed Codex executable required");
    let home = std::path::PathBuf::from(
        std::env::var_os("CODEX_TEST_HOME").expect("isolated home required"),
    )
    .join("empty");
    assert!(!home.exists(), "use a fresh isolated home");
    std::fs::create_dir_all(&home).unwrap();
    let db = home.join("sqlite");
    std::fs::create_dir_all(&db).unwrap();
    let config = format!("sqlite_home={}", serde_json::to_string(&db).unwrap());
    let mut process = OwnedServer(
        Command::new(binary)
            .args(["-c", &config, "app-server", "--listen", "stdio://"])
            .env("CODEX_HOME", &home)
            .env("CODEX_SQLITE_HOME", &db)
            .env("RUST_LOG", "error")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let reader = BufReader::new(process.0.stdout.take().unwrap());
    let writer = process.0.stdin.take().unwrap();
    let mut client = RpcClient::new(reader, writer);
    client.initialize().unwrap();
    assert!(client.list_projects().unwrap().is_empty());
    assert!(client
        .list_threads(ProjectFilter::All, false)
        .unwrap()
        .is_empty());
}

#[test]
#[ignore = "requires installed Codex executable and a fresh isolated home"]
fn managed_process_reads_an_isolated_home_and_exits() {
    let binary = std::path::PathBuf::from(std::env::var_os("CODEX_TEST_BINARY").unwrap());
    let home =
        std::path::PathBuf::from(std::env::var_os("CODEX_TEST_HOME").unwrap()).join("managed");
    assert!(!home.exists());
    let db = home.join("sqlite");
    std::fs::create_dir_all(&db).unwrap();
    let mut process = CodexProcess::start(&binary, &home, Some(&db)).unwrap();
    assert!(process.projects().unwrap().is_empty());
    assert!(process
        .threads(ProjectFilter::All, false)
        .unwrap()
        .is_empty());
}

#[test]
#[ignore = "requires installed Codex executable and a fresh isolated home"]
fn bundled_server_preserves_a_projects_multiple_roots() {
    let binary = std::env::var_os("CODEX_TEST_BINARY").expect("Codex executable required");
    let home = std::path::PathBuf::from(
        std::env::var_os("CODEX_TEST_HOME").expect("isolated home required"),
    )
    .join("multi-root");
    assert!(!home.exists(), "use a fresh isolated home");
    let root_a = home.join("workspace-a");
    let root_b = home.join("workspace-b");
    let db = home.join("sqlite");
    for path in [&root_a, &root_b, &db] {
        std::fs::create_dir_all(path).unwrap();
    }
    let config = format!("sqlite_home={}", serde_json::to_string(&db).unwrap());
    let start = || {
        Command::new(&binary)
            .args(["-c", &config, "app-server", "--listen", "stdio://"])
            .env("CODEX_HOME", &home)
            .env("CODEX_SQLITE_HOME", &db)
            .env("RUST_LOG", "error")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    let thread_id = "00000000-0000-4000-8000-000000000012";
    {
        let mut process = OwnedServer(start());
        let mut reader = BufReader::new(process.0.stdout.take().unwrap());
        let mut writer = process.0.stdin.take().unwrap();
        let initialize = json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"fixture","version":"1"},"capabilities":{"experimentalApi":true}}});
        writeln!(writer, "{initialize}").unwrap();
        writer.flush().unwrap();
        read_response(&mut reader, 1).unwrap();
        writeln!(writer, "{}", json!({"method":"initialized"})).unwrap();
        let create = json!({"id":2,"method":"project/create","params":{"idempotencyKey":"00000000-0000-4000-8000-000000000001","name":"Fixture","roots":[{"path":root_a},{"path":root_b}]}});
        writeln!(writer, "{create}").unwrap();
        writer.flush().unwrap();
        let project = read_response(&mut reader, 2).unwrap()["project"].clone();
        assert_eq!(project["name"], "Fixture");
        let sessions = home.join("sessions/2026/11/02");
        std::fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join(format!("rollout-2026-11-02T00-00-00-{thread_id}.jsonl"));
        let meta = json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":thread_id,"timestamp":"2026-11-02T00:00:00Z","cwd":root_a,"originator":"codex_cli_rs","cli_version":"0.0.0","source":"cli","model_provider":"openai"}});
        let event = json!({"timestamp":"2026-11-02T00:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"isolated fixture","images":[],"local_images":[],"text_elements":[]}});
        let msg = json!({"timestamp":"2026-11-02T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"isolated fixture"}]}});
        std::fs::write(&rollout, format!("{meta}\n{event}\n{msg}\n")).unwrap();
        let resume = json!({"id":3,"method":"thread/resume","params":{"threadId":thread_id,"path":rollout,"cwd":root_a,"runtimeWorkspaceRoots":[root_a,root_b]}});
        writeln!(writer, "{resume}").unwrap();
        writer.flush().unwrap();
        assert_eq!(
            read_response(&mut reader, 3).unwrap()["thread"]["id"],
            thread_id
        );
        let assign = json!({"id":4,"method":"thread/metadata/update","params":{"threadId":thread_id,"projectId":project["id"]}});
        writeln!(writer, "{assign}").unwrap();
        writer.flush().unwrap();
        read_response(&mut reader, 4).unwrap();
    }
    let mut process = OwnedServer(start());
    let reader = BufReader::new(process.0.stdout.take().unwrap());
    let writer = process.0.stdin.take().unwrap();
    let mut client = RpcClient::new(reader, writer);
    client.initialize().unwrap();
    let projects = client.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["name"], "Fixture");
    let roots = projects[0]["roots"].as_array().unwrap();
    assert_eq!(roots.len(), 2);
    assert_eq!(
        std::path::Path::new(roots[0]["path"].as_str().unwrap()),
        root_a
    );
    assert_eq!(
        std::path::Path::new(roots[1]["path"].as_str().unwrap()),
        root_b
    );
    let threads = client.list_threads(ProjectFilter::All, false).unwrap();
    let thread = threads
        .iter()
        .find(|v| v["id"] == thread_id)
        .expect("fixture thread missing");
    assert_eq!(thread["projectId"], projects[0]["id"]);
    let grouped = codex_manager_core::organize_catalog(projects, threads).unwrap();
    assert_eq!(grouped.projects[0].threads[0]["id"], thread_id);
}

use codex_manager_core::{absolute, backup_project, Adapter, Job, Journal, Store};
use serde_json::json;
use std::{fs, path::PathBuf};
fn main() -> Result<(), String> {
    let root =
        PathBuf::from(std::env::var_os("CODEX_TEST_HOME").ok_or("Set a fresh CODEX_TEST_HOME")?)
            .join("ui-fixture");
    absolute(&root)?;
    if root.exists() {
        return Err("Use a fresh fixture directory".into());
    }
    let home = root.join("store");
    fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let binary = std::env::var_os("CODEX_TEST_BINARY")
        .ok_or("Set CODEX_TEST_BINARY")?
        .into();
    let mut adapter = Adapter::isolated(
        Store {
            home: home.clone(),
            sqlite_home: home.clone(),
            database: None,
        },
        binary,
        Journal::open(&root.join("seed-state/journal.sqlite"))?,
    )?;
    for (i, (name, title)) in [
        ("브랜드 리뉴얼", "브랜드 방향과 디자인 정리"),
        ("홈페이지 제작", "메뉴 구성과 첫 화면"),
        ("콘텐츠 자료", "다음 주 콘텐츠 기획"),
    ]
    .iter()
    .enumerate()
    {
        let workspace = root.join("projects").join(format!("project-{:02}", i + 1));
        fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
        fs::write(
            workspace.join("작업 메모.txt"),
            "화면 검증용 예시 자료입니다.\n",
        )
        .map_err(|e| e.to_string())?;
        let p = adapter.create(name, std::slice::from_ref(&workspace))?;
        let id = uuid::Uuid::new_v4().to_string();
        let source = root.join(format!("thread-{i}.jsonl"));
        let meta = json!({"timestamp":"2026-11-02T00:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-02T00:00:00Z","cwd":workspace,"originator":"codex_cli_rs","cli_version":"0.0.0","source":"cli","model_provider":"openai"}});
        let user = json!({"timestamp":"2026-11-02T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"작업 파일과 대화를 함께 정리해 주세요."}]}});
        let assistant = json!({"timestamp":"2026-11-02T00:00:02Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"예시 프로젝트의 자료와 대화를 준비했습니다."}]}});
        fs::write(&source, format!("{meta}\n{user}\n{assistant}\n")).map_err(|e| e.to_string())?;
        adapter.import_thread(
            &source,
            &id,
            &workspace,
            std::slice::from_ref(&workspace),
            p["id"].as_str().ok_or("Project ID missing")?,
            title,
        )?;
        if i == 0 {
            adapter.release_server();
            backup_project(
                &adapter.snapshot()?,
                p["id"].as_str().unwrap(),
                &root.join("backups/brand"),
                &Job::new(),
            )?;
        }
    }
    println!("UI fixture created with 3 example projects");
    Ok(())
}

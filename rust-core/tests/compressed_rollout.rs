use codex_manager_core::*;
use serde_json::json;
use std::{fs, path::PathBuf};
const ID: &str = "00000000-0000-4000-8000-000000000099";
fn fixture() -> (PathBuf, PathBuf, String) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&root).unwrap();
    let path = root.join("기록.jsonl");
    let meta = json!({"type":"session_meta","payload":{"id":ID,"cwd":root,"runtime_workspace_roots":[root]}});
    let msg = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"보존할 메시지"}]}});
    let settings = json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"runtime_workspace_roots":[]}}});
    (root, path, format!("{meta}\n{msg}\n{settings}\n"))
}
fn compressed(path: &std::path::Path, bytes: &[u8]) -> PathBuf {
    let zst = path.with_extension("jsonl.zst");
    fs::write(&zst, zstd::stream::encode_all(bytes, 1).unwrap()).unwrap();
    zst
}
#[test]
fn stale_database_path_reads_compressed_settings_and_preserves_messages_on_import() {
    let (root, path, text) = fixture();
    let zst = compressed(&path, text.as_bytes());
    assert_eq!(runtime_roots(&path).unwrap(), Vec::<PathBuf>::new());
    let expected = digest(&zst, &Job::new()).unwrap();
    let out = prepare_rollout_with_job(
        &path,
        &root.join("home"),
        ID,
        &root,
        &[],
        Some(&expected),
        &Job::new(),
    )
    .unwrap();
    let actual = fs::read_to_string(out).unwrap();
    assert_eq!(actual.lines().nth(1), text.lines().nth(1));
    assert_eq!(
        fs::read(&zst).unwrap(),
        zstd::stream::encode_all(text.as_bytes(), 1).unwrap()
    );
}
#[test]
fn concatenated_frames_decode_and_truncated_or_corrupt_frames_fail() {
    let (root, path, text) = fixture();
    let split = text.find('\n').unwrap() + 1;
    let mut bytes = zstd::stream::encode_all(&text.as_bytes()[..split], 1).unwrap();
    bytes.extend(zstd::stream::encode_all(&text.as_bytes()[split..], 1).unwrap());
    let zst = path.with_extension("jsonl.zst");
    fs::write(&zst, &bytes).unwrap();
    assert!(prepare_rollout(&path, &root.join("home"), ID, &root, &[]).is_ok());
    for bad in [
        bytes[..bytes.len() - 1].to_vec(),
        [bytes.clone(), b"broken".to_vec()].concat(),
        b"not-zstandard".to_vec(),
    ] {
        fs::write(&zst, bad).unwrap();
        assert!(runtime_roots(&path).is_err());
        assert!(prepare_rollout(&path, &root.join("home"), ID, &root, &[]).is_err());
    }
}
#[test]
fn every_record_is_validated_even_before_latest_runtime_settings() {
    let (_, path, text) = fixture();
    let invalid = text.replacen("\n", "\nnot-json\n", 1);
    fs::write(&path, &invalid).unwrap();
    assert!(runtime_roots(&path).is_err());
    fs::remove_file(&path).unwrap();
    compressed(&path, invalid.as_bytes());
    assert!(runtime_roots(&path).is_err());
}
#[test]
fn compressed_backup_preserves_raw_bytes_and_database_id() {
    let (root, path, text) = fixture();
    let zst = compressed(&path, text.as_bytes());
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let db_path = root.join("state.sqlite");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    db.execute_batch("CREATE TABLE projects(id TEXT,name TEXT);CREATE TABLE project_roots(project_id TEXT,path TEXT);CREATE TABLE threads(id TEXT,project_id TEXT,cwd TEXT,rollout_path TEXT,name TEXT,archived INTEGER); INSERT INTO projects VALUES('p','Project');").unwrap();
    db.execute(
        "INSERT INTO project_roots VALUES('p',?1)",
        [workspace.to_str().unwrap()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES(?1,'p',?2,?3,'Title',1)",
        rusqlite::params![ID, workspace.to_str().unwrap(), path.to_str().unwrap()],
    )
    .unwrap();
    drop(db);
    let catalog = read_database_catalog(&db_path).unwrap();
    assert_eq!(all_threads(&catalog)[0]["path"], json!(zst));
    let out = root.join("backup");
    backup_project(&catalog, "p", &out, &Job::new()).unwrap();
    let bundle = load_verified_bundle(&out, &Job::new()).unwrap();
    assert_eq!(bundle["conversations"][0]["id"], ID);
    assert_eq!(bundle["conversations"][0]["archived"], true);
    let saved = out.join(bundle["conversations"][0]["rollout"].as_str().unwrap());
    assert_eq!(fs::read(&saved).unwrap(), fs::read(zst).unwrap());
    // Even self-consistent raw checksums cannot certify a truncated zstd stream.
    let bytes = fs::read(&saved).unwrap();
    fs::write(&saved, &bytes[..bytes.len() - 1]).unwrap();
    let checks = out.join("checksums.jsonl");
    let rewritten = fs::read_to_string(&checks)
        .unwrap()
        .lines()
        .map(|line| {
            let mut v: serde_json::Value = serde_json::from_str(line).unwrap();
            if v["path"] == bundle["conversations"][0]["rollout"] {
                v["size"] = json!(bytes.len() - 1);
                v["sha256"] = json!(digest(&saved, &Job::new()).unwrap());
            }
            format!("{v}\n")
        })
        .collect::<String>();
    fs::write(checks, rewritten).unwrap();
    assert!(load_verified_bundle(&out, &Job::new()).is_err());
}

#[test]
#[cfg(windows)]
fn active_writer_does_not_block_readonly_runtime_scan() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_, path, text) = fixture();
    fs::write(&path, text).unwrap();
    let _writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(7)
        .open(&path)
        .unwrap();
    assert!(runtime_roots(&path).unwrap().is_empty());
}

#[test]
fn reader_rejects_wrong_id_cancellation_and_live_path_substitution() {
    let (_, path, text) = fixture();
    let zst = compressed(&path, text.as_bytes());
    let job = Job::new();
    let mut wrong = RolloutReader::open(&path, Some("other-id"), &job).unwrap();
    assert!(wrong.next_record().is_err());
    drop(wrong);
    let mut reader = RolloutReader::open(&path, Some(ID), &job).unwrap();
    assert!(reader.next_record().unwrap().is_some());
    job.cancel();
    assert!(reader.next_record().is_err());
    drop(reader);
    let job = Job::new();
    let mut live = RolloutReader::open_live(&path, Some(ID), &job).unwrap();
    assert!(live.next_record().unwrap().is_some());
    // A new plain file must never silently substitute the sibling selected at open.
    fs::write(&path, &text).unwrap();
    let mut rejected = false;
    loop {
        match live.next_record() {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {
                rejected = true;
                break;
            }
        }
    }
    assert!(rejected);
    assert!(zst.exists());
}
#[test]
fn live_reader_detects_in_place_mutation_and_raw_vs_decoded_proofs_differ() {
    let (_, path, text) = fixture();
    fs::write(&path, &text).unwrap();
    let zst = compressed(&path, text.as_bytes());
    assert_ne!(
        digest(&path, &Job::new()).unwrap(),
        digest(&zst, &Job::new()).unwrap()
    );
    assert_eq!(
        decoded_rollout_digest(&path, &Job::new()).unwrap(),
        decoded_rollout_digest(&zst, &Job::new()).unwrap()
    );
    assert_eq!(
        messages_digest(&path, &Job::new()).unwrap(),
        messages_digest(&zst, &Job::new()).unwrap()
    );
    let mut reader = RolloutReader::open_live(&path, Some(ID), &Job::new()).unwrap();
    assert!(reader.next_record().unwrap().is_some());
    fs::write(&path, format!("{text}{{}}\n")).unwrap();
    let mut rejected = false;
    loop {
        match reader.next_record() {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {
                rejected = true;
                break;
            }
        }
    }
    assert!(rejected);
}

#[test]
fn a_failed_record_cannot_be_skipped_to_reach_verified_eof() {
    let (_, path, text) = fixture();
    fs::write(&path, text.replacen("\n", "\nnot-json\n", 1)).unwrap();
    let mut reader = RolloutReader::open(&path, Some(ID), &Job::new()).unwrap();
    assert!(reader.next_record().unwrap().is_some());
    assert!(reader.next_record().is_err());
    assert!(reader.next_record().is_err());
}

#[test]
fn cancellation_during_one_large_decoded_record_is_observed() {
    let (_, path, text) = fixture();
    let meta = text.lines().next().unwrap();
    let large = format!(
        "{meta}\n{{\"type\":\"response_item\",\"payload\":\"{}\"}}\n",
        "x".repeat(32 * 1024 * 1024)
    );
    compressed(&path, large.as_bytes());
    let job = Job::new();
    let mut reader = RolloutReader::open(&path, Some(ID), &job).unwrap();
    assert!(reader.next_record().unwrap().is_some());
    let cancel = job.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(2));
        cancel.cancel();
    });
    let result = reader.next_record();
    worker.join().unwrap();
    assert!(result.is_err());
}

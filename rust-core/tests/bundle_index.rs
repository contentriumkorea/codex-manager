use codex_manager_core::{validate_bundle_index, verify_bundle};
use serde_json::{json, Value};

fn fixture() -> (Value, Vec<Value>) {
    let manifest = json!({
        "format_version":1,"state":"complete",
        "bundle_id":"00000000-0000-4000-8000-000000000001",
        "project":{"id":"p","name":"Project","roots":["C:/original"]},
        "roots":[{"id":"root-01","original_path":"C:/original","bundle_path":"files/root-01"}],
        "conversations":[{"id":"00000000-0000-4000-8000-000000000002",
                          "rollout":"conversations/chat.jsonl","cwd":"C:/original",
                          "title":"chat","runtime_roots":[]}],
        "inventory":[{"root_id":"root-01","relative_path":"한글.txt","size":5,"sha256":"a".repeat(64)}],
        "directories":[],"dependencies":[]
    });
    let checksums = vec![
        json!({"path":"manifest.json","size":40,"sha256":"b".repeat(64)}),
        json!({"path":"conversations/chat.jsonl","size":10,"sha256":"c".repeat(64)}),
        json!({"path":"files/root-01/한글.txt","size":5,"sha256":"a".repeat(64)}),
    ];
    (manifest, checksums)
}

#[test]
fn accepts_complete_consistent_v1_backup_index() {
    let (manifest, checksums) = fixture();
    assert!(validate_bundle_index(&manifest, &checksums).is_ok());
}

#[test]
fn rejects_incomplete_or_changed_file_index() {
    let (mut manifest, checksums) = fixture();
    manifest["state"] = json!("partial");
    assert!(validate_bundle_index(&manifest, &checksums).is_err());
    manifest["state"] = json!("complete");
    manifest["inventory"][0]["size"] = json!(6);
    assert!(validate_bundle_index(&manifest, &checksums).is_err());
}

#[test]
fn rejects_escape_duplicate_checksum_and_undeclared_files() {
    let (mut manifest, mut checksums) = fixture();
    manifest["conversations"][0]["rollout"] = json!("../escape");
    assert!(validate_bundle_index(&manifest, &checksums).is_err());
    let (manifest, original) = fixture();
    checksums.push(original[1].clone());
    assert!(validate_bundle_index(&manifest, &checksums).is_err());
    checksums = original;
    checksums.push(json!({"path":"extra.txt","size":0,"sha256":"d".repeat(64)}));
    assert!(validate_bundle_index(&manifest, &checksums).is_err());
}

#[test]
#[ignore = "requires a freshly generated Python 0.10.4 fixture"]
fn python_exported_bundle_index_is_accepted() {
    let root = std::path::PathBuf::from(
        std::env::var_os("CODEX_BACKUP_FIXTURE").expect("fixture path required"),
    );
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let lines = std::fs::read_to_string(root.join("checksums.jsonl")).unwrap();
    let checksums: Vec<Value> = lines
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    validate_bundle_index(&manifest, &checksums).unwrap();
    verify_bundle(&root).unwrap();
}

#[test]
fn bundle_verification_detects_tampering_and_unlisted_files() {
    use sha2::{Digest, Sha256};
    use std::time::{SystemTime, UNIX_EPOCH};
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(format!("bundle-{nonce}"));
    std::fs::create_dir_all(root.join("files/root-01")).unwrap();
    std::fs::write(root.join("files/root-01/hello.txt"), b"hello").unwrap();
    let hash = format!("{:x}", Sha256::digest(b"hello"));
    let manifest = json!({
        "format_version":1,"state":"complete","bundle_id":"00000000-0000-4000-8000-000000000001",
        "project":{"id":"p","name":"Project","roots":["C:/original"]},
        "roots":[{"id":"root-01","original_path":"C:/original","bundle_path":"files/root-01"}],
        "conversations":[],"inventory":[{"root_id":"root-01","relative_path":"hello.txt","size":5,"sha256":hash}],
        "directories":[],"dependencies":[]
    });
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    std::fs::write(root.join("manifest.json"), &manifest_bytes).unwrap();
    let manifest_hash = format!("{:x}", Sha256::digest(&manifest_bytes));
    let checksums = [
        json!({"path":"manifest.json","size":manifest_bytes.len(),"sha256":manifest_hash}),
        json!({"path":"files/root-01/hello.txt","size":5,"sha256":hash}),
    ];
    let listing = checksums
        .iter()
        .map(|v| v.to_string() + "\n")
        .collect::<String>();
    std::fs::write(root.join("checksums.jsonl"), listing).unwrap();
    assert!(verify_bundle(&root).is_ok());
    std::fs::write(root.join("files/root-01/hello.txt"), b"HELLO").unwrap();
    assert!(verify_bundle(&root).is_err());
    std::fs::write(root.join("files/root-01/hello.txt"), b"hello").unwrap();
    std::fs::write(root.join("extra.txt"), b"x").unwrap();
    assert!(verify_bundle(&root).is_err());
}

#[cfg(windows)]
#[test]
fn rejects_a_bundle_root_that_is_a_windows_junction() {
    use std::os::windows::process::CommandExt;
    use std::time::{SystemTime, UNIX_EPOCH};
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let parent = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".test-artifacts")
        .join(format!("junction-{nonce}"));
    let target = parent.join("target");
    let link = parent.join("link");
    std::fs::create_dir_all(&target).unwrap();
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command",
            "New-Item -ItemType Junction -Path $env:CODEX_TEST_LINK -Target $env:CODEX_TEST_TARGET | Out-Null"])
        .env("CODEX_TEST_LINK", &link).env("CODEX_TEST_TARGET", &target)
        .creation_flags(0x08000000)
        .status().unwrap();
    assert!(status.success(), "junction fixture could not be created");
    let error = verify_bundle(&link).unwrap_err();
    assert!(error.contains("root"));
}

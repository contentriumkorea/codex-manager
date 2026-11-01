use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub fn validate_bundle_index(manifest: &Value, checksums: &[Value]) -> Result<(), String> {
    fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
        v.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing string: {key}"))
    }
    fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
        v.get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("missing array: {key}"))
    }
    fn uuid(s: &str) -> bool {
        s.len() == 36
            && s.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
    }
    if manifest.get("format_version").and_then(Value::as_u64) != Some(1)
        || manifest.get("state").and_then(Value::as_str) != Some("complete")
        || !uuid(string(manifest, "bundle_id")?)
    {
        return Err("unsupported or incomplete bundle".into());
    }
    let project = manifest.get("project").ok_or("missing project")?;
    string(project, "id")?;
    string(project, "name")?;
    array(project, "roots")?;
    array(manifest, "dependencies")?;
    let mut expected = HashMap::new();
    let mut normalized = HashSet::new();
    for entry in checksums {
        let path = string(entry, "path")?;
        let hash = string(entry, "sha256")?;
        if !valid_bundle_path(path)
            || entry.get("size").and_then(Value::as_u64).is_none()
            || hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || !normalized.insert(path.to_lowercase())
        {
            return Err("invalid or duplicate checksum entry".into());
        }
        expected.insert(path.to_owned(), entry);
    }
    if !expected.contains_key("manifest.json") {
        return Err("missing manifest hash".into());
    }
    let mut roots = HashSet::new();
    for root in array(manifest, "roots")? {
        let id = string(root, "id")?;
        string(root, "original_path")?;
        if !valid_bundle_path(id)
            || id.contains('/')
            || !roots.insert(id.to_owned())
            || string(root, "bundle_path")? != format!("files/{id}")
        {
            return Err("invalid or duplicate root".into());
        }
    }
    let mut declared = HashSet::from(["manifest.json".to_owned()]);
    let mut thread_ids = HashSet::new();
    for thread in array(manifest, "conversations")? {
        let id = string(thread, "id")?;
        let path = string(thread, "rollout")?;
        string(thread, "cwd")?;
        string(thread, "title")?;
        array(thread, "runtime_roots")?;
        if !uuid(id)
            || !thread_ids.insert(id.to_owned())
            || !valid_bundle_path(path)
            || !expected.contains_key(path)
            || !declared.insert(path.to_owned())
        {
            return Err("invalid conversation entry".into());
        }
    }
    for entry in array(manifest, "inventory")? {
        let root = string(entry, "root_id")?;
        let relative = string(entry, "relative_path")?;
        if !roots.contains(root) || !valid_bundle_path(relative) {
            return Err("invalid inventory path".into());
        }
        let path = format!("files/{root}/{relative}");
        let check = expected.get(&path).ok_or("missing inventory hash")?;
        if entry.get("sha256") != check.get("sha256")
            || entry.get("size") != check.get("size")
            || !declared.insert(path)
        {
            return Err("inventory and checksums differ".into());
        }
    }
    for directory in array(manifest, "directories")? {
        let pair = directory.as_array().ok_or("invalid directory")?;
        if pair.len() != 2
            || !pair[0].as_str().is_some_and(|id| roots.contains(id))
            || !pair[1].as_str().is_some_and(valid_bundle_path)
        {
            return Err("invalid directory".into());
        }
    }
    if declared.len() != expected.len() || !declared.iter().all(|path| expected.contains_key(path))
    {
        return Err("undeclared checksum entry".into());
    }
    Ok(())
}

pub fn valid_bundle_path(path: &str) -> bool {
    if path.is_empty()
        || path
            .chars()
            .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
    {
        return false;
    }
    path.split('/').all(|part| {
        if part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' ']) {
            return false;
        }
        let name = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(
            name.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) {
            return false;
        }
        for prefix in ["COM", "LPT"] {
            if let Some(suffix) = name.strip_prefix(prefix) {
                if matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                ) {
                    return false;
                }
            }
        }
        true
    })
}

//! JSONC locked-edit tests.

use super::*;
use tempfile::TempDir;

#[test]
fn locked_modify_jsonc_round_trip_preserves_comments() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("opencode.jsonc");
    let original =
        "{\n  // hello\n  \"a\": 1, // trailing line comment\n  \"b\": [1, 2,], /* block */\n}\n";
    std::fs::write(&path, original).unwrap();

    locked_modify_jsonc(&path, |_root| Ok(())).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn locked_modify_jsonc_appends_into_mcp_keeping_neighbor_comments() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("opencode.jsonc");
    std::fs::write(
            &path,
            "{\n  // top note\n  \"model\": \"x\",\n  \"mcp\": {\n    // about github\n    \"github\": {\"type\": \"local\", \"command\": [\"a\"]}\n  }\n}\n",
        )
        .unwrap();

    locked_modify_jsonc(&path, |root| {
        let mcp = root.object_value_or_set("mcp");
        mcp.append(
            "filesystem",
            to_cst_input(&serde_json::json!({"type": "local", "command": ["b"]})),
        );
        Ok(())
    })
    .unwrap();

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("// top note"), "top-level comment dropped");
    assert!(
        written.contains("// about github"),
        "mcp child comment dropped"
    );
    assert!(written.contains("\"github\""), "existing entry lost");
    assert!(written.contains("\"filesystem\""), "appended entry missing");
}

#[test]
fn locked_modify_jsonc_rejects_non_object_root() {
    // Refuse to silently overwrite a top-level array — better to error
    // than to destroy data. Mirrors locked_modify_json's behavior.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("weird.jsonc");
    std::fs::write(&path, "[1, 2, 3]").unwrap();

    let err = locked_modify_jsonc(&path, |_| Ok(()));
    assert!(matches!(err, Err(HkError::ConfigCorrupted(_))));
    // File untouched.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[1, 2, 3]");
}

#[test]
fn locked_modify_jsonc_seeds_empty_file_with_object() {
    // First-time write to an empty/non-existent file: seed with `{}`
    // so the helper has a valid object root to operate on.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("fresh.jsonc");

    locked_modify_jsonc(&path, |root| {
        root.append("mcp", to_cst_input(&serde_json::json!({})));
        Ok(())
    })
    .unwrap();

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("\"mcp\""));
    let _: serde_json::Value = serde_json::from_str(&written).unwrap();
}

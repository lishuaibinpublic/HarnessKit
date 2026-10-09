//! Hook deployment tests.

use super::*;
use tempfile::TempDir;

#[test]
fn test_deploy_hook_new_file() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("hooks.json");
    let entry = HookEntry {
        event: "PreToolUse".into(),
        matcher: Some("Bash".into()),
        command: "echo test".into(),
        enabled: true,
    };
    deploy_hook(&config, &entry, HookFormat::ClaudeLike).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let hook = &content["hooks"]["PreToolUse"][0];
    assert_eq!(hook["matcher"], "Bash");
    // Now writes object format: {"type":"command","command":"echo test"}
    assert_eq!(hook["hooks"][0]["type"], "command");
    assert_eq!(hook["hooks"][0]["command"], "echo test");
}

#[test]
fn test_deploy_hook_appends_to_existing_group() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("settings.json");
    // Existing hook in old string format
    std::fs::write(
        &config,
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":["echo first"]}]}}"#,
    )
    .unwrap();

    let entry = HookEntry {
        event: "PreToolUse".into(),
        matcher: Some("Bash".into()),
        command: "echo second".into(),
        enabled: true,
    };
    deploy_hook(&config, &entry, HookFormat::ClaudeLike).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let hooks = content["hooks"]["PreToolUse"][0]["hooks"]
        .as_array()
        .unwrap();
    assert_eq!(hooks.len(), 2);
    assert_eq!(hooks[0], "echo first"); // old string entry preserved
    assert_eq!(hooks[1]["command"], "echo second"); // new entry in object format
}

#[test]
fn test_deploy_hook_no_duplicate_command() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("hooks.json");
    // Existing hook in object format
    std::fs::write(&config, r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo test"}]}]}}"#).unwrap();

    let entry = HookEntry {
        event: "PreToolUse".into(),
        matcher: Some("Bash".into()),
        command: "echo test".into(),
        enabled: true,
    };
    deploy_hook(&config, &entry, HookFormat::ClaudeLike).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let hooks = content["hooks"]["PreToolUse"][0]["hooks"]
        .as_array()
        .unwrap();
    assert_eq!(hooks.len(), 1); // not duplicated
}

#[test]
fn test_restore_hook() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("settings.json");
    std::fs::write(&config, r#"{"hooks":{}}"#).unwrap();

    let entry = serde_json::json!({"matcher": "Bash", "hooks": ["echo test"]});
    restore_hook(&config, "PreToolUse", &entry, HookFormat::ClaudeLike).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(content["hooks"]["PreToolUse"][0]["matcher"], "Bash");
    assert_eq!(content["hooks"]["PreToolUse"][0]["hooks"][0], "echo test");
}

#[test]
fn test_read_hook_config() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("settings.json");
    std::fs::write(
        &config,
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":["echo test"]}]}}"#,
    )
    .unwrap();

    let entry = read_hook_config(
        &config,
        "PreToolUse",
        Some("Bash"),
        "echo test",
        HookFormat::ClaudeLike,
    )
    .unwrap();
    assert!(entry.is_some());
    assert_eq!(entry.unwrap()["matcher"], "Bash");

    let missing = read_hook_config(
        &config,
        "PreToolUse",
        Some("Bash"),
        "nonexistent",
        HookFormat::ClaudeLike,
    )
    .unwrap();
    assert!(missing.is_none());
}

#[test]
fn test_read_hook_config_windsurf_format() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("hooks.json");
    std::fs::write(
        &config,
        r#"{"hooks":{"post_cascade_response":[{"powershell":"python C:\\hooks\\log.py"}]}}"#,
    )
    .unwrap();

    let entry = read_hook_config(
        &config,
        "post_cascade_response",
        None,
        "python C:\\hooks\\log.py",
        HookFormat::Windsurf,
    )
    .unwrap();
    assert!(entry.is_some());
    assert_eq!(entry.unwrap()["powershell"], "python C:\\hooks\\log.py");
}

#[test]
fn test_deploy_hook_cursor_format() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("hooks.json");
    let entry = HookEntry {
        event: "stop".into(),
        matcher: None,
        command: "echo done".into(),
        enabled: true,
    };
    deploy_hook(&config, &entry, HookFormat::Cursor).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(content["version"], 1);
    assert_eq!(content["hooks"]["stop"][0]["command"], "echo done");
    // Should NOT have matcher or nested hooks array
    assert!(content["hooks"]["stop"][0].get("matcher").is_none());
    assert!(content["hooks"]["stop"][0].get("hooks").is_none());
}

#[test]
fn test_deploy_hook_copilot_format() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("hooks.json");
    let entry = HookEntry {
        event: "PreToolUse".into(),
        matcher: None,
        command: "./check.sh".into(),
        enabled: true,
    };
    deploy_hook(&config, &entry, HookFormat::Copilot).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(content["version"], 1);
    assert_eq!(content["hooks"]["PreToolUse"][0]["type"], "command");
    assert_eq!(content["hooks"]["PreToolUse"][0]["command"], "./check.sh");
}

#[test]
fn test_deploy_hook_windsurf_format() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("hooks.json");
    let entry = HookEntry {
        event: "pre_user_prompt".into(),
        matcher: None,
        command: "echo hi".into(),
        enabled: true,
    };
    deploy_hook(&config, &entry, HookFormat::Windsurf).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert!(content.get("version").is_none());
    assert_eq!(content["hooks"]["pre_user_prompt"][0]["command"], "echo hi");
}

#[test]
fn test_kiro_ide_hook_roundtrip() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("lint.json");
    let entry = HookEntry {
        event: "PostFileSave".into(),
        matcher: Some("\\.ts$".into()),
        command: "npm run lint".into(),
        enabled: true,
    };
    deploy_hook(&config, &entry, HookFormat::KiroIde).unwrap();
    let deployed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    // Kiro only documents "v1" (https://kiro.dev/docs/hooks/); other values
    // may make Kiro skip the file entirely.
    assert_eq!(deployed["version"], "v1");
    let saved = read_hook_config(
        &config,
        "PostFileSave",
        Some("\\.ts$"),
        "npm run lint",
        HookFormat::KiroIde,
    )
    .unwrap()
    .expect("Kiro hook should be readable");
    assert_eq!(saved["action"]["type"], "command");
    assert_eq!(saved["action"]["command"], "npm run lint");

    remove_hook(
        &config,
        "PostFileSave",
        Some("\\.ts$"),
        "npm run lint",
        HookFormat::KiroIde,
    )
    .unwrap();
    let removed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(removed["hooks"].as_array().unwrap().len(), 0);

    restore_hook(&config, "PostFileSave", &saved, HookFormat::KiroIde).unwrap();
    let restored = read_hook_config(
        &config,
        "PostFileSave",
        Some("\\.ts$"),
        "npm run lint",
        HookFormat::KiroIde,
    )
    .unwrap();
    assert!(restored.is_some());
}

#[test]
fn test_kiro_ide_restore_hook_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("lint.json");
    let entry = serde_json::json!({
        "name": "lint-on-save",
        "trigger": "PostFileSave",
        "matcher": "\\.ts$",
        "action": { "type": "command", "command": "npm run lint" },
    });
    restore_hook(&config, "PostFileSave", &entry, HookFormat::KiroIde).unwrap();
    restore_hook(&config, "PostFileSave", &entry, HookFormat::KiroIde).unwrap();
    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(
        content["hooks"].as_array().unwrap().len(),
        1,
        "double restore must not duplicate the hook"
    );
}

#[test]
fn test_set_kiro_hook_enabled_flips_in_place() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("lint.json");
    let entry = HookEntry {
        event: "PostFileSave".into(),
        matcher: Some("\\.ts$".into()),
        command: "npm run lint".into(),
        enabled: true,
    };
    deploy_hook(&config, &entry, HookFormat::KiroIde).unwrap();

    set_kiro_hook_enabled(
        &config,
        "PostFileSave",
        Some("\\.ts$"),
        "npm run lint",
        false,
    )
    .unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(doc["hooks"].as_array().unwrap().len(), 1, "entry kept");
    assert_eq!(doc["hooks"][0]["enabled"], false);

    set_kiro_hook_enabled(
        &config,
        "PostFileSave",
        Some("\\.ts$"),
        "npm run lint",
        true,
    )
    .unwrap();
    let doc2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(doc2["hooks"][0]["enabled"], true);

    // Unknown hook → NotFound, so callers can fall through to other files.
    let err = set_kiro_hook_enabled(&config, "Stop", None, "missing", false).unwrap_err();
    assert!(matches!(err, HkError::NotFound(_)));
}

#[test]
fn test_remove_hook_cursor_format() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("hooks.json");
    std::fs::write(
        &config,
        r#"{"version":1,"hooks":{"stop":[{"command":"echo done"},{"command":"echo other"}]}}"#,
    )
    .unwrap();

    remove_hook(&config, "stop", None, "echo done", HookFormat::Cursor).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let stops = content["hooks"]["stop"].as_array().unwrap();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["command"], "echo other");
}

#[test]
fn test_remove_hook_copilot_format() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("hooks.json");
    std::fs::write(&config, r#"{"version":1,"hooks":{"PreToolUse":[{"type":"command","command":"./check.sh"},{"type":"command","command":"./other.sh"}]}}"#).unwrap();

    remove_hook(
        &config,
        "PreToolUse",
        None,
        "./check.sh",
        HookFormat::Copilot,
    )
    .unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let hooks = content["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0]["command"], "./other.sh");
}

#[test]
fn test_remove_hook_windsurf_format() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("hooks.json");
    std::fs::write(
            &config,
            r#"{"hooks":{"post_cascade_response":[{"powershell":"python C:\\hooks\\log.py"},{"command":"echo other"}]}}"#,
        )
        .unwrap();

    remove_hook(
        &config,
        "post_cascade_response",
        None,
        "python C:\\hooks\\log.py",
        HookFormat::Windsurf,
    )
    .unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let hooks = content["hooks"]["post_cascade_response"]
        .as_array()
        .unwrap();
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0]["command"], "echo other");
}

#[test]
fn test_hermes_yaml_hook_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.yaml");
    std::fs::write(&cfg, "model:\n  default: x\n").unwrap();
    let entry = HookEntry {
        event: "pre_tool_call".into(),
        matcher: Some("terminal".into()),
        command: "~/.hermes/agent-hooks/block.sh".into(),
        enabled: true,
    };
    deploy_hook(&cfg, &entry, HookFormat::HermesYaml).unwrap();
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(
        doc.get("model")
            .and_then(|m| m.get("default"))
            .and_then(|v| v.as_str()),
        Some("x")
    );
    let saved = read_hook_config(
        &cfg,
        "pre_tool_call",
        Some("terminal"),
        "~/.hermes/agent-hooks/block.sh",
        HookFormat::HermesYaml,
    )
    .unwrap();
    assert!(saved.is_some());
    remove_hook(
        &cfg,
        "pre_tool_call",
        Some("terminal"),
        "~/.hermes/agent-hooks/block.sh",
        HookFormat::HermesYaml,
    )
    .unwrap();
    let after: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert!(after
        .get("hooks")
        .and_then(|h| h.get("pre_tool_call"))
        .is_none());
    restore_hook(
        &cfg,
        "pre_tool_call",
        &saved.unwrap(),
        HookFormat::HermesYaml,
    )
    .unwrap();
    let restored = read_hook_config(
        &cfg,
        "pre_tool_call",
        Some("terminal"),
        "~/.hermes/agent-hooks/block.sh",
        HookFormat::HermesYaml,
    )
    .unwrap();
    assert!(restored.is_some());
}

#[test]
fn test_hermes_yaml_hook_deploy_dedup() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.yaml");
    std::fs::write(&cfg, "model:\n  default: x\n").unwrap();
    let entry = HookEntry {
        event: "pre_tool_call".into(),
        matcher: Some("terminal".into()),
        command: "~/.hermes/agent-hooks/block.sh".into(),
        enabled: true,
    };
    // Deploying the identical hook twice must not duplicate the list item.
    deploy_hook(&cfg, &entry, HookFormat::HermesYaml).unwrap();
    deploy_hook(&cfg, &entry, HookFormat::HermesYaml).unwrap();
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    let seq = doc
        .get("hooks")
        .and_then(|h| h.get("pre_tool_call"))
        .and_then(|v| v.as_sequence())
        .expect("pre_tool_call should be a sequence");
    assert_eq!(seq.len(), 1, "duplicate deploy should be deduped");
}

#[test]
fn test_hermes_yaml_hook_matcherless_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.yaml");
    std::fs::write(&cfg, "model:\n  default: x\n").unwrap();
    let entry = HookEntry {
        event: "on_session_start".into(),
        matcher: None,
        command: "~/.hermes/agent-hooks/log.sh".into(),
        enabled: true,
    };
    deploy_hook(&cfg, &entry, HookFormat::HermesYaml).unwrap();

    // read_hook_config with matcher=None finds the matcher-less entry.
    let saved = read_hook_config(
        &cfg,
        "on_session_start",
        None,
        "~/.hermes/agent-hooks/log.sh",
        HookFormat::HermesYaml,
    )
    .unwrap();
    assert!(saved.is_some());

    // The written item must carry no `matcher` key.
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    let item = doc
        .get("hooks")
        .and_then(|h| h.get("on_session_start"))
        .and_then(|v| v.as_sequence())
        .and_then(|seq| seq.first())
        .expect("on_session_start should have one item");
    assert!(
        item.get("matcher").is_none(),
        "matcher-less hook must not write a matcher key"
    );
    assert_eq!(
        item.get("command").and_then(|v| v.as_str()),
        Some("~/.hermes/agent-hooks/log.sh")
    );
}

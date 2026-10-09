//! Per-agent toggle tests.

use super::*;
use tempfile::TempDir;

#[test]
fn toml_disable_enable_roundtrip_preserves_remote_fields() {
    // The old restore path narrowed snapshots to command/args/env,
    // destroying url/http_headers on re-enable.
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    let entry = remote_entry(McpTransport::Http);
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Toml)).unwrap();

    let snapshot = read_mcp_server_config(&config, "linear", McpFormat::Toml)
        .unwrap()
        .unwrap();
    remove_mcp_server(&config, "linear", McpFormat::Toml).unwrap();
    restore_mcp_server(&config, "linear", &snapshot, McpFormat::Toml).unwrap();

    let doc: toml::Table = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    let restored = doc["mcp_servers"]["linear"].as_table().unwrap();
    assert_eq!(
        restored["url"].as_str(),
        Some("https://mcp.linear.app/mcp"),
        "url must survive disable→enable"
    );
    assert_eq!(
        restored["http_headers"]["Authorization"].as_str(),
        Some("Bearer tok")
    );
    assert!(!restored.contains_key("command"));
}

#[test]
fn test_mcp_toml_disable_enable_roundtrip_with_sanitized_name() {
    // Full roundtrip: deploy → read → remove (disable) → restore (enable)
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    let original_name = "microsoft/markitdown";

    // 1. Deploy with a name that needs sanitization
    let entry = McpServerEntry {
        name: original_name.into(),
        command: "uvx".into(),
        args: vec!["markitdown-mcp@0.0.1a4".into()],
        env: Default::default(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Toml)).unwrap();

    // 2. Read (for saving before disable) — using original name
    let saved = read_mcp_server_config(&config, original_name, McpFormat::Toml)
        .unwrap()
        .expect("should read entry");

    // 3. Remove (disable) — using original name
    remove_mcp_server(&config, original_name, McpFormat::Toml).unwrap();
    assert!(
        read_mcp_server_config(&config, original_name, McpFormat::Toml)
            .unwrap()
            .is_none(),
        "entry should be gone after disable"
    );

    // 4. Restore (enable) — using original name
    restore_mcp_server(&config, original_name, &saved, McpFormat::Toml).unwrap();
    let restored = read_mcp_server_config(&config, original_name, McpFormat::Toml)
        .unwrap()
        .expect("should be restored");
    assert_eq!(restored["command"], "uvx");
}

#[test]
fn test_restore_plugin_entry() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("settings.json");
    std::fs::write(&config, r#"{"enabledPlugins":{}}"#).unwrap();

    restore_plugin_entry(&config, "my-plugin@source", &serde_json::json!(true)).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(content["enabledPlugins"]["my-plugin@source"], true);
}

#[test]
fn test_read_plugin_config() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("settings.json");
    std::fs::write(&config, r#"{"enabledPlugins":{"my-plugin@source":true}}"#).unwrap();

    let entry = read_plugin_config(&config, "my-plugin@source").unwrap();
    assert_eq!(entry.unwrap(), serde_json::json!(true));
}

#[test]
fn test_set_kiro_mcp_enabled_flips_disabled() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("mcp.json");
    std::fs::write(
        &config,
        r#"{"mcpServers":{"github":{"command":"npx","args":["server"]}}}"#,
    )
    .unwrap();
    set_kiro_mcp_enabled(&config, "github", false).unwrap();
    let disabled: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(disabled["mcpServers"]["github"]["disabled"], true);

    set_kiro_mcp_enabled(&config, "github", true).unwrap();
    let enabled: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert!(enabled["mcpServers"]["github"].get("disabled").is_none());
}

#[test]
fn test_set_omp_mcp_enabled_flips_flag_and_scrubs_lists() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("mcp.json");
    // Entry currently force-enabled via the allowlist; a stale denylist
    // entry for another server must survive untouched.
    std::fs::write(
            &config,
            r#"{
              "mcpServers": {"github": {"type": "http", "url": "https://example.com/mcp", "enabled": false}},
              "enabledServers": ["github"],
              "disabledServers": ["other"]
            }"#,
        )
        .unwrap();

    // Disable: entry flag set, name scrubbed from the allowlist (which
    // would otherwise override enabled:false), entry keys preserved.
    set_omp_mcp_enabled(&config, &config, "github", false).unwrap();
    let disabled: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(disabled["mcpServers"]["github"]["enabled"], false);
    assert_eq!(disabled["mcpServers"]["github"]["type"], "http");
    assert_eq!(
        disabled["mcpServers"]["github"]["url"],
        "https://example.com/mcp"
    );
    assert!(disabled["enabledServers"].as_array().unwrap().is_empty());
    assert_eq!(disabled["disabledServers"][0], "other");

    // Enable: flag removed (absent means enabled).
    set_omp_mcp_enabled(&config, &config, "github", true).unwrap();
    let enabled: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert!(enabled["mcpServers"]["github"].get("enabled").is_none());
    assert_eq!(
        enabled["mcpServers"]["github"]["url"],
        "https://example.com/mcp"
    );
}

#[test]
fn test_set_omp_mcp_enabled_project_entry_scrubs_user_denylist() {
    let dir = TempDir::new().unwrap();
    // Project entry file and user file are distinct for project scope.
    let project = dir.path().join("project-mcp.json");
    let user = dir.path().join("user-mcp.json");
    std::fs::write(
        &project,
        r#"{"mcpServers": {"srv": {"command": "echo", "enabled": false}}}"#,
    )
    .unwrap();
    std::fs::write(&user, r#"{"mcpServers": {}, "disabledServers": ["srv"]}"#).unwrap();

    set_omp_mcp_enabled(&project, &user, "srv", true).unwrap();
    let p: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    let u: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&user).unwrap()).unwrap();
    assert!(p["mcpServers"]["srv"].get("enabled").is_none());
    // Denylist would override the entry flag — must be scrubbed.
    assert!(u["disabledServers"].as_array().unwrap().is_empty());
}

#[test]
fn test_set_omp_mcp_enabled_missing_user_file_ok() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project-mcp.json");
    std::fs::write(&project, r#"{"mcpServers": {"srv": {"command": "echo"}}}"#).unwrap();
    let user = dir.path().join("does-not-exist.json");

    set_omp_mcp_enabled(&project, &user, "srv", false).unwrap();
    let p: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    assert_eq!(p["mcpServers"]["srv"]["enabled"], false);
    // No user file must not be created just to scrub a list.
    assert!(!user.exists());
}

#[test]
fn test_set_hermes_plugin_enabled_toggles_list() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.yaml");
    std::fs::write(&cfg, "plugins:\n  enabled:\n    - calculator\n").unwrap();
    set_hermes_plugin_enabled(&cfg, "weather", true).unwrap();
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    let list: Vec<&str> = doc["plugins"]["enabled"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(list.contains(&"calculator") && list.contains(&"weather"));
    set_hermes_plugin_enabled(&cfg, "calculator", false).unwrap();
    let doc2: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    let list2: Vec<&str> = doc2["plugins"]["enabled"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(!list2.contains(&"calculator") && list2.contains(&"weather"));
}

#[test]
fn test_set_hermes_plugin_enabled_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.yaml");
    std::fs::write(&cfg, "plugins:\n  enabled:\n    - calculator\n").unwrap();

    // Enabling an already-enabled plugin must not duplicate it.
    set_hermes_plugin_enabled(&cfg, "calculator", true).unwrap();
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    let list: Vec<&str> = doc["plugins"]["enabled"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(list, vec!["calculator"], "no duplicate on re-enable");

    // Disabling an absent plugin must be a clean no-op.
    set_hermes_plugin_enabled(&cfg, "ghost", false).unwrap();
    let doc2: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    let list2: Vec<&str> = doc2["plugins"]["enabled"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        list2,
        vec!["calculator"],
        "disabling absent plugin is a no-op"
    );
}

#[test]
fn test_set_hermes_mcp_enabled_flips_in_place_preserving_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.yaml");
    std::fs::write(
            &cfg,
            "mcp_servers:\n  github:\n    command: npx\n    args:\n    - -y\n    env:\n      TOKEN: secret123\n    tools:\n      include:\n      - a\n      - b\n    enabled: true\n  time:\n    command: uvx\n",
        )
        .unwrap();
    // disable github in place
    set_hermes_mcp_enabled(&cfg, "github", false).unwrap();
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    let gh = doc
        .get("mcp_servers")
        .and_then(|m| m.get("github"))
        .unwrap();
    assert_eq!(gh.get("enabled").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        gh.get("env")
            .and_then(|e| e.get("TOKEN"))
            .and_then(|v| v.as_str()),
        Some("secret123")
    );
    let include: Vec<&str> = gh["tools"]["include"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(include, vec!["a", "b"]);
    assert!(doc.get("mcp_servers").and_then(|m| m.get("time")).is_some());
    // re-enable
    set_hermes_mcp_enabled(&cfg, "github", true).unwrap();
    let doc2: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(
        doc2["mcp_servers"]["github"]["enabled"].as_bool(),
        Some(true)
    );
    // `time` has no `enabled` key on disk; disabling must INSERT enabled:false.
    set_hermes_mcp_enabled(&cfg, "time", false).unwrap();
    let doc3: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(
        doc3["mcp_servers"]["time"]["enabled"].as_bool(),
        Some(false)
    );
    // and `time` keeps its command (entry not rebuilt)
    assert_eq!(doc3["mcp_servers"]["time"]["command"].as_str(), Some("uvx"));
}

#[test]
fn test_set_hermes_mcp_enabled_missing_server_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.yaml");
    std::fs::write(&cfg, "mcp_servers:\n  time:\n    command: uvx\n").unwrap();
    assert!(set_hermes_mcp_enabled(&cfg, "ghost", false).is_err());
}

#[test]
fn test_set_gemini_extension_enabled_disable() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let ext_dir = home.join(".gemini").join("extensions");
    std::fs::create_dir_all(&ext_dir).unwrap();

    set_gemini_extension_enabled(&ext_dir, "my-ext", false, home).unwrap();

    let content: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(ext_dir.join("extension-enablement.json")).unwrap(),
    )
    .unwrap();
    let overrides = content["my-ext"]["overrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 1);
    let expected = format!("!{}/*", home.to_string_lossy());
    assert_eq!(overrides[0].as_str().unwrap(), expected);
}

#[test]
fn test_set_gemini_extension_enabled_enable() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let ext_dir = home.join(".gemini").join("extensions");
    std::fs::create_dir_all(&ext_dir).unwrap();

    set_gemini_extension_enabled(&ext_dir, "my-ext", false, home).unwrap();
    set_gemini_extension_enabled(&ext_dir, "my-ext", true, home).unwrap();

    let content: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(ext_dir.join("extension-enablement.json")).unwrap(),
    )
    .unwrap();
    let overrides = content["my-ext"]["overrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 1);
    let expected = format!("{}/*", home.to_string_lossy());
    assert_eq!(overrides[0].as_str().unwrap(), expected);
}

#[test]
fn test_set_gemini_extension_enabled_preserves_other_extensions() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let ext_dir = home.join(".gemini").join("extensions");
    std::fs::create_dir_all(&ext_dir).unwrap();

    std::fs::write(
        ext_dir.join("extension-enablement.json"),
        r#"{"other-ext": {"overrides": ["!/some/workspace/*"]}}"#,
    )
    .unwrap();

    set_gemini_extension_enabled(&ext_dir, "my-ext", false, home).unwrap();

    let content: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(ext_dir.join("extension-enablement.json")).unwrap(),
    )
    .unwrap();
    assert!(content["other-ext"]["overrides"].as_array().unwrap().len() == 1);
    assert!(content["my-ext"]["overrides"].as_array().unwrap().len() == 1);
}

#[test]
fn test_set_gemini_extension_enabled_preserves_workspace_rules() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let ext_dir = home.join(".gemini").join("extensions");
    std::fs::create_dir_all(&ext_dir).unwrap();

    let home_str = home.to_string_lossy();
    let initial = serde_json::json!({
        "my-ext": { "overrides": [
            format!("!/some/workspace/*"),
        ]}
    });
    std::fs::write(
        ext_dir.join("extension-enablement.json"),
        initial.to_string(),
    )
    .unwrap();

    set_gemini_extension_enabled(&ext_dir, "my-ext", false, home).unwrap();

    let content: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(ext_dir.join("extension-enablement.json")).unwrap(),
    )
    .unwrap();
    let overrides = content["my-ext"]["overrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 2);
    assert_eq!(overrides[0].as_str().unwrap(), "!/some/workspace/*");
    assert_eq!(overrides[1].as_str().unwrap(), format!("!{}/*", home_str));
}

#[test]
fn test_remove_gemini_extension_entry() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let ext_dir = home.join(".gemini").join("extensions");
    std::fs::create_dir_all(&ext_dir).unwrap();

    // Create enablement with two extensions
    set_gemini_extension_enabled(&ext_dir, "ext-a", false, home).unwrap();
    set_gemini_extension_enabled(&ext_dir, "ext-b", false, home).unwrap();

    // Remove one
    remove_gemini_extension_entry(&ext_dir, "ext-a").unwrap();

    let content: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(ext_dir.join("extension-enablement.json")).unwrap(),
    )
    .unwrap();
    assert!(content.get("ext-a").is_none(), "ext-a should be removed");
    assert!(content.get("ext-b").is_some(), "ext-b should remain");
}

#[test]
fn test_remove_codex_plugin_entry() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");

    // Set up two plugin entries
    set_codex_plugin_enabled(&config, "pluginA@marketplace", false).unwrap();
    set_codex_plugin_enabled(&config, "pluginB@marketplace", true).unwrap();

    // Remove one
    remove_codex_plugin_entry(&config, "pluginA@marketplace").unwrap();

    let content: toml::Table = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    let plugins = content["plugins"].as_table().unwrap();
    assert!(!plugins.contains_key("pluginA@marketplace"));
    assert!(plugins.contains_key("pluginB@marketplace"));
}

#[test]
fn test_remove_vscode_plugin_entry() {
    let dir = TempDir::new().unwrap();
    let gs = dir.path().join("globalStorage");
    std::fs::create_dir_all(&gs).unwrap();
    let db_path = gs.join("state.vscdb");

    // Set up state.vscdb
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT UNIQUE, value TEXT)",
        [],
    )
    .unwrap();

    // Add two entries
    set_vscode_plugin_enabled(dir.path(), "file:///plugin-a", false).unwrap();
    set_vscode_plugin_enabled(dir.path(), "file:///plugin-b", true).unwrap();

    // Remove one
    remove_vscode_plugin_entry(dir.path(), "file:///plugin-a").unwrap();

    let result: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'agentPlugins.enablement'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let entries: Vec<(String, bool)> = serde_json::from_str(&result).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "file:///plugin-b");
}

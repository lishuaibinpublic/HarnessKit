//! MCP deploy/remove/restore/read tests.

use super::*;
use tempfile::TempDir;

#[test]
fn build_mcp_json_value_spells_each_remote_schema() {
    let http = remote_entry(McpTransport::Http);
    let sse = remote_entry(McpTransport::Sse);

    let v = build_mcp_json_value(&http, RemoteMcpSchema::TypeAndUrl).unwrap();
    assert_eq!(v["type"], "http");
    assert_eq!(v["url"], "https://mcp.linear.app/mcp");
    assert_eq!(v["headers"]["Authorization"], "Bearer tok");
    assert!(v.get("command").is_none());
    assert_eq!(
        build_mcp_json_value(&sse, RemoteMcpSchema::TypeAndUrl).unwrap()["type"],
        "sse"
    );

    let v = build_mcp_json_value(&http, RemoteMcpSchema::PlainUrl).unwrap();
    assert_eq!(v["url"], "https://mcp.linear.app/mcp");
    assert!(v.get("type").is_none());

    // Gemini spells the transport through the key itself.
    let v = build_mcp_json_value(&http, RemoteMcpSchema::GeminiSplit).unwrap();
    assert_eq!(v["httpUrl"], "https://mcp.linear.app/mcp");
    assert!(v.get("url").is_none());
    let v = build_mcp_json_value(&sse, RemoteMcpSchema::GeminiSplit).unwrap();
    assert_eq!(v["url"], "https://mcp.linear.app/mcp");
    assert!(v.get("httpUrl").is_none());

    let v = build_mcp_json_value(&http, RemoteMcpSchema::ServerUrl).unwrap();
    assert_eq!(v["serverUrl"], "https://mcp.linear.app/mcp");
}

#[test]
fn validate_remote_mcp_target_rejects_unsupported_combinations() {
    let http = remote_entry(McpTransport::Http);
    let sse = remote_entry(McpTransport::Sse);

    let err =
        validate_remote_mcp_target(&http, "someagent", RemoteMcpSchema::Unsupported).unwrap_err();
    assert!(matches!(&err, HkError::Validation(m) if m.contains("someagent")));

    // Codex (TOML) is HTTP-only.
    let err = validate_remote_mcp_target(&sse, "codex", RemoteMcpSchema::Toml).unwrap_err();
    assert!(matches!(&err, HkError::Validation(m) if m.contains("not SSE")));
    validate_remote_mcp_target(&http, "codex", RemoteMcpSchema::Toml).unwrap();

    // dsh (DshTransport) is HTTP-only too.
    let err = validate_remote_mcp_target(&sse, "dsh", RemoteMcpSchema::DshTransport).unwrap_err();
    assert!(matches!(&err, HkError::Validation(m) if m.contains("not SSE")));
    assert!(validate_remote_mcp_target(&http, "dsh", RemoteMcpSchema::DshTransport).is_ok());

    // Remote without url is corrupt regardless of target.
    let mut broken = remote_entry(McpTransport::Http);
    broken.url = None;
    let err =
        validate_remote_mcp_target(&broken, "claude", RemoteMcpSchema::TypeAndUrl).unwrap_err();
    assert!(matches!(err, HkError::ConfigCorrupted(_)));
}

#[test]
fn deploy_remote_mcp_json_end_to_end_per_agent_spelling() {
    // Full deploy_mcp_server dispatch (validate + format + remote schema)
    // for the JSON-family agents, not just the value builder.
    let dir = TempDir::new().unwrap();

    // Claude (TypeAndUrl): {type, url, headers} under mcpServers.
    let config = dir.path().join("claude.json");
    let entry = remote_entry(McpTransport::Http);
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::McpServers)).unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let server = &doc["mcpServers"]["linear"];
    assert_eq!(server["type"], "http");
    assert_eq!(server["url"], "https://mcp.linear.app/mcp");
    assert_eq!(server["headers"]["Authorization"], "Bearer tok");
    assert!(server.get("command").is_none());

    // Gemini (GeminiSplit): SSE entries land under `url`, not `httpUrl`.
    let config = dir.path().join("gemini.json");
    let sse = remote_entry(McpTransport::Sse);
    let gemini = crate::adapter::gemini::GeminiAdapter::with_home("/nonexistent".into());
    deploy_mcp_server(&config, &sse, &gemini).unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let server = &doc["mcpServers"]["linear"];
    assert_eq!(server["url"], "https://mcp.linear.app/mcp");
    assert!(server.get("httpUrl").is_none());
    assert!(server.get("type").is_none());

    // Windsurf (ServerUrl): single serverUrl key regardless of protocol.
    let config = dir.path().join("windsurf.json");
    let windsurf = crate::adapter::windsurf::WindsurfAdapter::with_home("/nonexistent".into());
    deploy_mcp_server(&config, &entry, &windsurf).unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let server = &doc["mcpServers"]["linear"];
    assert_eq!(server["serverUrl"], "https://mcp.linear.app/mcp");
    assert!(server.get("url").is_none());
}

#[test]
fn deploy_remote_mcp_hermes_yaml_writes_url_headers_and_transport() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.yaml");
    let mut entry = remote_entry(McpTransport::Sse);
    entry.name = "stripe".into();
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::HermesYaml)).unwrap();

    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let server = &doc["mcp_servers"]["stripe"];
    assert_eq!(server["url"].as_str(), Some("https://mcp.linear.app/mcp"));
    assert_eq!(server["transport"].as_str(), Some("sse"));
    assert_eq!(
        server["headers"]["Authorization"].as_str(),
        Some("Bearer tok")
    );
    assert!(server.get("command").is_none());
}

#[test]
fn deploy_remote_mcp_opencode_writes_remote_type() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("opencode.json");
    std::fs::write(&config, "{}").unwrap();
    let entry = remote_entry(McpTransport::Http);
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Opencode)).unwrap();

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let server = &doc["mcp"]["linear"];
    assert_eq!(server["type"], "remote");
    assert_eq!(server["url"], "https://mcp.linear.app/mcp");
    assert_eq!(server["headers"]["Authorization"], "Bearer tok");
    assert!(server.get("command").is_none());
}

#[test]
fn deploy_remote_mcp_grok_strips_url_aliases() {
    // A redeploy over a urlTemplate-keyed entry must leave exactly one
    // url spelling — url + urlTemplate together is a duplicate-field
    // error that makes Grok drop the whole entry.
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "[mcp_servers.linear]\nurlTemplate = \"https://old.example\"\n",
    )
    .unwrap();
    let entry = remote_entry(McpTransport::Http);
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::GrokToml)).unwrap();
    let doc: toml::Value = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    let server = doc["mcp_servers"]["linear"].as_table().unwrap();
    assert_eq!(server["url"].as_str(), Some("https://mcp.linear.app/mcp"));
    assert!(!server.contains_key("urlTemplate"), "{server:?}");

    // Same strip on the stdio branch: a remote→stdio redeploy over a
    // urlTemplate-keyed entry must not leave the alias behind.
    let stdio = McpServerEntry {
        name: "linear".into(),
        command: "npx".into(),
        args: vec![],
        env: Default::default(),
        transport: McpTransport::Stdio,
        url: None,
        headers: Default::default(),
        enabled: true,
    };
    std::fs::write(
        &config,
        "[mcp_servers.linear]\nurl_template = \"https://old.example\"\n",
    )
    .unwrap();
    deploy_mcp_server(&config, &stdio, &*test_adapter(McpFormat::GrokToml)).unwrap();
    let doc: toml::Value = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    let server = doc["mcp_servers"]["linear"].as_table().unwrap();
    assert_eq!(server["command"].as_str(), Some("npx"));
    assert!(!server.contains_key("url_template"), "{server:?}");
}

#[test]
fn deploy_remote_mcp_grok_writes_headers_and_sse_type() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
            &config,
            "theme = \"dark\"\n\n[mcp_servers.linear]\ncwd = \"/keep\"\nurl = \"https://old.example\"\n",
        )
        .unwrap();
    let entry = remote_entry(McpTransport::Sse);
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::GrokToml)).unwrap();

    let doc: toml::Value = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    let server = &doc["mcp_servers"]["linear"];
    assert_eq!(server["url"].as_str(), Some("https://mcp.linear.app/mcp"));
    assert_eq!(server["type"].as_str(), Some("sse"));
    assert_eq!(
        server["headers"]["Authorization"].as_str(),
        Some("Bearer tok")
    );
    assert_eq!(server["cwd"].as_str(), Some("/keep"));
    assert_eq!(doc["theme"].as_str(), Some("dark"));
    assert!(server.get("http_headers").is_none());
    assert!(server.get("command").is_none());
}

#[test]
fn test_remove_mcp_server_opencode_preserves_comments() {
    // End-to-end: remove only the targeted entry; surrounding user
    // comments and sibling entries stay verbatim. The comment that
    // was directly above the removed entry stays as an "orphan" by
    // design — HK never edits user comment text.
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("opencode.jsonc");
    std::fs::write(
            &config,
            "{\n  // top note\n  \"model\": \"x\",\n  \"mcp\": {\n    // about github\n    \"github\": {\"type\": \"local\", \"command\": [\"a\"]},\n    // about filesystem\n    \"filesystem\": {\"type\": \"local\", \"command\": [\"b\"]}\n  }\n}\n",
        )
        .unwrap();

    remove_mcp_server(&config, "github", McpFormat::Opencode).unwrap();

    let written = std::fs::read_to_string(&config).unwrap();
    assert!(written.contains("// top note"));
    assert!(
        written.contains("// about filesystem"),
        "sibling comment dropped"
    );
    assert!(written.contains("\"filesystem\""), "sibling entry lost");
    assert!(!written.contains("\"github\""), "target entry not removed");
}

#[test]
fn test_restore_mcp_server_opencode_preserves_comments() {
    // End-to-end: restoring a previously-saved entry into mcp keeps
    // every other comment, formatting, and sibling intact. Mirrors
    // the HK toggle flow (disable → restore).
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("opencode.jsonc");
    std::fs::write(
            &config,
            "{\n  // top note\n  \"model\": \"x\",\n  \"mcp\": {\n    // about github\n    \"github\": {\"type\": \"local\", \"command\": [\"a\"]}\n  }\n}\n",
        )
        .unwrap();

    let saved = serde_json::json!({"type": "local", "command": ["b"]});
    restore_mcp_server(&config, "filesystem", &saved, McpFormat::Opencode).unwrap();

    let written = std::fs::read_to_string(&config).unwrap();
    assert!(written.contains("// top note"));
    assert!(written.contains("// about github"));
    assert!(written.contains("\"github\""), "existing entry lost");
    assert!(written.contains("\"filesystem\""), "restored entry missing");
}

#[test]
fn test_deploy_mcp_server_opencode_preserves_comments() {
    // End-to-end guarantee for the deploy path (cross-agent install
    // into OpenCode): existing user comments and formatting outside
    // the touched mcp entry survive intact.
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("opencode.jsonc");
    std::fs::write(
            &config,
            "{\n  // top note kept\n  \"model\": \"claude-opus-4\",\n  \"mcp\": {\n    // about github\n    \"github\": {\"type\": \"local\", \"command\": [\"existing\"]}\n  }\n}\n",
        )
        .unwrap();

    let entry = McpServerEntry {
        name: "filesystem".into(),
        command: "npx".into(),
        args: vec!["-y".into(), "@mcp/fs".into()],
        env: std::collections::HashMap::new(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Opencode)).unwrap();

    let written = std::fs::read_to_string(&config).unwrap();
    assert!(
        written.contains("// top note kept"),
        "top-level comment dropped"
    );
    assert!(
        written.contains("// about github"),
        "mcp child comment dropped"
    );
    assert!(written.contains("\"github\""), "existing entry lost");
    assert!(written.contains("\"filesystem\""), "deployed entry missing");
    assert!(
        written.contains("\"npx\""),
        "deployed entry's command missing"
    );
}

#[test]
fn test_deploy_mcp_server_new_file() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("mcp.json");
    let entry = McpServerEntry {
        name: "github".into(),
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
        env: [("GITHUB_TOKEN".into(), "ghp_test".into())].into(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::McpServers)).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let server = &content["mcpServers"]["github"];
    assert_eq!(server["command"], "npx");
    assert_eq!(server["args"][0], "-y");
    assert_eq!(server["env"]["GITHUB_TOKEN"], "ghp_test");
}

#[test]
fn test_deploy_mcp_server_existing_file() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("settings.json");
    std::fs::write(
        &config,
        r#"{"theme":"dark","mcpServers":{"existing":{"command":"node"}}}"#,
    )
    .unwrap();

    let entry = McpServerEntry {
        name: "new-server".into(),
        command: "python".into(),
        args: vec!["server.py".into()],
        env: Default::default(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::McpServers)).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(content["theme"], "dark"); // preserved
    assert_eq!(content["mcpServers"]["existing"]["command"], "node"); // preserved
    assert_eq!(content["mcpServers"]["new-server"]["command"], "python"); // added
}

#[test]
fn test_deploy_mcp_server_servers_format() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("mcp.json");
    let entry = McpServerEntry {
        name: "memory".into(),
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-memory".into()],
        env: Default::default(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Servers)).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert!(
        content.get("mcpServers").is_none(),
        "should not use mcpServers key"
    );
    let server = &content["servers"]["memory"];
    assert_eq!(server["command"], "npx");
}

#[test]
fn test_deploy_mcp_server_toml_format() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    // Existing TOML content to preserve
    std::fs::write(&config, "model = \"o4-mini\"\n").unwrap();

    let entry = McpServerEntry {
        name: "context7".into(),
        command: "npx".into(),
        args: vec!["-y".into(), "@upstash/context7-mcp".into()],
        env: [("MY_KEY".into(), "val".into())].into(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Toml)).unwrap();

    let content = std::fs::read_to_string(&config).unwrap();
    let doc: toml::Table = content.parse().unwrap();
    assert_eq!(doc["model"].as_str().unwrap(), "o4-mini"); // preserved
    let server = doc["mcp_servers"]["context7"].as_table().unwrap();
    assert_eq!(server["command"].as_str().unwrap(), "npx");
    assert_eq!(
        server["args"].as_array().unwrap()[0].as_str().unwrap(),
        "-y"
    );
    assert_eq!(server["env"]["MY_KEY"].as_str().unwrap(), "val");
}

#[test]
fn test_deploy_mcp_server_opencode_format() {
    // OpenCode schema (https://opencode.ai/config.json):
    //   - top-level key "mcp"
    //   - entry must declare type: "local"
    //   - command is a single array merging the binary + its args
    //   - env block is named "environment"
    //   - additionalProperties: false → no separate "args"/"env" fields
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("opencode.json");
    let entry = McpServerEntry {
        name: "github".into(),
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
        env: [("GITHUB_TOKEN".into(), "ghp_test".into())].into(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Opencode)).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();

    assert!(
        content.get("mcpServers").is_none(),
        "must not use the Claude-style mcpServers key"
    );
    let server = &content["mcp"]["github"];
    assert_eq!(server["type"], "local");
    assert_eq!(server["command"][0], "npx");
    assert_eq!(server["command"][1], "-y");
    assert_eq!(server["command"][2], "@modelcontextprotocol/server-github");
    assert_eq!(server["environment"]["GITHUB_TOKEN"], "ghp_test");
    // additionalProperties: false is enforced upstream — verify we honor it.
    assert!(
        server.get("args").is_none(),
        "must not emit a separate args field"
    );
    assert!(
        server.get("env").is_none(),
        "must use 'environment', not 'env'"
    );
}

#[test]
fn test_deploy_mcp_server_opencode_omits_environment_when_empty() {
    // Schema marks `environment` optional. Emitting `"environment": {}` is
    // legal but noisy; we omit the field entirely when the source has no
    // env vars to keep the on-disk config minimal.
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("opencode.json");
    let entry = McpServerEntry {
        name: "memory".into(),
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-memory".into()],
        env: Default::default(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Opencode)).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let server = &content["mcp"]["memory"];
    assert_eq!(server["type"], "local");
    assert!(server["command"].is_array());
    assert!(
        server.get("environment").is_none(),
        "should omit environment field when source has no env vars"
    );
}

#[test]
fn test_deploy_mcp_server_opencode_preserves_existing_keys() {
    // OpenCode's opencode.json holds many top-level keys (model, agent,
    // skills, etc.). Deploy must merge into the existing "mcp" object
    // without clobbering siblings or sibling-format settings.
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("opencode.json");
    std::fs::write(
            &config,
            r#"{"model":"claude-sonnet-4-6","mcp":{"existing":{"type":"local","command":["node","s.js"]}}}"#,
        )
        .unwrap();

    let entry = McpServerEntry {
        name: "added".into(),
        command: "python".into(),
        args: vec!["server.py".into()],
        env: Default::default(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Opencode)).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(content["model"], "claude-sonnet-4-6"); // sibling preserved
    assert_eq!(content["mcp"]["existing"]["command"][0], "node"); // sibling entry preserved
    assert_eq!(content["mcp"]["added"]["command"][0], "python"); // new entry added
}

#[test]
fn test_opencode_remove_restore_and_read_uses_mcp_key() {
    // Exercise the three json_top_key code paths (remove/restore/read) for
    // McpFormat::Opencode in one round-trip. Regression guard: an earlier
    // implementation routed Opencode through the wildcard arm and silently
    // operated on "mcpServers" instead of "mcp".
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("opencode.json");
    std::fs::write(
            &config,
            r#"{"mcp":{"github":{"type":"local","command":["npx","server-github"],"environment":{"TOKEN":"abc"}}}}"#,
        )
        .unwrap();

    // read
    let saved = read_mcp_server_config(&config, "github", McpFormat::Opencode).unwrap();
    assert!(
        saved.is_some(),
        "read must find entry under 'mcp', not 'mcpServers'"
    );
    let saved = saved.unwrap();
    assert_eq!(saved["environment"]["TOKEN"], "abc");

    // remove
    remove_mcp_server(&config, "github", McpFormat::Opencode).unwrap();
    let after_remove = read_mcp_server_config(&config, "github", McpFormat::Opencode).unwrap();
    assert!(after_remove.is_none(), "remove must delete from 'mcp' key");

    // restore
    restore_mcp_server(&config, "github", &saved, McpFormat::Opencode).unwrap();
    let restored = read_mcp_server_config(&config, "github", McpFormat::Opencode).unwrap();
    assert_eq!(
        restored.unwrap(),
        saved,
        "restored entry must match what was saved (bit-perfect round-trip)"
    );

    // Confirm the entry actually lives under "mcp" on disk.
    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert!(content.get("mcp").is_some());
    assert!(
        content.get("mcpServers").is_none(),
        "must not have leaked into mcpServers via fallback"
    );
}

#[test]
fn test_opencode_deploy_then_adapter_read_roundtrip() {
    // Cross-module integration: bytes deployer writes must be exactly what
    // the OpencodeAdapter's parser reads back — i.e. a McpServerEntry
    // survives a full write→read loop with command/args/env intact.
    use crate::adapter::AgentAdapter;
    use crate::adapter::opencode::OpencodeAdapter;

    let dir = TempDir::new().unwrap();
    let config = dir.path().join("opencode.json");
    let original = McpServerEntry {
        name: "context7".into(),
        command: "npx".into(),
        args: vec!["-y".into(), "@upstash/context7-mcp".into()],
        env: [("API_KEY".into(), "k1".into())].into(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &original, &*test_adapter(McpFormat::Opencode)).unwrap();

    let adapter = OpencodeAdapter::with_home(dir.path().to_path_buf());
    let entries = adapter.read_mcp_servers_from(&config);
    assert_eq!(entries.len(), 1);
    let read_back = &entries[0];
    assert_eq!(read_back.name, original.name);
    assert_eq!(read_back.command, original.command);
    assert_eq!(read_back.args, original.args);
    assert_eq!(read_back.env, original.env);
}

#[test]
fn test_sanitize_mcp_name_replaces_slash() {
    assert_eq!(
        sanitize_mcp_name("microsoft/markitdown"),
        "microsoft-markitdown"
    );
}

#[test]
fn test_sanitize_mcp_name_preserves_valid_chars() {
    assert_eq!(sanitize_mcp_name("my_server-1"), "my_server-1");
}

#[test]
fn test_deploy_mcp_server_toml_sanitizes_name_and_preserves_original() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    let entry = McpServerEntry {
        name: "microsoft/markitdown".into(),
        command: "uvx".into(),
        args: vec!["markitdown-mcp@0.0.1a4".into()],
        env: Default::default(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Toml)).unwrap();

    let doc: toml::Table = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    let servers = doc["mcp_servers"].as_table().unwrap();
    // TOML key should be sanitized: "/" → "-"
    assert!(servers.contains_key("microsoft-markitdown"));
    assert!(!servers.contains_key("microsoft/markitdown"));
    // Original name preserved in _hk_name for scanner round-trip
    let server = servers["microsoft-markitdown"].as_table().unwrap();
    assert_eq!(server["_hk_name"].as_str().unwrap(), "microsoft/markitdown");
}

#[test]
fn test_deploy_mcp_server_toml_no_hk_name_when_unchanged() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    let entry = McpServerEntry {
        name: "context7".into(),
        command: "npx".into(),
        args: vec![],
        env: Default::default(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Toml)).unwrap();

    let doc: toml::Table = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    let server = doc["mcp_servers"]["context7"].as_table().unwrap();
    // No _hk_name needed when name didn't require sanitization
    assert!(!server.contains_key("_hk_name"));
}

#[cfg(not(target_os = "windows"))]
#[test]
fn test_resolve_command_path_absolute_passthrough() {
    // Already absolute paths should be returned unchanged.
    assert_eq!(resolve_command_path("/usr/bin/env"), "/usr/bin/env");
}

#[cfg(not(target_os = "windows"))]
#[test]
fn test_resolve_command_path_resolves_known_command() {
    // "ls" should resolve to an absolute path on any Unix system.
    let resolved = resolve_command_path("ls");
    assert!(
        resolved.starts_with('/'),
        "expected absolute path, got: {resolved}"
    );
}

#[test]
fn test_resolve_command_path_unknown_fallback() {
    // Non-existent command should return the original string.
    assert_eq!(
        resolve_command_path("__nonexistent_cmd_12345__"),
        "__nonexistent_cmd_12345__"
    );
}

#[cfg(not(target_os = "windows"))]
#[test]
fn test_build_path_for_command_includes_parent_dir() {
    let path = build_path_for_command("/Users/zoe/.nvm/versions/node/v24.13.0/bin/npx");
    assert_eq!(
        path.unwrap(),
        "/Users/zoe/.nvm/versions/node/v24.13.0/bin:/usr/local/bin:/usr/bin:/bin"
    );
}

#[test]
fn test_build_path_for_command_bare_name_returns_none() {
    // Bare command name (no directory) should return None.
    assert!(build_path_for_command("npx").is_none());
}

#[test]
#[cfg(target_os = "windows")]
fn test_resolve_command_path_absolute_passthrough_windows() {
    assert_eq!(
        resolve_command_path(r"C:\Windows\System32\cmd.exe"),
        r"C:\Windows\System32\cmd.exe"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn test_resolve_command_path_resolves_known_command_windows() {
    let resolved = resolve_command_path("cmd");
    assert!(
        crate::sanitize::is_windows_abs_path(&resolved),
        "expected absolute path, got: {resolved}"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn test_build_path_for_command_includes_parent_dir_windows() {
    let path = build_path_for_command(r"C:\Users\test\AppData\Local\Programs\node\npx.exe");
    assert_eq!(
        path.unwrap(),
        r"C:\Users\test\AppData\Local\Programs\node;C:\Windows\System32;C:\Windows"
    );
}

#[test]
fn test_read_mcp_server_config_toml_finds_sanitized_key() {
    // When the TOML key is sanitized ("microsoft-markitdown") but the caller
    // uses the original name ("microsoft/markitdown"), the lookup should still work.
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    let entry = McpServerEntry {
        name: "microsoft/markitdown".into(),
        command: "uvx".into(),
        args: vec!["markitdown-mcp@0.0.1a4".into()],
        env: Default::default(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Toml)).unwrap();

    // Read using the original (unsanitized) name
    let result = read_mcp_server_config(&config, "microsoft/markitdown", McpFormat::Toml).unwrap();
    assert!(result.is_some(), "should find entry via original name");
    assert_eq!(result.unwrap()["command"], "uvx");
}

#[test]
fn test_remove_mcp_server_toml_removes_sanitized_key() {
    // remove_mcp_server should find and remove the sanitized TOML key
    // when called with the original name.
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    let entry = McpServerEntry {
        name: "microsoft/markitdown".into(),
        command: "uvx".into(),
        args: vec!["markitdown-mcp@0.0.1a4".into()],
        env: Default::default(),
        enabled: true,
        ..Default::default()
    };
    deploy_mcp_server(&config, &entry, &*test_adapter(McpFormat::Toml)).unwrap();

    // Remove using the original name
    remove_mcp_server(&config, "microsoft/markitdown", McpFormat::Toml).unwrap();

    // Verify it's gone
    let result = read_mcp_server_config(&config, "microsoft/markitdown", McpFormat::Toml).unwrap();
    assert!(result.is_none(), "entry should be removed");
}

#[test]
fn test_restore_mcp_server() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("settings.json");
    std::fs::write(&config, r#"{"mcpServers":{}}"#).unwrap();

    let entry_json = r#"{"command":"npx","args":["-y","@mcp/github"],"env":{"TOKEN":"abc"}}"#;
    let entry: serde_json::Value = serde_json::from_str(entry_json).unwrap();
    restore_mcp_server(&config, "github", &entry, McpFormat::McpServers).unwrap();

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(content["mcpServers"]["github"]["command"], "npx");
    assert_eq!(content["mcpServers"]["github"]["env"]["TOKEN"], "abc");
}

#[test]
fn test_read_mcp_server_config() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("settings.json");
    std::fs::write(
        &config,
        r#"{"mcpServers":{"github":{"command":"npx","args":["-y"]}}}"#,
    )
    .unwrap();

    let entry = read_mcp_server_config(&config, "github", McpFormat::McpServers).unwrap();
    assert!(entry.is_some());
    assert_eq!(entry.unwrap()["command"], "npx");

    let missing = read_mcp_server_config(&config, "nonexistent", McpFormat::McpServers).unwrap();
    assert!(missing.is_none());
}

#[test]
fn test_remove_and_restore_mcp_roundtrip() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("settings.json");
    std::fs::write(
        &config,
        r#"{"mcpServers":{"github":{"command":"npx","args":["-y"],"env":{}}}}"#,
    )
    .unwrap();

    // Read, remove, restore
    let saved = read_mcp_server_config(&config, "github", McpFormat::McpServers)
        .unwrap()
        .unwrap();
    remove_mcp_server(&config, "github", McpFormat::McpServers).unwrap();

    let after_remove: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert!(after_remove["mcpServers"].get("github").is_none());

    restore_mcp_server(&config, "github", &saved, McpFormat::McpServers).unwrap();
    let after_restore: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(after_restore["mcpServers"]["github"]["command"], "npx");
}

fn openclaw_config(v: &str) -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("openclaw.json");
    std::fs::write(&cfg, v).unwrap();
    (tmp, cfg)
}

fn parse_openclaw(path: &std::path::Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap();
    jsonc_parser::parse_to_serde_value::<serde_json::Value>(&text, &Default::default()).unwrap()
}

#[test]
fn test_deploy_mcp_server_openclaw_creates_nested_servers_keeping_json5_style() {
    let (_tmp, cfg) =
        openclaw_config("{\n  // hand-written gateway config\n  models: { default: 'gpt' },\n}\n");
    let entry = McpServerEntry {
        name: "fs".into(),
        command: "npx".into(),
        args: vec!["-y".into(), "srv".into()],
        env: [("K".to_string(), "V".to_string())].into_iter().collect(),
        ..Default::default()
    };
    deploy_mcp_server(
        &cfg,
        &entry,
        test_adapter(McpFormat::OpenClawJson5).as_ref(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        text.contains("// hand-written gateway config"),
        "comment lost: {text}"
    );
    assert!(
        text.contains("default: 'gpt'"),
        "single-quoted value lost: {text}"
    );
    let v = parse_openclaw(&cfg);
    let fs = v.pointer("/mcp/servers/fs").unwrap();
    assert_eq!(fs["command"], "npx");
    assert_eq!(fs["args"][0], "-y");
    assert_eq!(fs["env"]["K"], "V");
}

#[test]
fn test_openclaw_remove_and_redeploy_and_toggle_paths() {
    let (_tmp, cfg) = openclaw_config(
        "{\n  mcp: { servers: { a: { command: 'x' }, b: { command: 'y', enabled: false } } },\n}\n",
    );
    // remove drops only the one nested key
    remove_mcp_server(&cfg, "a", McpFormat::OpenClawJson5).unwrap();
    let v = parse_openclaw(&cfg);
    assert!(v.pointer("/mcp/servers/a").is_none());
    assert!(v.pointer("/mcp/servers/b").is_some());

    // redeploying an existing server updates the stdio keys but keeps the
    // user's `enabled: false`
    let entry = McpServerEntry {
        name: "b".into(),
        command: "z".into(),
        ..Default::default()
    };
    deploy_mcp_server(
        &cfg,
        &entry,
        test_adapter(McpFormat::OpenClawJson5).as_ref(),
    )
    .unwrap();
    let v = parse_openclaw(&cfg);
    assert_eq!(
        v.pointer("/mcp/servers/b/command").unwrap(),
        &serde_json::json!("z")
    );
    assert_eq!(
        v.pointer("/mcp/servers/b/enabled").unwrap(),
        &serde_json::json!(false)
    );

    // native toggle flips `enabled` in place, other keys untouched
    crate::deployer::set_openclaw_mcp_enabled(&cfg, "b", true).unwrap();
    let v = parse_openclaw(&cfg);
    assert_eq!(
        v.pointer("/mcp/servers/b/enabled").unwrap(),
        &serde_json::json!(true)
    );
    assert_eq!(
        v.pointer("/mcp/servers/b/command").unwrap(),
        &serde_json::json!("z")
    );
    assert!(parse_openclaw(&cfg).pointer("/mcp/servers/a").is_none());

    // unknown server fails loudly instead of writing a stray entry
    assert!(matches!(
        crate::deployer::set_openclaw_mcp_enabled(&cfg, "ghost", false),
        Err(HkError::NotFound(_))
    ));
}

#[test]
fn test_openclaw_stdio_redeploy_over_remote_entry_drops_url_keys() {
    let (_tmp, cfg) = openclaw_config(
        "{\n  mcp: { servers: { web: { url: 'https://x/mcp', transport: 'streamable-http', type: 'http', headers: { Authorization: 'Bearer t' }, enabled: false } } },\n}\n",
    );
    let entry = McpServerEntry {
        name: "web".into(),
        command: "npx".into(),
        ..Default::default()
    };
    deploy_mcp_server(
        &cfg,
        &entry,
        test_adapter(McpFormat::OpenClawJson5).as_ref(),
    )
    .unwrap();
    let web = parse_openclaw(&cfg)
        .pointer("/mcp/servers/web")
        .unwrap()
        .clone();
    assert_eq!(web["command"], "npx");
    for gone in ["url", "transport", "type", "headers"] {
        assert!(web.get(gone).is_none(), "{gone} should be dropped: {web}");
    }
    assert_eq!(web["enabled"], false, "user's enabled flag survives");
}

#[test]
fn test_openclaw_toggle_and_remove_never_create_the_gateway_config() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("openclaw.json");
    assert!(matches!(
        crate::deployer::set_openclaw_mcp_enabled(&missing, "a", true),
        Err(HkError::NotFound(_))
    ));
    remove_mcp_server(&missing, "a", McpFormat::OpenClawJson5).unwrap();
    assert!(
        !missing.exists(),
        "an empty openclaw.json would stop the gateway"
    );
}

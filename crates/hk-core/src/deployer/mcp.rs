//! MCP server deploy/remove/restore/read across all config formats.

use super::*;
use crate::deployer::dsh::*;
use crate::deployer::fs_util::*;
use crate::deployer::toml_util::*;

/// Sanitize an MCP server name to contain only `[a-zA-Z0-9_-]`.
///
/// Codex requires server names to match `^[a-zA-Z0-9_-]+$`, and TOML bare keys
/// also cannot contain characters like `/`. This replaces any disallowed character
/// with `-` so that names like `microsoft/markitdown` become `microsoft-markitdown`.
pub fn sanitize_mcp_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Resolve a command name to its absolute path using `which`.
///
/// GUI-based agents (e.g. Antigravity) do not inherit the user's shell `$PATH`,
/// so bare command names like `npx` or `uvx` fail with ENOENT. This resolves the
/// command to an absolute path (e.g. `/Users/zoe/.local/bin/uvx`) at deploy time.
/// Returns the original command unchanged if resolution fails.
pub fn resolve_command_path(command: &str) -> String {
    // Already absolute — nothing to do.
    // Unix: starts with '/'
    // Windows: starts with drive letter like 'C:\'
    if command.starts_with('/') || crate::sanitize::is_windows_abs_path(command) {
        return command.to_string();
    }
    crate::scanner::run_which(command).unwrap_or_else(|| command.to_string())
}

/// Build a PATH value that includes the directory of the resolved command.
///
/// GUI-based agents don't inherit the user's shell PATH, so scripts like `npx`
/// (which use `#!/usr/bin/env node`) fail because `node` isn't found.
/// This constructs a PATH containing the command's directory plus essential
/// system directories, ensuring sibling binaries (e.g. `node` next to `npx`)
/// are discoverable.
pub fn build_path_for_command(resolved_command: &str) -> Option<String> {
    let parent = std::path::Path::new(resolved_command).parent()?;
    let parent_str = parent.to_str()?;
    if parent_str.is_empty() {
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        Some(format!(r"{};C:\Windows\System32;C:\Windows", parent_str))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Some(format!("{}:/usr/local/bin:/usr/bin:/bin", parent_str))
    }
}

/// For agents that don't reliably inherit shell `$PATH` (see
/// `AgentAdapter::needs_path_injection`), resolve the entry's command to an
/// absolute path and inject `PATH` into env so scripts with `#!/usr/bin/env node`
/// shebangs can find sibling binaries.
///
/// Idempotent and non-destructive: existing `PATH` in env is preserved (or_insert),
/// so a user's manual override is never overwritten. To re-compute PATH (e.g. when
/// repairing dirty data), remove the existing key first then call this function.
pub fn ensure_path_injection(entry: &mut crate::adapter::McpServerEntry) {
    // Remote entries launch no subprocess — nothing to resolve or inject.
    if entry.transport != McpTransport::Stdio {
        return;
    }
    entry.command = resolve_command_path(&entry.command);
    if let Some(path_val) = build_path_for_command(&entry.command) {
        entry.env.entry("PATH".to_string()).or_insert(path_val);
    }
}

/// Top-level JSON key under which each JSON-based MCP format stores server
/// entries. The format → key mapping is the only thing that varies between
/// JSON-format agents in the remove/restore/read paths, so centralizing it
/// here keeps that knowledge in one place and forces explicit handling of
/// every JSON variant via the compiler-checked match.
///
/// Toml and Opencode are excluded — both formats route to dedicated
/// functions (`*_toml` / `*_opencode`) before this helper is reached.
/// Centralizing the format → key map this way forces every variant to be
/// considered when a new MCP-supporting agent is added.
fn json_top_key(format: McpFormat) -> &'static str {
    match format {
        McpFormat::McpServers => "mcpServers",
        McpFormat::Servers => "servers",
        McpFormat::Toml => unreachable!("Toml format uses a separate TOML code path"),
        McpFormat::Opencode => {
            unreachable!("Opencode format routes through dedicated CST helpers")
        }
        McpFormat::HermesYaml => {
            unreachable!("HermesYaml format routes through dedicated YAML helpers")
        }
        McpFormat::DshCordis => {
            unreachable!(
                "DshCordis never reaches the JSON writers — install/remove \
                 route through the dedicated cordis writers \
                 (deploy_mcp_server_dsh_cordis / remove_mcp_server_dsh_cordis); \
                 toggling uses the native patch-layer path (set_dsh_mcp_enabled)"
            )
        }
        McpFormat::GrokToml => {
            unreachable!("GrokToml format uses a separate TOML code path")
        }
        McpFormat::OpenClawJson5 => {
            unreachable!(
                "OpenClaw MCP routes through dedicated JSON5 CST helpers \
                 (deploy_mcp_server_openclaw / remove_mcp_server_openclaw / \
                 set_openclaw_mcp_enabled)"
            )
        }
    }
}

/// Deploy an MCP server config entry into the target agent's config file.
/// Format varies by agent — see `McpFormat`. Remote (HTTP/SSE) entries are
/// validated against the target's `RemoteMcpSchema` first: a target that
/// can't express the entry's transport gets a hard error instead of a
/// broken config (issue #105's failure mode was writing `command = ""`).
/// The UI prevents these combinations up front via `AgentCapabilities`;
/// this guard covers direct API callers.
pub fn deploy_mcp_server(
    config_path: &Path,
    entry: &McpServerEntry,
    adapter: &dyn crate::adapter::AgentAdapter,
) -> Result<(), HkError> {
    let remote_schema = adapter.remote_mcp_schema();
    if entry.transport != McpTransport::Stdio {
        validate_remote_mcp_target(entry, adapter.name(), remote_schema)?;
    }
    match adapter.mcp_format() {
        McpFormat::McpServers => {
            deploy_mcp_server_json(config_path, entry, "mcpServers", remote_schema)
        }
        McpFormat::Servers => deploy_mcp_server_json(config_path, entry, "servers", remote_schema),
        McpFormat::Toml => deploy_mcp_server_toml(config_path, entry),
        McpFormat::Opencode => deploy_mcp_server_opencode(config_path, entry),
        McpFormat::HermesYaml => deploy_mcp_server_hermes_yaml(config_path, entry),
        McpFormat::DshCordis => deploy_mcp_server_dsh_cordis(config_path, entry),
        McpFormat::GrokToml => deploy_mcp_server_grok_toml(config_path, entry),
        McpFormat::OpenClawJson5 => deploy_mcp_server_openclaw(config_path, entry, remote_schema),
    }
}

/// Refuse remote entries the target agent cannot load.
pub(super) fn validate_remote_mcp_target(
    entry: &McpServerEntry,
    agent_name: &str,
    schema: RemoteMcpSchema,
) -> Result<(), HkError> {
    if entry.url.is_none() {
        return Err(HkError::ConfigCorrupted(format!(
            "Remote MCP server '{}' has no url",
            entry.name
        )));
    }
    match schema {
        RemoteMcpSchema::Unsupported => Err(HkError::Validation(format!(
            "{agent_name} does not support remote (HTTP/SSE) MCP servers"
        ))),
        RemoteMcpSchema::Toml | RemoteMcpSchema::DshTransport
            if entry.transport == McpTransport::Sse =>
        {
            Err(HkError::Validation(format!(
                "{agent_name} supports Streamable HTTP MCP servers only, not SSE"
            )))
        }
        _ => Ok(()),
    }
}

/// The JSON object for one server entry, in the target agent's spelling.
pub(super) fn build_mcp_json_value(
    entry: &McpServerEntry,
    remote: RemoteMcpSchema,
) -> Result<serde_json::Value, HkError> {
    if entry.transport == McpTransport::Stdio {
        return Ok(serde_json::json!({
            "command": entry.command,
            "args": entry.args,
            "env": entry.env,
        }));
    }
    let url = entry.url.clone().unwrap_or_default();
    let mut obj = serde_json::Map::new();
    match remote {
        RemoteMcpSchema::TypeAndUrl => {
            let type_str = if entry.transport == McpTransport::Sse {
                "sse"
            } else {
                "http"
            };
            obj.insert("type".into(), type_str.into());
            obj.insert("url".into(), url.into());
        }
        RemoteMcpSchema::PlainUrl => {
            obj.insert("url".into(), url.into());
        }
        RemoteMcpSchema::GeminiSplit => {
            let key = if entry.transport == McpTransport::Sse {
                "url"
            } else {
                "httpUrl"
            };
            obj.insert(key.into(), url.into());
        }
        RemoteMcpSchema::ServerUrl => {
            obj.insert("serverUrl".into(), url.into());
        }
        // Non-JSON formats have their own writers; validation rejects
        // Unsupported before this point. Reaching here means an adapter's
        // mcp_format() and remote_mcp_schema() disagree — surface it as an
        // error instead of a panic.
        RemoteMcpSchema::Toml
        | RemoteMcpSchema::OpencodeRemote
        | RemoteMcpSchema::HermesUrl
        | RemoteMcpSchema::DshTransport
        | RemoteMcpSchema::GrokToml
        | RemoteMcpSchema::Unsupported => {
            return Err(HkError::Internal(format!(
                "remote JSON value requested for non-JSON schema {remote:?}"
            )));
        }
    }
    if !entry.headers.is_empty() {
        obj.insert(
            "headers".into(),
            serde_json::Value::Object(
                entry
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect(),
            ),
        );
    }
    Ok(serde_json::Value::Object(obj))
}

/// JSON-based MCP deploy (Claude, Gemini, Cursor, Antigravity, Copilot,
/// Windsurf, Kiro, omp). `top_key` is "mcpServers" or "servers" depending
/// on the agent; `remote` picks the agent's remote-entry spelling.
fn deploy_mcp_server_json(
    config_path: &Path,
    entry: &McpServerEntry,
    top_key: &str,
    remote: RemoteMcpSchema,
) -> Result<(), HkError> {
    locked_modify_json(config_path, |config| {
        let servers = config
            .as_object_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
            .entry(top_key)
            .or_insert_with(|| serde_json::json!({}));
        servers
            .as_object_mut()
            .ok_or_else(|| HkError::ConfigCorrupted(format!("{} is not an object", top_key)))?
            .insert(entry.name.clone(), build_mcp_json_value(entry, remote)?);
        Ok(())
    })
}

/// TOML-based MCP deploy (Codex: ~/.codex/config.toml with [mcp_servers.<name>]).
fn deploy_mcp_server_toml(config_path: &Path, entry: &McpServerEntry) -> Result<(), HkError> {
    // Build server entry table. Remote entries use url/http_headers
    // (Codex's Streamable HTTP schema); stdio entries use command/args/env.
    // Dispatch on transport (like the JSON writers); validation guarantees
    // remote entries carry a url by the time a writer runs.
    let mut server_table = toml::Table::new();
    if entry.transport != McpTransport::Stdio {
        let url = entry.url.clone().unwrap_or_default();
        server_table.insert("url".into(), toml::Value::String(url));
        if !entry.headers.is_empty() {
            let mut headers_table = toml::Table::new();
            for (k, v) in &entry.headers {
                headers_table.insert(k.clone(), toml::Value::String(v.clone()));
            }
            server_table.insert("http_headers".into(), toml::Value::Table(headers_table));
        }
    } else {
        server_table.insert("command".into(), toml::Value::String(entry.command.clone()));
        if !entry.args.is_empty() {
            server_table.insert(
                "args".into(),
                toml::Value::Array(
                    entry
                        .args
                        .iter()
                        .map(|a| toml::Value::String(a.clone()))
                        .collect(),
                ),
            );
        }
        if !entry.env.is_empty() {
            let mut env_table = toml::Table::new();
            for (k, v) in &entry.env {
                env_table.insert(k.clone(), toml::Value::String(v.clone()));
            }
            server_table.insert("env".into(), toml::Value::Table(env_table));
        }
    }

    upsert_mcp_server_toml(config_path, &entry.name, toml::Value::Table(server_table))
}

/// Insert/replace `[mcp_servers.<name>]` in a TOML config, preserving the
/// rest of the file. Shared by deploy (freshly built table) and restore
/// (snapshot transcoded wholesale).
///
/// Codex requires names to match ^[a-zA-Z0-9_-]+$; sanitize before inserting.
/// The original name is stored as `_hk_name` so the scanner can recover it
/// for consistent grouping with other agents that use the unsanitized name.
pub(super) fn upsert_mcp_server_toml(
    config_path: &Path,
    name: &str,
    mut server_val: toml::Value,
) -> Result<(), HkError> {
    let parent = config_path
        .parent()
        .ok_or_else(|| HkError::Validation("Invalid config path".into()))?;
    std::fs::create_dir_all(parent)?;

    // Read existing TOML or start fresh
    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Table = if existing.is_empty() {
        toml::Table::new()
    } else {
        existing
            .parse::<toml::Table>()
            .map_err(|e| HkError::ConfigCorrupted(format!("Failed to parse TOML config: {e}")))?
    };

    // Get or create [mcp_servers] table
    let mcp_servers = doc
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| HkError::ConfigCorrupted("mcp_servers is not a table".into()))?;

    let safe_name = sanitize_mcp_name(name);
    if safe_name != name {
        server_val
            .as_table_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("MCP server entry is not a table".into()))?
            .insert("_hk_name".into(), toml::Value::String(name.to_string()));
    }
    mcp_servers.insert(safe_name, server_val);

    // Write back atomically
    atomic_write(
        config_path,
        &toml::to_string_pretty(&doc).map_err(|e| HkError::Internal(e.to_string()))?,
    )?;

    Ok(())
}

/// Grok Build MCP deploy: `[mcp_servers.<name>]` with `headers` (not Codex
/// `http_headers`) and optional `type = "sse"`. Extra Grok keys on an
/// existing entry (`cwd`, timeouts, oauth, …) are kept. A newly written
/// server is removed from `disabled_mcp_servers` so it starts enabled.
fn deploy_mcp_server_grok_toml(config_path: &Path, entry: &McpServerEntry) -> Result<(), HkError> {
    let existing = read_toml_table(config_path)?;
    let safe_name = sanitize_mcp_name(&entry.name);
    let mut server_table = existing
        .get("mcp_servers")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get(&safe_name).or_else(|| t.get(&entry.name)))
        .and_then(|v| v.as_table())
        .cloned()
        .unwrap_or_default();

    if entry.transport != McpTransport::Stdio {
        let url = entry.url.clone().unwrap_or_default();
        server_table.insert("url".into(), toml::Value::String(url));
        // `urlTemplate`/`url_template` are deserialize-only aliases of `url`
        // upstream; leaving one beside the `url` we just wrote would be a
        // duplicate-field error that makes Grok drop the whole entry.
        server_table.remove("urlTemplate");
        server_table.remove("url_template");
        server_table.remove("command");
        server_table.remove("args");
        server_table.remove("env");
        if entry.transport == McpTransport::Sse {
            server_table.insert("type".into(), toml::Value::String("sse".into()));
        } else {
            server_table.remove("type");
        }
        if entry.headers.is_empty() {
            server_table.remove("headers");
        } else {
            let mut headers_table = toml::Table::new();
            for (k, v) in &entry.headers {
                headers_table.insert(k.clone(), toml::Value::String(v.clone()));
            }
            server_table.insert("headers".into(), toml::Value::Table(headers_table));
        }
    } else {
        server_table.remove("url");
        server_table.remove("urlTemplate");
        server_table.remove("url_template");
        server_table.remove("type");
        server_table.remove("headers");
        server_table.insert("command".into(), toml::Value::String(entry.command.clone()));
        if entry.args.is_empty() {
            server_table.remove("args");
        } else {
            server_table.insert(
                "args".into(),
                toml::Value::Array(
                    entry
                        .args
                        .iter()
                        .map(|a| toml::Value::String(a.clone()))
                        .collect(),
                ),
            );
        }
        if entry.env.is_empty() {
            server_table.remove("env");
        } else {
            let mut env_table = toml::Table::new();
            for (k, v) in &entry.env {
                env_table.insert(k.clone(), toml::Value::String(v.clone()));
            }
            server_table.insert("env".into(), toml::Value::Table(env_table));
        }
    }
    server_table.insert("enabled".into(), toml::Value::Boolean(true));
    upsert_mcp_server_toml(config_path, &entry.name, toml::Value::Table(server_table))?;
    modify_toml_table(config_path, |table| {
        remove_string_from_array(table, "disabled_mcp_servers", &entry.name);
        remove_string_from_array(table, "disabled_mcp_servers", &safe_name);
        Ok(())
    })
}

/// JSON-based MCP deploy for OpenCode (`~/.config/opencode/opencode.json[c]`).
/// Schema reference: https://opencode.ai/config.json (McpLocalConfig).
///
/// Differs from `mcpServers`/`servers` agents in four ways:
///   - top-level key is `"mcp"`
///   - `command` is a single array merging the binary + its args
///   - env block is named `"environment"` (not `"env"`)
///   - entry must declare `"type": "local"` (the schema also defines a
///     `"remote"` variant that HarnessKit does not deploy)
///
/// `additionalProperties: false` upstream means we must not emit any
/// extra fields (e.g. no separate `args`/`env`).
///
/// Goes through `locked_modify_jsonc` so existing user comments and
/// formatting in opencode.jsonc (or opencode.json — OpenCode's loader
/// runs both through jsonc-parser) survive a deploy. Replaces an
/// existing same-named entry in place rather than re-appending.
fn deploy_mcp_server_opencode(config_path: &Path, entry: &McpServerEntry) -> Result<(), HkError> {
    let value = build_opencode_mcp_value(entry);
    locked_modify_jsonc(config_path, |root| {
        let mcp = root.object_value_or_set("mcp");
        let cst_value = to_cst_input(&value);
        if let Some(existing) = mcp.get(&entry.name) {
            existing.set_value(cst_value);
        } else {
            mcp.append(&entry.name, cst_value);
        }
        Ok(())
    })
}

/// YAML-based MCP deploy for Hermes (`~/.hermes/config.yaml`, "mcp_servers" key).
///
/// Reads the full config.yaml, upserts the server entry under `mcp_servers.<name>`,
/// and writes the file back. Command-based entries use `command`/`args`/`env` keys;
/// URL-based entries (where `entry.command` starts with "http") use a `url` key.
/// The rest of config.yaml is preserved through serde_yaml round-trip.
fn deploy_mcp_server_hermes_yaml(
    config_path: &Path,
    entry: &McpServerEntry,
) -> Result<(), HkError> {
    modify_hermes_yaml(config_path, |root| {
        let servers = root
            .entry("mcp_servers".into())
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("mcp_servers is not a mapping".into()))?;
        let mut server = serde_yaml::Mapping::new();
        if entry.transport != crate::adapter::McpTransport::Stdio {
            // Remote: {url, headers?, transport: sse?} — Streamable HTTP
            // is Hermes' default, so only SSE needs the transport key.
            let url = entry.url.clone().unwrap_or_default();
            server.insert("url".into(), url.into());
            if entry.transport == crate::adapter::McpTransport::Sse {
                server.insert("transport".into(), "sse".into());
            }
            if !entry.headers.is_empty() {
                let mut headers = serde_yaml::Mapping::new();
                for (k, v) in &entry.headers {
                    headers.insert(k.clone().into(), v.clone().into());
                }
                server.insert("headers".into(), serde_yaml::Value::Mapping(headers));
            }
        } else {
            server.insert("command".into(), entry.command.clone().into());
            if !entry.args.is_empty() {
                let args: Vec<serde_yaml::Value> = entry
                    .args
                    .iter()
                    .cloned()
                    .map(serde_yaml::Value::String)
                    .collect();
                server.insert("args".into(), serde_yaml::Value::Sequence(args));
            }
            if !entry.env.is_empty() {
                let mut env = serde_yaml::Mapping::new();
                for (k, v) in &entry.env {
                    env.insert(k.clone().into(), v.clone().into());
                }
                server.insert("env".into(), serde_yaml::Value::Mapping(env));
            }
        }
        server.insert("enabled".into(), serde_yaml::Value::Bool(true));
        servers.insert(
            entry.name.clone().into(),
            serde_yaml::Value::Mapping(server),
        );
        Ok(())
    })
}

/// Load config.yaml as a mutable root mapping (empty mapping if absent/blank),
/// run `f`, then atomically write it back. The single primitive every Hermes
/// YAML writer (MCP, hooks, plugins) routes through.
///
/// Note: CREATES the file (and parent dirs) even on a no-op `f`; remove-style
/// callers that must not create an absent file should pre-check existence.
pub(super) fn modify_hermes_yaml(
    config_path: &Path,
    f: impl FnOnce(&mut serde_yaml::Mapping) -> Result<(), HkError>,
) -> Result<(), HkError> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: serde_yaml::Value = if existing.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(&existing).map_err(|e| {
            HkError::ConfigCorrupted(format!("Failed to parse Hermes config.yaml: {e}"))
        })?
    };
    let root = doc
        .as_mapping_mut()
        .ok_or_else(|| HkError::ConfigCorrupted("config.yaml root is not a mapping".into()))?;
    f(root)?;
    let output = serde_yaml::to_string(&doc).map_err(|e| HkError::Internal(e.to_string()))?;
    atomic_write(config_path, &output)?;
    Ok(())
}

/// Build the `serde_json::Value` shape OpenCode's `McpLocalConfig` schema
/// expects for one server entry. Shared by `deploy_mcp_server_opencode`
/// (cross-agent install path) and intentionally also reachable as the
/// "regenerate from McpServerEntry" reference. Schema invariants are
/// documented at the parent function — keep them in sync.
fn build_opencode_mcp_value(entry: &McpServerEntry) -> serde_json::Value {
    let mut server_obj = serde_json::Map::new();
    if entry.transport != McpTransport::Stdio {
        // McpRemoteConfig: {type: "remote", url, headers?}
        let url = entry.url.clone().unwrap_or_default();
        server_obj.insert("type".into(), serde_json::Value::String("remote".into()));
        server_obj.insert("url".into(), serde_json::Value::String(url));
        if !entry.headers.is_empty() {
            server_obj.insert(
                "headers".into(),
                serde_json::Value::Object(
                    entry
                        .headers
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect(),
                ),
            );
        }
    } else {
        let mut command_array = vec![serde_json::Value::String(entry.command.clone())];
        command_array.extend(entry.args.iter().cloned().map(serde_json::Value::String));
        server_obj.insert("type".into(), serde_json::Value::String("local".into()));
        server_obj.insert("command".into(), serde_json::Value::Array(command_array));
        if !entry.env.is_empty() {
            server_obj.insert(
                "environment".into(),
                serde_json::Value::Object(
                    entry
                        .env
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect(),
                ),
            );
        }
    }
    serde_json::Value::Object(server_obj)
}

/// Remove an MCP server entry from a config file by name.
pub fn remove_mcp_server(
    config_path: &Path,
    server_name: &str,
    format: McpFormat,
) -> Result<(), HkError> {
    if !config_path.exists() {
        return Ok(());
    }
    match format {
        McpFormat::Toml => {
            let content = std::fs::read_to_string(config_path)?;
            let mut doc: toml::Table = content
                .parse::<toml::Table>()
                .map_err(|e| HkError::ConfigCorrupted(e.to_string()))?;
            if let Some(servers) = doc.get_mut("mcp_servers").and_then(|v| v.as_table_mut()) {
                // Try original name first, then sanitized TOML key.
                if servers.remove(server_name).is_none() {
                    servers.remove(&sanitize_mcp_name(server_name));
                }
            }
            atomic_write(
                config_path,
                &toml::to_string_pretty(&doc).map_err(|e| HkError::Internal(e.to_string()))?,
            )?;
            Ok(())
        }
        McpFormat::Opencode => remove_mcp_server_opencode(config_path, server_name),
        McpFormat::HermesYaml => modify_hermes_yaml(config_path, |root| {
            if let Some(servers) = root.get_mut("mcp_servers").and_then(|v| v.as_mapping_mut()) {
                servers.remove(server_name);
            }
            Ok(())
        }),
        McpFormat::DshCordis => remove_mcp_server_dsh_cordis(config_path, server_name),
        McpFormat::GrokToml => remove_mcp_server_grok_toml(config_path, server_name),
        McpFormat::OpenClawJson5 => remove_mcp_server_openclaw(config_path, server_name),
        _ => locked_modify_json(config_path, |config| {
            let key = json_top_key(format);
            if let Some(servers) = config.get_mut(key).and_then(|v| v.as_object_mut()) {
                servers.remove(server_name);
            }
            Ok(())
        }),
    }
}

/// OpenClaw keeps MCP servers under the nested `mcp.servers` object of its
/// JSON5 config (docs.openclaw.ai/tools/mcp). Stdio entries use the shared
/// `{command, args, env}` spelling; remote ones never get here because the
/// adapter reports `RemoteMcpSchema::Unsupported`. Redeploying an existing
/// server rewrites those keys only, so a user's `enabled: false`, `cwd` and
/// other fields survive.
fn deploy_mcp_server_openclaw(
    config_path: &Path,
    entry: &McpServerEntry,
    remote: RemoteMcpSchema,
) -> Result<(), HkError> {
    let value = build_mcp_json_value(entry, remote)?;
    locked_modify_jsonc(config_path, |root| {
        let server = root
            .object_value_or_set("mcp")
            .object_value_or_set("servers")
            .object_value_or_set(&entry.name);
        // A same-named remote entry becomes stdio: drop the URL-only keys,
        // which OpenClaw rejects next to a command (e.g. per-requester OAuth).
        for key in ["url", "transport", "type", "headers", "auth", "oauth"] {
            if let Some(prop) = server.get(key) {
                prop.remove();
            }
        }
        for (key, field) in value.as_object().into_iter().flatten() {
            let field = to_cst_input(field);
            if let Some(prop) = server.get(key) {
                prop.set_value(field);
            } else {
                server.append(key, field);
            }
        }
        Ok(())
    })
}

fn remove_mcp_server_openclaw(config_path: &Path, server_name: &str) -> Result<(), HkError> {
    locked_modify_jsonc(config_path, |root| {
        if let Some(mcp) = root.object_value("mcp")
            && let Some(servers) = mcp.object_value("servers")
            && let Some(prop) = servers.get(server_name)
        {
            prop.remove();
        }
        Ok(())
    })
}

/// Native per-server toggle: flip the entry's `enabled` field in place under
/// `mcp.servers`, keeping every other key (secrets included) byte-identical
/// apart from that one value. Mirrors OpenClaw's own enable/disable; the
/// state is read back by `OpenClawAdapter::read_mcp_servers` on rescan.
/// Docs: https://docs.openclaw.ai/tools/mcp
pub fn set_openclaw_mcp_enabled(
    config_path: &Path,
    server_name: &str,
    enabled: bool,
) -> Result<(), HkError> {
    // The CST helper creates the file it opens, and an empty `openclaw.json`
    // is a config the gateway refuses to load — so check first.
    if !config_path.exists() {
        return Err(HkError::NotFound(format!(
            "OpenClaw config not found: {}",
            config_path.display()
        )));
    }
    locked_modify_jsonc(config_path, |root| {
        let entry_obj = root
            .object_value("mcp")
            .and_then(|mcp| mcp.object_value("servers"))
            .and_then(|servers| servers.object_value(server_name))
            .ok_or_else(|| {
                HkError::NotFound(format!("MCP server '{server_name}' not found in config"))
            })?;
        let value = jsonc_parser::cst::CstInputValue::Bool(enabled);
        if let Some(prop) = entry_obj.get("enabled") {
            prop.set_value(value);
        } else {
            entry_obj.append("enabled", value);
        }
        Ok(())
    })
}

fn remove_mcp_server_grok_toml(config_path: &Path, server_name: &str) -> Result<(), HkError> {
    modify_toml_table(config_path, |table| {
        let safe = sanitize_mcp_name(server_name);
        if let Some(servers) = table.get_mut("mcp_servers").and_then(|v| v.as_table_mut()) {
            if servers.remove(server_name).is_none() {
                servers.remove(&safe);
            }
            if servers.is_empty() {
                table.remove("mcp_servers");
            }
        }
        remove_string_from_array(table, "disabled_mcp_servers", server_name);
        remove_string_from_array(table, "disabled_mcp_servers", &safe);
        if let Some(tools) = table
            .get_mut("disabled_mcp_tools")
            .and_then(|v| v.as_table_mut())
        {
            tools.remove(server_name);
            tools.remove(&safe);
            if tools.is_empty() {
                table.remove("disabled_mcp_tools");
            }
        }
        Ok(())
    })
}

/// Remove `server_name` from OpenCode's `mcp` block while preserving the
/// rest of the file verbatim (comments, formatting, sibling entries).
/// No-op if the server isn't present. Per the design decision in this PR,
/// any leading user-comments next to the removed entry stay in place — HK
/// never edits user comment text, only its own data entries.
fn remove_mcp_server_opencode(config_path: &Path, server_name: &str) -> Result<(), HkError> {
    locked_modify_jsonc(config_path, |root| {
        if let Some(mcp) = root.object_value("mcp")
            && let Some(prop) = mcp.get(server_name)
        {
            prop.remove();
        }
        Ok(())
    })
}

/// Restore a previously disabled MCP server entry into the config file.
pub fn restore_mcp_server(
    config_path: &Path,
    server_name: &str,
    entry: &serde_json::Value,
    format: McpFormat,
) -> Result<(), HkError> {
    match format {
        McpFormat::Toml => {
            // Transcode the saved JSON snapshot back to TOML wholesale. The
            // snapshot is the raw on-disk table (read_mcp_server_config), so a
            // generic conversion preserves every key — url, http_headers, and
            // anything Codex adds later — where a field-by-field copy through
            // McpServerEntry silently dropped unknown ones.
            let toml_val: toml::Value = serde_json::from_value(entry.clone())
                .map_err(|e| HkError::ConfigCorrupted(format!("saved MCP snapshot: {e}")))?;
            upsert_mcp_server_toml(config_path, server_name, toml_val)
        }
        McpFormat::Opencode => restore_mcp_server_opencode(config_path, server_name, entry),
        McpFormat::HermesYaml => unreachable!(
            "Hermes MCP uses native in-place enable/disable (set_hermes_mcp_enabled); \
             the remove+snapshot+restore path is never reached for Hermes"
        ),
        McpFormat::DshCordis => unreachable!(
            "dsh MCP uses native in-place enable/disable (set_dsh_mcp_enabled); \
             the remove+snapshot+restore path is never reached for dsh"
        ),
        McpFormat::GrokToml => unreachable!(
            "Grok MCP uses native in-place enable/disable (set_grok_mcp_enabled); \
             the remove+snapshot+restore path is never reached for grok"
        ),
        McpFormat::OpenClawJson5 => unreachable!(
            "OpenClaw MCP uses native in-place enable/disable (set_openclaw_mcp_enabled); \
             the remove+snapshot+restore path is never reached for openclaw"
        ),
        _ => {
            let key = json_top_key(format);
            locked_modify_json(config_path, |config| {
                let servers = config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry(key)
                    .or_insert_with(|| serde_json::json!({}));
                servers
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted(format!("{key} is not an object")))?
                    .insert(server_name.to_string(), entry.clone());
                Ok(())
            })
        }
    }
}

/// Restore a previously-saved entry into OpenCode's `mcp` block while
/// preserving the rest of the file verbatim. Creates the `mcp` block if
/// absent. Replaces an existing entry with the same name in place (rare
/// but possible if the user re-enables an entry that's also been
/// re-installed by another path).
fn restore_mcp_server_opencode(
    config_path: &Path,
    server_name: &str,
    entry: &serde_json::Value,
) -> Result<(), HkError> {
    locked_modify_jsonc(config_path, |root| {
        let mcp = root.object_value_or_set("mcp");
        let cst_value = to_cst_input(entry);
        if let Some(existing) = mcp.get(server_name) {
            existing.set_value(cst_value);
        } else {
            mcp.append(server_name, cst_value);
        }
        Ok(())
    })
}

/// Read an MCP server entry's full JSON value from a config file.
pub fn read_mcp_server_config(
    config_path: &Path,
    server_name: &str,
    format: McpFormat,
) -> Result<Option<serde_json::Value>, HkError> {
    if !config_path.exists() {
        return Ok(None);
    }
    match format {
        McpFormat::Toml => {
            let content = std::fs::read_to_string(config_path)?;
            let doc: toml::Table = content
                .parse::<toml::Table>()
                .map_err(|e| HkError::ConfigCorrupted(e.to_string()))?;
            // Try the original name first, then the sanitized TOML key.
            // The scanner uses `_hk_name` to recover the original name, so
            // callers pass the original while the TOML key is sanitized.
            let safe_name = sanitize_mcp_name(server_name);
            let server = doc
                .get("mcp_servers")
                .and_then(|v| v.as_table())
                .and_then(|t| t.get(server_name).or_else(|| t.get(&safe_name)));
            // Convert TOML value to JSON for uniform storage in DB
            match server {
                Some(val) => {
                    let json_str = serde_json::to_string(&val)?;
                    let json_val: serde_json::Value = serde_json::from_str(&json_str)?;
                    Ok(Some(json_val))
                }
                None => Ok(None),
            }
        }
        McpFormat::Opencode => read_mcp_server_config_opencode(config_path, server_name),
        McpFormat::HermesYaml => unreachable!(
            "Hermes MCP uses native in-place enable/disable (set_hermes_mcp_enabled); \
             the read-config-for-snapshot path is never reached for Hermes"
        ),
        McpFormat::DshCordis => unreachable!(
            "dsh MCP uses native in-place enable/disable (set_dsh_mcp_enabled); \
             the read-config-for-snapshot path is never reached for dsh"
        ),
        McpFormat::GrokToml => unreachable!(
            "Grok MCP uses native in-place enable/disable (set_grok_mcp_enabled); \
             the read-config-for-snapshot path is never reached for grok"
        ),
        McpFormat::OpenClawJson5 => unreachable!(
            "OpenClaw MCP uses native in-place enable/disable (set_openclaw_mcp_enabled); \
             the read-config-for-snapshot path is never reached for openclaw"
        ),
        _ => {
            let config = read_or_create_json(config_path)?;
            let key = json_top_key(format);
            Ok(config.get(key).and_then(|v| v.get(server_name)).cloned())
        }
    }
}

/// Read a single OpenCode MCP entry's value as `serde_json::Value`. Tolerant
/// of jsonc syntax (`//` comments, trailing commas) since OpenCode's loader
/// accepts the same superset for both `opencode.json` and `opencode.jsonc`.
/// Returns `None` if the file lacks `mcp` or that specific entry. Read-only,
/// no advisory lock — locks would only matter if we were modifying.
fn read_mcp_server_config_opencode(
    config_path: &Path,
    server_name: &str,
) -> Result<Option<serde_json::Value>, HkError> {
    use jsonc_parser::cst::CstRootNode;
    let content = std::fs::read_to_string(config_path)?;
    if content.is_empty() {
        return Ok(None);
    }
    let cst = CstRootNode::parse(&content, &Default::default())
        .map_err(|e| HkError::ConfigCorrupted(format!("Failed to parse jsonc: {e}")))?;
    let Some(root) = cst.object_value() else {
        return Ok(None);
    };
    let Some(prop) = root
        .object_value("mcp")
        .and_then(|mcp| mcp.get(server_name))
    else {
        return Ok(None);
    };
    Ok(prop.to_serde_value())
}


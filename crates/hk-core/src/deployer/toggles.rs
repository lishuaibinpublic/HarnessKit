//! Per-agent enable/disable toggles for MCP servers, hooks and plugins.

use super::*;
use crate::deployer::fs_util::*;
use crate::deployer::mcp::*;
use crate::deployer::toml_util::*;

/// Add/remove a plugin name under `plugins.enabled` in Hermes config.yaml.
/// Hermes plugins are disabled by default; presence in the list = enabled.
pub fn set_hermes_plugin_enabled(
    config_path: &Path,
    name: &str,
    enabled: bool,
) -> Result<(), HkError> {
    modify_hermes_yaml(config_path, |root| {
        let plugins = root
            .entry("plugins".into())
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("plugins is not a mapping".into()))?;
        let list = plugins
            .entry("enabled".into())
            .or_insert_with(|| serde_yaml::Value::Sequence(vec![]))
            .as_sequence_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("plugins.enabled is not a sequence".into()))?;
        let present = list.iter().any(|v| v.as_str() == Some(name));
        if enabled && !present {
            list.push(serde_yaml::Value::String(name.to_string()));
        } else if !enabled && present {
            list.retain(|v| v.as_str() != Some(name));
        }
        Ok(())
    })
}

/// Flip a Hermes MCP server's native `enabled` field IN PLACE (true/false),
/// leaving the rest of the entry (command/args/env/tools/…) untouched. This is
/// the in-place "disable" Hermes itself uses: the config stays put and only
/// `enabled` toggles — unlike HarnessKit's generic MCP disable, it never removes
/// the entry from the config or snapshots it in the DB.
///
/// Hermes supports a per-server `enabled: bool` (default `true`). A server with
/// `enabled: false` is skipped entirely — no connection, discovery, or tool
/// registration — while its config is retained for later reuse.
///   Docs:   https://hermes-agent.nousresearch.com/docs/reference/mcp-config-reference
///   Source: https://github.com/NousResearch/hermes-agent/blob/main/hermes_cli/mcp_config.py
pub fn set_hermes_mcp_enabled(
    config_path: &Path,
    name: &str,
    enabled: bool,
) -> Result<(), HkError> {
    modify_hermes_yaml(config_path, |root| {
        let servers = root
            .get_mut("mcp_servers")
            .and_then(|v| v.as_mapping_mut())
            .ok_or_else(|| HkError::ConfigCorrupted("mcp_servers is not a mapping".into()))?;
        let server = servers
            .get_mut(name)
            .and_then(|v| v.as_mapping_mut())
            .ok_or_else(|| HkError::NotFound(format!("MCP server '{name}' not found in config")))?;
        server.insert("enabled".into(), serde_yaml::Value::Bool(enabled));
        Ok(())
    })
}

fn apply_grok_user_mcp_enabled(table: &mut toml::Table, server_name: &str, enabled: bool) {
    let safe = sanitize_mcp_name(server_name);
    let mut disabled = toml_string_array(table, "disabled_mcp_servers");
    if enabled {
        disabled.retain(|n| n != server_name && n != &safe);
    } else if !disabled.iter().any(|n| n == server_name || n == &safe) {
        disabled.push(server_name.to_string());
    }
    set_string_array(table, "disabled_mcp_servers", &disabled);

    if let Some(servers) = table.get_mut("mcp_servers").and_then(|v| v.as_table_mut()) {
        let key = if servers.contains_key(server_name) {
            server_name.to_string()
        } else {
            safe
        };
        if let Some(entry) = servers.get_mut(&key).and_then(|v| v.as_table_mut()) {
            entry.insert("enabled".into(), toml::Value::Boolean(enabled));
        }
    }
}

/// Grok personal MCP toggle. Disable writes user `disabled_mcp_servers` and
/// `enabled = false` on a user entry if present — never the project file.
/// Enable clears the user list and unsticks a winning project `enabled = false`.
pub fn set_grok_mcp_enabled(
    user_config: &Path,
    project_config: Option<&Path>,
    server_name: &str,
    enabled: bool,
) -> Result<(), HkError> {
    modify_toml_table(user_config, |table| {
        apply_grok_user_mcp_enabled(table, server_name, enabled);
        Ok(())
    })?;
    if enabled && let Some(project) = project_config.filter(|p| p.exists() && *p != user_config) {
        unstick_grok_project_mcp(project, server_name)?;
    }
    Ok(())
}

fn unstick_grok_project_mcp(path: &Path, server_name: &str) -> Result<(), HkError> {
    let doc = read_toml_table(path)?;
    let safe = sanitize_mcp_name(server_name);
    let sticky = doc
        .get("mcp_servers")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get(server_name).or_else(|| t.get(&safe)))
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("enabled"))
        .and_then(|v| v.as_bool())
        == Some(false);
    if !sticky {
        return Ok(());
    }
    modify_toml_table(path, |table| {
        if let Some(servers) = table.get_mut("mcp_servers").and_then(|v| v.as_table_mut()) {
            let key = if servers.contains_key(server_name) {
                server_name.to_string()
            } else {
                safe
            };
            if let Some(entry) = servers.get_mut(&key).and_then(|v| v.as_table_mut()) {
                entry.insert("enabled".into(), toml::Value::Boolean(true));
            }
        }
        Ok(())
    })
}

/// Toggle a Grok hook via `$GROK_HOME/disabled-hooks` using the real spec.name.
/// Comments and unrelated lines are preserved.
pub fn set_grok_hook_enabled(
    disabled_hooks_path: &Path,
    spec_name: &str,
    enabled: bool,
) -> Result<(), HkError> {
    if enabled {
        if !disabled_hooks_path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(disabled_hooks_path)?;
        let mut found = false;
        let kept: Vec<&str> = content
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed == spec_name {
                    found = true;
                    false
                } else {
                    true
                }
            })
            .collect();
        if !found {
            return Ok(());
        }
        if let Some(parent) = disabled_hooks_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = kept.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        atomic_write(disabled_hooks_path, &out)
    } else {
        let existing = crate::adapter::grok::read_disabled_hook_names(disabled_hooks_path);
        if existing.contains(spec_name) {
            return Ok(());
        }
        if let Some(parent) = disabled_hooks_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut content = std::fs::read_to_string(disabled_hooks_path).unwrap_or_default();
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(spec_name);
        content.push('\n');
        atomic_write(disabled_hooks_path, &content)
    }
}

fn grok_plugin_table(root: &mut toml::Table) -> Result<&mut toml::Table, HkError> {
    let plugins = root
        .entry("plugins")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| HkError::ConfigCorrupted("[plugins] is not a table".into()))?;
    Ok(plugins)
}

/// Enable/disable a Grok plugin by stable id in `[plugins].enabled` /
/// `[plugins].disabled`. Disabled wins on conflict; enable adds to enabled
/// and removes from disabled.
pub fn set_grok_plugin_enabled(
    config_path: &Path,
    plugin_id: &str,
    enabled: bool,
) -> Result<(), HkError> {
    modify_toml_table(config_path, |table| {
        let plugins = grok_plugin_table(table)?;
        let mut enabled_list = toml_string_array(plugins, "enabled");
        let mut disabled_list = toml_string_array(plugins, "disabled");
        if enabled {
            disabled_list.retain(|v| v != plugin_id);
            if !enabled_list.iter().any(|v| v == plugin_id) {
                enabled_list.push(plugin_id.to_string());
            }
        } else {
            enabled_list.retain(|v| v != plugin_id);
            if !disabled_list.iter().any(|v| v == plugin_id) {
                disabled_list.push(plugin_id.to_string());
            }
        }
        set_string_array(plugins, "enabled", &enabled_list);
        set_string_array(plugins, "disabled", &disabled_list);
        if plugins.is_empty() {
            table.remove("plugins");
        }
        Ok(())
    })
}

/// Drop a plugin id (and optional display name) from both Grok plugin lists
/// after the directory is deleted.
pub fn remove_grok_plugin_lists(
    config_path: &Path,
    plugin_id: &str,
    plugin_name: Option<&str>,
) -> Result<(), HkError> {
    // Read once, then bail before writing unless a list actually referenced
    // the plugin: rewriting discards comments and key order, which must never
    // happen to an untouched shared project config just because a plugin
    // elsewhere was deleted. (A missing file reads as an empty table and
    // returns right here.)
    let mut doc = read_toml_table(config_path)?;
    let keep = |v: &String| v != plugin_id && plugin_name.is_none_or(|n| v != n);
    let Some(plugins) = doc.get_mut("plugins").and_then(|v| v.as_table_mut()) else {
        return Ok(());
    };
    let references_plugin = ["enabled", "disabled"]
        .iter()
        .any(|key| toml_string_array(plugins, key).iter().any(|v| !keep(v)));
    if !references_plugin {
        return Ok(());
    }
    for key in ["enabled", "disabled"] {
        let mut list = toml_string_array(plugins, key);
        list.retain(keep);
        set_string_array(plugins, key, &list);
    }
    if plugins.is_empty() {
        doc.remove("plugins");
    }
    atomic_write(
        config_path,
        &toml::to_string_pretty(&doc).map_err(|e| HkError::Internal(e.to_string()))?,
    )
}

/// Flip a Kiro MCP server's native `disabled` flag in place.
pub fn set_kiro_mcp_enabled(
    config_path: &Path,
    server_name: &str,
    enabled: bool,
) -> Result<(), HkError> {
    locked_modify_json(config_path, |config| {
        let servers = config
            .get_mut("mcpServers")
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| HkError::NotFound("No mcpServers block found".into()))?;
        let server = servers
            .get_mut(server_name)
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| HkError::NotFound(format!("MCP server '{server_name}' not found")))?;
        if enabled {
            server.remove("disabled");
        } else {
            server.insert("disabled".into(), serde_json::Value::Bool(true));
        }
        Ok(())
    })
}

/// Flip an omp MCP server's native per-entry `enabled` flag in place, then
/// scrub the user-level name list that would override the flag: on disable
/// the name is removed from `enabledServers` (the force-enable allowlist
/// overrides `enabled: false`), on enable from `disabledServers` (the
/// denylist overrides everything). Both lists live only in the *user*
/// mcp.json but gate servers from every source (omp mcp/config.ts), so
/// `user_config_path` differs from `entry_config_path` for project-scoped
/// servers.
///
/// The entry flag — not the denylist — carries the toggle so it stays scoped
/// to this one entry: `disabledServers` matches by NAME across all sources
/// and would knock out same-named servers in other projects.
pub fn set_omp_mcp_enabled(
    entry_config_path: &Path,
    user_config_path: &Path,
    server_name: &str,
    enabled: bool,
) -> Result<(), HkError> {
    locked_modify_json(entry_config_path, |config| {
        let servers = config
            .get_mut("mcpServers")
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| HkError::NotFound("No mcpServers block found".into()))?;
        let server = servers
            .get_mut(server_name)
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| HkError::NotFound(format!("MCP server '{server_name}' not found")))?;
        if enabled {
            // Absent means enabled (mcp-config.md: "skip when false").
            server.remove("enabled");
        } else {
            server.insert("enabled".into(), serde_json::Value::Bool(false));
        }
        Ok(())
    })?;
    // A missing user file can't contain the name — don't create one just to
    // scrub it.
    if !user_config_path.exists() {
        return Ok(());
    }
    let list_key = if enabled { "disabledServers" } else { "enabledServers" };
    locked_modify_json(user_config_path, |config| {
        if let Some(list) = config.get_mut(list_key).and_then(|v| v.as_array_mut()) {
            list.retain(|v| v.as_str() != Some(server_name));
        }
        Ok(())
    })
}

/// Set enabledPlugins[plugin_key] to true or false (Claude native toggle).
pub fn set_plugin_enabled(
    config_path: &Path,
    plugin_key: &str,
    enabled: bool,
) -> Result<(), HkError> {
    locked_modify_json(config_path, |config| {
        let plugins = config
            .as_object_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
            .entry("enabledPlugins")
            .or_insert_with(|| serde_json::json!({}));
        plugins
            .as_object_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("enabledPlugins is not an object".into()))?
            .insert(plugin_key.to_string(), serde_json::Value::Bool(enabled));
        Ok(())
    })
}

/// Set [plugins."plugin_key"] enabled = true/false in Codex config.toml.
/// Uses file locking to prevent concurrent read-modify-write races.
pub fn set_codex_plugin_enabled(
    config_path: &Path,
    plugin_key: &str,
    enabled: bool,
) -> Result<(), HkError> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(config_path)?;
    file.lock_exclusive()?;

    let mut content = String::new();
    (&file).read_to_string(&mut content)?;
    let mut doc: toml::Table = if content.is_empty() {
        toml::Table::new()
    } else {
        content
            .parse::<toml::Table>()
            .map_err(|e| HkError::ConfigCorrupted(e.to_string()))?
    };
    let plugins = doc
        .entry("plugins")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| HkError::ConfigCorrupted("plugins is not a table".into()))?;
    let entry = plugins
        .entry(plugin_key)
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| HkError::ConfigCorrupted("plugin entry is not a table".into()))?;
    entry.insert("enabled".into(), toml::Value::Boolean(enabled));

    let output = toml::to_string_pretty(&doc).map_err(|e| HkError::Internal(e.to_string()))?;
    (&file).seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    (&file).write_all(output.as_bytes())?;
    (&file).flush()?;

    file.unlock()?;
    Ok(())
}

/// Remove a [plugins."plugin_key"] entry from Codex config.toml.
pub fn remove_codex_plugin_entry(config_path: &Path, plugin_key: &str) -> Result<(), HkError> {
    if !config_path.exists() {
        return Ok(());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(config_path)?;
    file.lock_exclusive()?;

    let mut content = String::new();
    (&file).read_to_string(&mut content)?;
    let mut doc: toml::Table = content
        .parse::<toml::Table>()
        .map_err(|e| HkError::ConfigCorrupted(e.to_string()))?;

    if let Some(plugins) = doc.get_mut("plugins").and_then(|v| v.as_table_mut()) {
        plugins.remove(plugin_key);
    }

    let output = toml::to_string_pretty(&doc).map_err(|e| HkError::Internal(e.to_string()))?;
    (&file).seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    (&file).write_all(output.as_bytes())?;
    (&file).flush()?;

    file.unlock()?;
    Ok(())
}

/// Set VS Code agent plugin enablement in state.vscdb.
/// Reads the current `agentPlugins.enablement` array, updates the entry for the
/// given plugin URI, and writes it back. Creates the entry if it doesn't exist.
pub fn set_vscode_plugin_enabled(
    vscode_user_dir: &Path,
    plugin_uri: &str,
    enabled: bool,
) -> Result<(), HkError> {
    let db_path = vscode_user_dir.join("globalStorage").join("state.vscdb");
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| HkError::Internal(format!("Failed to open VS Code state DB: {}", e)))?;

    // Read current enablement array
    let current: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'agentPlugins.enablement'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "[]".to_string());

    let mut entries: Vec<(String, bool)> = serde_json::from_str(&current).unwrap_or_default();

    // Update or insert the entry
    let mut found = false;
    for entry in &mut entries {
        if entry.0 == plugin_uri {
            entry.1 = enabled;
            found = true;
            break;
        }
    }
    if !found {
        entries.push((plugin_uri.to_string(), enabled));
    }

    let new_value =
        serde_json::to_string(&entries).map_err(|e| HkError::Internal(e.to_string()))?;

    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES ('agentPlugins.enablement', ?1)
         ON CONFLICT(key) DO UPDATE SET value = ?1",
        rusqlite::params![new_value],
    )
    .map_err(|e| HkError::Internal(format!("Failed to update VS Code state DB: {}", e)))?;

    Ok(())
}

/// Remove a plugin entry from VS Code's state.vscdb enablement array.
pub fn remove_vscode_plugin_entry(vscode_user_dir: &Path, plugin_uri: &str) -> Result<(), HkError> {
    let db_path = vscode_user_dir.join("globalStorage").join("state.vscdb");
    if !db_path.exists() {
        return Ok(());
    }
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| HkError::Internal(format!("Failed to open VS Code state DB: {}", e)))?;

    let current: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'agentPlugins.enablement'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "[]".to_string());

    let mut entries: Vec<(String, bool)> = serde_json::from_str(&current).unwrap_or_default();

    entries.retain(|e| e.0 != plugin_uri);

    let new_value =
        serde_json::to_string(&entries).map_err(|e| HkError::Internal(e.to_string()))?;

    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES ('agentPlugins.enablement', ?1)
         ON CONFLICT(key) DO UPDATE SET value = ?1",
        rusqlite::params![new_value],
    )
    .map_err(|e| HkError::Internal(format!("Failed to update VS Code state DB: {}", e)))?;

    Ok(())
}

/// Set Gemini extension enablement in extension-enablement.json.
/// Updates only the user-scope rule (`{homedir}/*`) and preserves workspace-scope rules.
pub fn set_gemini_extension_enabled(
    extensions_dir: &Path,
    extension_name: &str,
    enabled: bool,
    home: &Path,
) -> Result<(), HkError> {
    let home_str = home.to_string_lossy();
    let enable_rule = format!("{}/*", home_str);
    let disable_rule = format!("!{}/*", home_str);

    modify_gemini_enablement(extensions_dir, |config| {
        let entry = config
            .entry(extension_name.to_string())
            .or_insert_with(|| serde_json::json!({"overrides": []}));
        let overrides = entry
            .get_mut("overrides")
            .and_then(|v| v.as_array_mut())
            .ok_or_else(|| HkError::ConfigCorrupted("overrides is not an array".into()))?;

        // Remove existing user-scope rules (both enable and disable)
        overrides.retain(|v| {
            let s = v.as_str().unwrap_or("");
            s != enable_rule && s != disable_rule
        });

        // Add the new rule
        let rule = if enabled { &enable_rule } else { &disable_rule };
        overrides.push(serde_json::Value::String(rule.to_string()));
        Ok(())
    })
}

/// Remove an extension entry from Gemini's extension-enablement.json.
pub fn remove_gemini_extension_entry(
    extensions_dir: &Path,
    extension_name: &str,
) -> Result<(), HkError> {
    let enablement_path = extensions_dir.join("extension-enablement.json");
    if !enablement_path.exists() {
        return Ok(());
    }
    modify_gemini_enablement(extensions_dir, |config| {
        config.remove(extension_name);
        Ok(())
    })
}

/// Locked read-modify-write for extension-enablement.json.
fn modify_gemini_enablement(
    extensions_dir: &Path,
    modify: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> Result<(), HkError>,
) -> Result<(), HkError> {
    let enablement_path = extensions_dir.join("extension-enablement.json");
    if let Some(parent) = enablement_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&enablement_path)?;
    file.lock_exclusive()?;

    let mut content = String::new();
    (&file).read_to_string(&mut content)?;
    let mut config: serde_json::Map<String, serde_json::Value> = if content.is_empty() {
        serde_json::Map::new()
    } else {
        serde_json::from_str(&content)
            .map_err(|e| HkError::ConfigCorrupted(format!("extension-enablement.json: {}", e)))?
    };

    modify(&mut config)?;

    let output =
        serde_json::to_string_pretty(&config).map_err(|e| HkError::Internal(e.to_string()))?;
    (&file).seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    (&file).write_all(output.as_bytes())?;
    (&file).flush()?;

    file.unlock()?;
    Ok(())
}

/// Remove a plugin entry from a config file's enabledPlugins object by key.
pub fn remove_plugin_entry(config_path: &Path, plugin_key: &str) -> Result<(), HkError> {
    if !config_path.exists() {
        return Ok(());
    }
    locked_modify_json(config_path, |config| {
        if let Some(plugins) = config
            .get_mut("enabledPlugins")
            .and_then(|v| v.as_object_mut())
        {
            plugins.remove(plugin_key);
        }
        Ok(())
    })
}

/// Restore a previously disabled plugin entry into enabledPlugins.
pub fn restore_plugin_entry(
    config_path: &Path,
    plugin_key: &str,
    value: &serde_json::Value,
) -> Result<(), HkError> {
    locked_modify_json(config_path, |config| {
        let plugins = config
            .as_object_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
            .entry("enabledPlugins")
            .or_insert_with(|| serde_json::json!({}));
        plugins
            .as_object_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("enabledPlugins is not an object".into()))?
            .insert(plugin_key.to_string(), value.clone());
        Ok(())
    })
}

/// Read a plugin entry's value from enabledPlugins in a config file.
pub fn read_plugin_config(
    config_path: &Path,
    plugin_key: &str,
) -> Result<Option<serde_json::Value>, HkError> {
    if !config_path.exists() {
        return Ok(None);
    }
    let config = read_or_create_json(config_path)?;
    Ok(config
        .get("enabledPlugins")
        .and_then(|v| v.get(plugin_key))
        .cloned())
}


//! Hook deployment, removal, restore and reads (all agent formats).

use super::*;
use crate::deployer::fs_util::*;
use crate::deployer::mcp::*;

/// Deploy a hook config entry into the target agent's config file.
/// Reads the existing JSON, appends the hook under "hooks" -> event, writes back.
pub fn deploy_hook(
    config_path: &Path,
    entry: &HookEntry,
    format: HookFormat,
) -> Result<(), HkError> {
    if format == HookFormat::HermesYaml {
        return deploy_hook_hermes_yaml(config_path, entry);
    }
    locked_modify_json(config_path, |config| {
        match format {
            HookFormat::ClaudeLike => {
                let hooks = config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry("hooks")
                    .or_insert_with(|| serde_json::json!({}));
                let event_arr = hooks
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hooks is not an object".into()))?
                    .entry(&entry.event)
                    .or_insert_with(|| serde_json::json!([]));
                let arr = event_arr
                    .as_array_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hook event is not an array".into()))?;

                let matcher_val = entry.matcher.as_deref().map(serde_json::Value::from);
                let group = arr.iter_mut().find(|h| {
                    h.get("matcher").and_then(|v| v.as_str()).map(String::from) == entry.matcher
                });
                // Use object format {"type":"command","command":"..."} — accepted by Claude, required by Codex/Gemini
                let cmd_obj = serde_json::json!({ "type": "command", "command": entry.command });
                if let Some(group) = group {
                    let cmds = group.as_object_mut().and_then(|o| {
                        o.entry("hooks")
                            .or_insert_with(|| serde_json::json!([]))
                            .as_array_mut()
                    });
                    if let Some(cmds) = cmds
                        && !cmds.iter().any(|c| {
                            c.get("command").and_then(|v| v.as_str()) == Some(&entry.command)
                        })
                    {
                        cmds.push(cmd_obj);
                    }
                } else {
                    let mut group = serde_json::json!({ "hooks": [cmd_obj] });
                    if let Some(m) = &matcher_val {
                        group
                            .as_object_mut()
                            .unwrap()
                            .insert("matcher".into(), m.clone());
                    }
                    arr.push(group);
                }
            }
            HookFormat::Cursor => {
                config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry("version")
                    .or_insert(serde_json::json!(1));
                let hooks = config
                    .as_object_mut()
                    .unwrap()
                    .entry("hooks")
                    .or_insert_with(|| serde_json::json!({}));
                let event_arr = hooks
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hooks is not an object".into()))?
                    .entry(&entry.event)
                    .or_insert_with(|| serde_json::json!([]));
                let arr = event_arr
                    .as_array_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("event is not an array".into()))?;
                let hook_val = serde_json::json!({ "command": entry.command });
                if !arr.contains(&hook_val) {
                    arr.push(hook_val);
                }
            }
            HookFormat::Windsurf => {
                let hooks = config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry("hooks")
                    .or_insert_with(|| serde_json::json!({}));
                let event_arr = hooks
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hooks is not an object".into()))?
                    .entry(&entry.event)
                    .or_insert_with(|| serde_json::json!([]));
                let arr = event_arr
                    .as_array_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("event is not an array".into()))?;
                let hook_val = serde_json::json!({ "command": entry.command });
                if !arr.contains(&hook_val) {
                    arr.push(hook_val);
                }
            }
            HookFormat::Copilot => {
                config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry("version")
                    .or_insert(serde_json::json!(1));
                let hooks = config
                    .as_object_mut()
                    .unwrap()
                    .entry("hooks")
                    .or_insert_with(|| serde_json::json!({}));
                let event_arr = hooks
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hooks is not an object".into()))?
                    .entry(&entry.event)
                    .or_insert_with(|| serde_json::json!([]));
                let arr = event_arr
                    .as_array_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("event is not an array".into()))?;
                let hook_val = serde_json::json!({ "type": "command", "command": entry.command });
                if !arr.contains(&hook_val) {
                    arr.push(hook_val);
                }
            }
            HookFormat::HermesYaml => {
                // Handled by the early return above; YAML is not JSON.
                unreachable!("HermesYaml handled before locked_modify_json")
            }
            HookFormat::KiroIde => {
                config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry("version")
                    .or_insert(serde_json::json!("v1"));
                let hooks = config
                    .as_object_mut()
                    .unwrap()
                    .entry("hooks")
                    .or_insert_with(|| serde_json::json!([]));
                let arr = hooks
                    .as_array_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hooks is not an array".into()))?;
                if !arr.iter().any(|h| {
                    kiro_hook_matches(h, &entry.event, entry.matcher.as_deref(), &entry.command)
                }) {
                    arr.push(kiro_hook_value(entry));
                }
            }
            HookFormat::None => {
                return Err(HkError::Internal("Agent does not support hooks".into()));
            }
        }
        Ok(())
    })
}

/// Remove a specific hook command from a config file by event, matcher, and command.
/// Only removes the given command from the group's hooks array.
/// If the hooks array becomes empty, removes the group.
/// If the event array becomes empty, removes the event key.
pub fn remove_hook(
    config_path: &Path,
    event: &str,
    matcher: Option<&str>,
    command: &str,
    format: HookFormat,
) -> Result<(), HkError> {
    if format == HookFormat::HermesYaml {
        return remove_hook_hermes_yaml(config_path, event, matcher, command);
    }
    if !config_path.exists() {
        return Ok(());
    }
    locked_modify_json(config_path, |config| {
        match format {
            HookFormat::ClaudeLike => {
                if let Some(hooks) = config.get_mut("hooks").and_then(|v| v.as_object_mut())
                    && let Some(event_arr) = hooks.get_mut(event).and_then(|v| v.as_array_mut())
                {
                    for group in event_arr.iter_mut() {
                        let group_matcher = group.get("matcher").and_then(|v| v.as_str());
                        if group_matcher != matcher {
                            continue;
                        }
                        if let Some(cmds) = group.get_mut("hooks").and_then(|v| v.as_array_mut()) {
                            // Match both string format "cmd" and object format {"type":"command","command":"cmd"}
                            cmds.retain(|c| {
                                if c.as_str() == Some(command) {
                                    return false;
                                }
                                if c.get("command").and_then(|v| v.as_str()) == Some(command) {
                                    return false;
                                }
                                true
                            });
                        }
                    }
                    event_arr.retain(|h| {
                        h.get("hooks")
                            .and_then(|v| v.as_array())
                            .map(|a| !a.is_empty())
                            .unwrap_or(true)
                    });
                    if event_arr.is_empty() {
                        hooks.remove(event);
                    }
                }
            }
            HookFormat::Cursor => {
                if let Some(hooks) = config.get_mut("hooks").and_then(|v| v.as_object_mut())
                    && let Some(event_arr) = hooks.get_mut(event).and_then(|v| v.as_array_mut())
                {
                    let cmd_val = serde_json::json!({ "command": command });
                    event_arr.retain(|h| h != &cmd_val);
                    if event_arr.is_empty() {
                        hooks.remove(event);
                    }
                }
            }
            HookFormat::Windsurf => {
                if let Some(hooks) = config.get_mut("hooks").and_then(|v| v.as_object_mut())
                    && let Some(event_arr) = hooks.get_mut(event).and_then(|v| v.as_array_mut())
                {
                    event_arr.retain(|h| {
                        h.get("command").and_then(|v| v.as_str()) != Some(command)
                            && h.get("powershell").and_then(|v| v.as_str()) != Some(command)
                    });
                    if event_arr.is_empty() {
                        hooks.remove(event);
                    }
                }
            }
            HookFormat::Copilot => {
                if let Some(hooks) = config.get_mut("hooks").and_then(|v| v.as_object_mut())
                    && let Some(event_arr) = hooks.get_mut(event).and_then(|v| v.as_array_mut())
                {
                    event_arr
                        .retain(|h| h.get("command").and_then(|v| v.as_str()) != Some(command));
                    if event_arr.is_empty() {
                        hooks.remove(event);
                    }
                }
            }
            HookFormat::HermesYaml => {
                // Handled by the early return above; YAML is not JSON.
                unreachable!("HermesYaml handled before locked_modify_json")
            }
            HookFormat::KiroIde => {
                if let Some(hooks) = config.get_mut("hooks").and_then(|v| v.as_array_mut()) {
                    hooks.retain(|h| !kiro_hook_matches(h, event, matcher, command));
                }
            }
            HookFormat::None => {
                return Err(HkError::Internal("Agent does not support hooks".into()));
            }
        }
        Ok(())
    })
}

/// Restore a previously disabled hook entry into the config file.
pub fn restore_hook(
    config_path: &Path,
    event: &str,
    entry: &serde_json::Value,
    format: HookFormat,
) -> Result<(), HkError> {
    if format == HookFormat::HermesYaml {
        return restore_hook_hermes_yaml(config_path, event, entry);
    }
    locked_modify_json(config_path, |config| {
        match format {
            HookFormat::ClaudeLike => {
                let hooks = config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry("hooks")
                    .or_insert_with(|| serde_json::json!({}));
                let event_arr = hooks
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hooks is not an object".into()))?
                    .entry(event)
                    .or_insert_with(|| serde_json::json!([]));
                let arr = event_arr
                    .as_array_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hook event is not an array".into()))?;
                arr.push(entry.clone());
            }
            HookFormat::Cursor | HookFormat::Copilot => {
                config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry("version")
                    .or_insert(serde_json::json!(1));
                let hooks = config
                    .as_object_mut()
                    .unwrap()
                    .entry("hooks")
                    .or_insert_with(|| serde_json::json!({}));
                let event_arr = hooks
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hooks is not an object".into()))?
                    .entry(event)
                    .or_insert_with(|| serde_json::json!([]));
                let arr = event_arr
                    .as_array_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hook event is not an array".into()))?;
                arr.push(entry.clone());
            }
            HookFormat::Windsurf => {
                let hooks = config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry("hooks")
                    .or_insert_with(|| serde_json::json!({}));
                let event_arr = hooks
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hooks is not an object".into()))?
                    .entry(event)
                    .or_insert_with(|| serde_json::json!([]));
                let arr = event_arr
                    .as_array_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hook event is not an array".into()))?;
                arr.push(entry.clone());
            }
            HookFormat::HermesYaml => {
                // Handled by the early return above; YAML is not JSON.
                unreachable!("HermesYaml handled before locked_modify_json")
            }
            HookFormat::KiroIde => {
                config
                    .as_object_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("Config is not an object".into()))?
                    .entry("version")
                    .or_insert(serde_json::json!("v1"));
                let hooks = config
                    .as_object_mut()
                    .unwrap()
                    .entry("hooks")
                    .or_insert_with(|| serde_json::json!([]));
                let arr = hooks
                    .as_array_mut()
                    .ok_or_else(|| HkError::ConfigCorrupted("hooks is not an array".into()))?;
                // Same (event, matcher, command) identity as deploy_hook, so a
                // double-restore doesn't duplicate the entry.
                let matcher = entry.get("matcher").and_then(|v| v.as_str());
                let command = entry
                    .get("action")
                    .and_then(|a| a.get("command"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if !arr
                    .iter()
                    .any(|h| kiro_hook_matches(h, event, matcher, command))
                {
                    arr.push(entry.clone());
                }
            }
            HookFormat::None => {
                return Err(HkError::Internal("Agent does not support hooks".into()));
            }
        }
        Ok(())
    })
}

/// Read a hook entry's full JSON value from a config file.
pub fn read_hook_config(
    config_path: &Path,
    event: &str,
    matcher: Option<&str>,
    command: &str,
    format: HookFormat,
) -> Result<Option<serde_json::Value>, HkError> {
    if format == HookFormat::HermesYaml {
        return read_hook_config_hermes_yaml(config_path, event, matcher, command);
    }
    if !config_path.exists() {
        return Ok(None);
    }
    let config = read_or_create_json(config_path)?;
    if format == HookFormat::KiroIde {
        let Some(hooks) = config.get("hooks").and_then(|v| v.as_array()) else {
            return Ok(None);
        };
        return Ok(hooks
            .iter()
            .find(|entry| kiro_hook_matches(entry, event, matcher, command))
            .cloned());
    }
    let hooks = config.get("hooks").and_then(|v| v.as_object());
    let Some(hooks) = hooks else {
        return Ok(None);
    };
    let Some(event_arr) = hooks.get(event).and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    match format {
        HookFormat::ClaudeLike => {
            for group in event_arr {
                let group_matcher = group.get("matcher").and_then(|v| v.as_str());
                if group_matcher != matcher {
                    continue;
                }
                if let Some(cmds) = group.get("hooks").and_then(|v| v.as_array())
                    && cmds.iter().any(|c| {
                        // Match both string format "cmd" and object format {"command":"cmd"}
                        c.as_str() == Some(command)
                            || c.get("command").and_then(|v| v.as_str()) == Some(command)
                    })
                {
                    return Ok(Some(group.clone()));
                }
            }
            Ok(None)
        }
        HookFormat::Cursor => {
            let cmd_val = serde_json::json!({ "command": command });
            for entry in event_arr {
                if entry == &cmd_val {
                    return Ok(Some(entry.clone()));
                }
            }
            Ok(None)
        }
        HookFormat::Windsurf => {
            for entry in event_arr {
                if entry.get("command").and_then(|v| v.as_str()) == Some(command)
                    || entry.get("powershell").and_then(|v| v.as_str()) == Some(command)
                {
                    return Ok(Some(entry.clone()));
                }
            }
            Ok(None)
        }
        HookFormat::Copilot => {
            for entry in event_arr {
                if entry.get("command").and_then(|v| v.as_str()) == Some(command) {
                    return Ok(Some(entry.clone()));
                }
            }
            Ok(None)
        }
        HookFormat::KiroIde => Ok(None),
        // Handled by the early return above; YAML is not JSON.
        HookFormat::HermesYaml => Ok(None),
        HookFormat::None => Ok(None),
    }
}

/// True if a hooks-list item matches (matcher, command).
fn hermes_hook_item_matches(
    item: &serde_yaml::Value,
    matcher: Option<&str>,
    command: &str,
) -> bool {
    let item_cmd = item.get("command").and_then(|v| v.as_str());
    let item_matcher = item.get("matcher").and_then(|v| v.as_str());
    item_cmd == Some(command) && item_matcher == matcher
}

/// YAML-based hook deploy for Hermes (`~/.hermes/config.yaml`, root "hooks" key).
/// Upserts `{matcher?, command}` under `hooks.<event>` (a list), preserving the
/// rest of config.yaml. Deduplicates on (matcher, command).
fn deploy_hook_hermes_yaml(config_path: &Path, entry: &HookEntry) -> Result<(), HkError> {
    modify_hermes_yaml(config_path, |root| {
        let hooks = root
            .entry("hooks".into())
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("hooks is not a mapping".into()))?;
        let list = hooks
            .entry(entry.event.clone().into())
            .or_insert_with(|| serde_yaml::Value::Sequence(vec![]))
            .as_sequence_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("hook event is not a sequence".into()))?;
        if list
            .iter()
            .any(|i| hermes_hook_item_matches(i, entry.matcher.as_deref(), &entry.command))
        {
            return Ok(()); // dedup
        }
        let mut item = serde_yaml::Mapping::new();
        if let Some(m) = &entry.matcher {
            item.insert("matcher".into(), m.clone().into());
        }
        item.insert("command".into(), entry.command.clone().into());
        list.push(serde_yaml::Value::Mapping(item));
        Ok(())
    })
}

/// YAML-based hook remove for Hermes. Drops the matching `{matcher?, command}`
/// item from `hooks.<event>`; removes the event key entirely if it becomes empty.
fn remove_hook_hermes_yaml(
    config_path: &Path,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) -> Result<(), HkError> {
    if !config_path.exists() {
        return Ok(());
    }
    modify_hermes_yaml(config_path, |root| {
        let Some(hooks) = root.get_mut("hooks").and_then(|v| v.as_mapping_mut()) else {
            return Ok(());
        };
        if let Some(list) = hooks.get_mut(event).and_then(|v| v.as_sequence_mut()) {
            list.retain(|i| !hermes_hook_item_matches(i, matcher, command));
            if list.is_empty() {
                hooks.remove(event);
            }
        }
        Ok(())
    })
}

/// YAML-based hook restore for Hermes. Pushes the previously-saved entry (stored
/// as a `serde_json::Value` by `read_hook_config_hermes_yaml`) back under
/// `hooks.<event>`.
fn restore_hook_hermes_yaml(
    config_path: &Path,
    event: &str,
    entry: &serde_json::Value,
) -> Result<(), HkError> {
    let yaml_item: serde_yaml::Value =
        serde_yaml::to_value(entry).map_err(|e| HkError::Internal(e.to_string()))?;
    modify_hermes_yaml(config_path, |root| {
        let hooks = root
            .entry("hooks".into())
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("hooks is not a mapping".into()))?;
        let list = hooks
            .entry(event.to_string().into())
            .or_insert_with(|| serde_yaml::Value::Sequence(vec![]))
            .as_sequence_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("hook event is not a sequence".into()))?;
        list.push(yaml_item);
        Ok(())
    })
}

/// YAML-based hook read for Hermes. Returns the matching `hooks.<event>` item
/// converted to a `serde_json::Value` (mirrors the JSON formats' saved-entry type).
fn read_hook_config_hermes_yaml(
    config_path: &Path,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) -> Result<Option<serde_json::Value>, HkError> {
    if !config_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(config_path)?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
        HkError::ConfigCorrupted(format!("Failed to parse Hermes config.yaml: {e}"))
    })?;
    let Some(item) = doc
        .get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|v| v.as_sequence())
        .and_then(|seq| {
            seq.iter()
                .find(|i| hermes_hook_item_matches(i, matcher, command))
        })
    else {
        return Ok(None);
    };
    let json_str = serde_json::to_string(item).map_err(|e| HkError::Internal(e.to_string()))?;
    let json_val = serde_json::from_str(&json_str).map_err(|e| HkError::Internal(e.to_string()))?;
    Ok(Some(json_val))
}

/// Flip a Kiro IDE hook's native `enabled` flag in place, keeping the entry
/// in the file — mirrors Kiro's own panel toggle ("skip without deleting").
pub fn set_kiro_hook_enabled(
    config_path: &Path,
    event: &str,
    matcher: Option<&str>,
    command: &str,
    enabled: bool,
) -> Result<(), HkError> {
    locked_modify_json(config_path, |config| {
        let hooks = config
            .get_mut("hooks")
            .and_then(|v| v.as_array_mut())
            .ok_or_else(|| HkError::NotFound("No hooks array found".into()))?;
        let hook = hooks
            .iter_mut()
            .find(|h| kiro_hook_matches(h, event, matcher, command))
            .ok_or_else(|| HkError::NotFound(format!("Hook for '{event}' not found in config")))?;
        let obj = hook
            .as_object_mut()
            .ok_or_else(|| HkError::ConfigCorrupted("hook is not an object".into()))?;
        obj.insert("enabled".into(), serde_json::Value::Bool(enabled));
        Ok(())
    })
}

fn kiro_hook_matches(
    hook: &serde_json::Value,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) -> bool {
    hook.get("trigger").and_then(|v| v.as_str()) == Some(event)
        && hook.get("matcher").and_then(|v| v.as_str()) == matcher
        && hook
            .get("action")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str())
            == Some("command")
        && hook
            .get("action")
            .and_then(|v| v.get("command"))
            .and_then(|v| v.as_str())
            == Some(command)
}

fn kiro_hook_value(entry: &HookEntry) -> serde_json::Value {
    let mut hook = serde_json::json!({
        "name": format!("{} {}", entry.event, entry.command),
        "trigger": entry.event,
        "action": { "type": "command", "command": entry.command },
    });
    if let Some(matcher) = &entry.matcher
        && let Some(obj) = hook.as_object_mut()
    {
        obj.insert("matcher".into(), serde_json::Value::String(matcher.clone()));
    }
    hook
}


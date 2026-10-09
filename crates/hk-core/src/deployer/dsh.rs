//! PenguinHarness/dsh managed-block patching: parse, render, insert, toggle.

use super::*;
use crate::deployer::fs_util::*;

pub(super) const DSH_BLOCK_BEGIN: &str = "# >>> managed by HarnessKit — do not edit this block >>>";

pub(super) const DSH_BLOCK_END: &str = "# <<< managed by HarnessKit <<<";

/// Structured model of the HK-owned managed block at the end of the home
/// `cordis.patch.yml`. ONE engine serves the MCP toggle, the MCP insert
/// writer, and the plugin toggle — no second block format, no second
/// marker pair.
#[derive(Debug, Default)]
pub(super) struct DshManagedBlock {
    /// Id-targeted `{id, disabled}` override entries. BTreeMap keeps the
    /// render order deterministic (sorted by row id).
    pub(super) toggles: std::collections::BTreeMap<String, bool>,
    /// Full HK-authored insert ROWS in insertion order. Each renders as its
    /// own `- insert:` group holding exactly one row mapping, and each row
    /// is `{id, name, config}` (+ optional `disabled`) per the mcp-client
    /// schema. Toggling an HK-inserted server edits the `disabled` field of
    /// its own row — never a separate override entry.
    pub(super) inserts: Vec<serde_yaml::Mapping>,
}

impl DshManagedBlock {
    fn is_empty(&self) -> bool {
        self.toggles.is_empty() && self.inserts.is_empty()
    }

    /// serverName of an insert row, but ONLY for mcp-client plugin rows —
    /// the same gate the reader applies, so every block-side matcher
    /// (find/remove/list) agrees with the reader's definition of an MCP row
    /// and can never match a non-MCP plugin insert.
    fn insert_server_name(row: &serde_yaml::Mapping) -> Option<&str> {
        if row.get("name")?.as_str()? != crate::adapter::dsh::MCP_CLIENT_PLUGIN {
            return None;
        }
        row.get("config")?.get("serverName")?.as_str()
    }

    fn find_insert_mut(&mut self, server_name: &str) -> Option<&mut serde_yaml::Mapping> {
        self.inserts
            .iter_mut()
            .find(|row| Self::insert_server_name(row) == Some(server_name))
    }

    fn remove_insert(&mut self, server_name: &str) -> bool {
        let before = self.inserts.len();
        self.inserts
            .retain(|row| Self::insert_server_name(row) != Some(server_name));
        self.inserts.len() != before
    }

    fn insert_row_ids(&self) -> Vec<String> {
        self.inserts
            .iter()
            .filter_map(|row| row.get("id").and_then(|v| v.as_str()).map(String::from))
            .collect()
    }

    fn insert_server_names(&self) -> Vec<String> {
        self.inserts
            .iter()
            .filter_map(|row| Self::insert_server_name(row).map(String::from))
            .collect()
    }
}

/// Flip a dsh MCP server via the official patch-layer mechanism: an
/// id-targeted `disabled:` override inside an HK-owned marked block at the
/// END of the home-level `cordis.patch.yml` (the last always-applied user
/// layer — later entries win in dsh's single ordered apply).
///
/// Hard rules (upstream-verified):
/// - Only ever writes `home_patch` — NEVER `<profileDir>/cordis.yml` (dsh
///   overwrites that on boot) and never any profile's patch file.
/// - User bytes outside the markers are preserved; the sole structural edits
///   involve the `[]` empty-list placeholder (see render_dsh_patch).
/// - The edited text must re-parse as a YAML sequence, else nothing is
///   written (a broken file would make dsh keep last-good config and
///   silently ignore all future edits).
pub fn set_dsh_mcp_enabled(
    home_patch: &Path,
    server_name: &str,
    enabled: bool,
) -> Result<(), HkError> {
    use crate::adapter::dsh::DshAdapter;

    // The install writer stores the SANITIZED serverName; match it on lookup.
    let server_name = &normalize_dsh_server_name(server_name);

    let (user_text, mut block) = read_and_split_home_patch(home_patch)?;

    // An HK-inserted server (Task-8 install writer) is toggled by editing the
    // `disabled` field of its OWN insert row — no separate override entry.
    if let Some(row) = block.find_insert_mut(server_name) {
        if enabled {
            row.remove("disabled");
        } else {
            row.insert(serde_yaml::Value::from("disabled"), serde_yaml::Value::from(true));
        }
        return write_dsh_patch(home_patch, &user_text, &block);
    }

    let row_id = DshAdapter::mcp_row_id_in_text(&user_text, server_name).ok_or_else(|| {
        HkError::NotFound(format!(
            "MCP server '{server_name}' not found in {}",
            home_patch.display()
        ))
    })?;

    // Base state = the file WITHOUT our block.
    let base_enabled = DshAdapter::mcp_enabled_in_text(&user_text)
        .get(server_name)
        .copied()
        .unwrap_or(true);

    if base_enabled == enabled {
        block.toggles.remove(&row_id);
    } else {
        block.toggles.insert(row_id, !enabled); // value = disabled flag
    }

    write_dsh_patch(home_patch, &user_text, &block)
}

/// Flip a dsh PLUGIN row via the same official patch-layer mechanism as the
/// MCP toggle: an id-targeted `disabled:` override inside the HK block at
/// the end of the home `cordis.patch.yml`. The home layer is applied after
/// every profile's layer, so the WRITE is machine-global — the one override
/// affects that row id in every profile that contains it (upstream
/// precedent: dsh's own web-app bundle disables base rows exactly this way;
/// hot-reload applies it live). Accepted side effect: disabling a row that
/// exists only in profile A leaves the override "dangling" from profile B's
/// perspective — dsh warn-skips it per boot; upstream cosmetic noise, not
/// surfaced by HK.
///
/// The BASE state, by contrast, is per-profile: dsh boots ONE profile at a
/// time and composes only the layers of THAT profile plus the home patch
/// (upstream composeProfile), so a sibling profile's file is never loaded
/// alongside it and must never be folded in. `base_layers` is that profile's
/// ordered chain below the home patch — each mounted bundle's own patch file
/// in `bundles` order, then the profile's `cordis.patch.yml` — exactly what
/// the UI row carried as `PluginEntry::base_layers`. The fold is
/// `base_layers ++ [home user text]`, our own managed block stripped.
///
/// The chain, not just the defining layer: `hmr` is DEFINED by
/// `@deepseek-ai/dsh-base` (enabled) and DISABLED by
/// `@deepseek-ai/dsh-web-app` two layers later. Folding only the definition
/// would read `hmr` as enabled, so "enable" would match the base state, drop
/// the override, and silently leave the row disabled.
///
/// Bundle patch files are read-only inputs here — HK never writes any layer
/// but the home patch, and never `<profileDir>/cordis.yml` (dsh overwrites
/// that on boot).
///
/// `home_patch` is taken as a path, not derived from a dsh home, so that it
/// is the SAME value the adapter hands out in `base_layers` — the
/// `layer == home_patch` test below must compare like with like. A re-derived
/// path (trailing slash, symlinked `$DSH_HOME`) would compare unequal for a
/// home-defined row, send us down the re-read branch, and fold HK's own
/// managed block back in as base state — every toggle a silent no-op.
pub fn set_dsh_plugin_enabled(
    home_patch: &Path,
    row_id: &str,
    enabled: bool,
    base_layers: &[PathBuf],
) -> Result<(), HkError> {
    use crate::adapter::dsh::DshAdapter;

    let (user_text, mut block) = read_and_split_home_patch(home_patch)?;

    // Fold every layer below the home patch, then the home user text. The
    // home patch is read through read_and_split_home_patch above (block
    // stripped), so never re-read it here: folding our own managed block
    // back in would make every toggle look like the base state.
    let mut texts: Vec<String> = base_layers
        .iter()
        .filter(|layer| layer.as_path() != home_patch)
        .map(|layer| std::fs::read_to_string(layer).unwrap_or_default())
        .collect();
    texts.push(user_text.clone());
    let mut defined = false;
    let mut disabled_state: Option<bool> = None;
    for text in &texts {
        let (layer_defined, layer_state) = DshAdapter::plugin_row_state_in_text(text, row_id);
        defined |= layer_defined;
        if let Some(d) = layer_state {
            disabled_state = Some(d);
        }
    }
    if !defined {
        let layers = base_layers
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(HkError::NotFound(format!(
            "plugin row '{row_id}' is not defined by any of [{layers}] or {}",
            home_patch.display()
        )));
    }
    let base_enabled = !disabled_state.unwrap_or(false);

    if base_enabled == enabled {
        block.toggles.remove(row_id);
    } else {
        block.toggles.insert(row_id.to_string(), !enabled);
    }
    write_dsh_patch(home_patch, &user_text, &block)
}

/// serverName must satisfy mcp-client's `/^[A-Za-z0-9_-]{1,32}$/`
/// (source-verified: packages/mcp/mcp-client/src/index.ts). Map every other
/// char to `-`, cap at 32; a name with no valid alphanumeric at all errors.
pub(super) fn sanitize_dsh_server_name(name: &str) -> Result<String, HkError> {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(32)
        .collect();
    if !cleaned.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(HkError::Validation(format!(
            "cannot derive a valid dsh serverName from '{name}' \
             (needs at least one of A-Za-z0-9; pattern [A-Za-z0-9_-], max 32 chars)"
        )));
    }
    Ok(cleaned)
}

/// Lookup-side normalization: identity for valid names; an unsanitizable
/// name is kept raw — it can't match any written row, so callers fall
/// through to "not found"/"no conflict". The install writer deliberately
/// does NOT use this (it must error on unsanitizable input).
pub(crate) fn normalize_dsh_server_name(name: &str) -> String {
    sanitize_dsh_server_name(name).unwrap_or_else(|_| name.to_string())
}

/// One full mcp-client insert row per the source-verified Config schema:
/// always an explicit `transport` discriminant and `serverName`; optional
/// keys only when non-empty (the schema defaults them). This is the ONLY
/// site that builds insert rows, so the key order (id, name, config) is
/// fixed here once (serde_yaml Mapping preserves insertion order — the
/// rendered byte format depends on it); env/header keys are sorted for the
/// same determinism.
///
/// Name round-trip: `serverName` must match mcp-client's
/// `/^[A-Za-z0-9_-]{1,32}$/`, so `microsoft/markitdown` is stored as
/// `microsoft-markitdown`. When sanitizing changed the name, the ORIGINAL is
/// recorded as `_hk_name` right after it so the reader can hand the scanner
/// the unsanitized name (`adapter::dsh::mcp_entries_in_text`). Without it the
/// scanner reads the row back under a different name and HK models it as a
/// SECOND extension — the ghost-duplicate-row bug. Same conditional as
/// Codex's `upsert_mcp_server_toml`: written ONLY when the name changed, so
/// already-valid names keep their exact previous bytes.
pub(super) fn build_dsh_insert_row(
    row_id: &str,
    server_name: &str,
    entry: &McpServerEntry,
) -> serde_yaml::Mapping {
    use crate::adapter::dsh::HK_NAME_CONFIG_KEY;
    use serde_yaml::{Mapping, Value};
    // Placed immediately after `serverName` in both transport branches — the
    // key it qualifies.
    let insert_hk_name = |config: &mut Mapping| {
        if server_name != entry.name {
            config.insert(
                Value::from(HK_NAME_CONFIG_KEY),
                Value::from(entry.name.clone()),
            );
        }
    };
    let mut config = Mapping::new();
    if entry.transport == McpTransport::Stdio {
        config.insert(Value::from("transport"), Value::from("stdio"));
        config.insert(Value::from("serverName"), Value::from(server_name));
        insert_hk_name(&mut config);
        config.insert(Value::from("command"), Value::from(entry.command.clone()));
        if !entry.args.is_empty() {
            config.insert(
                Value::from("args"),
                Value::Sequence(entry.args.iter().map(|a| Value::from(a.clone())).collect()),
            );
        }
        if !entry.env.is_empty() {
            let mut env = Mapping::new();
            let mut keys: Vec<&String> = entry.env.keys().collect();
            keys.sort();
            for k in keys {
                env.insert(Value::from(k.clone()), Value::from(entry.env[k].clone()));
            }
            config.insert(Value::from("env"), Value::Mapping(env));
        }
    } else {
        // Both Http and (schema-rejected upstream of this fn) Sse spell the
        // written transport as streamable-http — dsh ships no SSE transport,
        // and validate_remote_mcp_target refuses Sse before this point.
        config.insert(Value::from("transport"), Value::from("streamable-http"));
        config.insert(Value::from("serverName"), Value::from(server_name));
        insert_hk_name(&mut config);
        config.insert(
            Value::from("url"),
            Value::from(entry.url.clone().unwrap_or_default()),
        );
        if !entry.headers.is_empty() {
            let mut headers = Mapping::new();
            let mut keys: Vec<&String> = entry.headers.keys().collect();
            keys.sort();
            for k in keys {
                headers.insert(Value::from(k.clone()), Value::from(entry.headers[k].clone()));
            }
            config.insert(Value::from("headers"), Value::Mapping(headers));
        }
    }
    let mut row = Mapping::new();
    row.insert(Value::from("id"), Value::from(row_id));
    row.insert(
        Value::from("name"),
        Value::from(crate::adapter::dsh::MCP_CLIENT_PLUGIN),
    );
    row.insert(Value::from("config"), Value::Mapping(config));
    row
}

/// dsh MCP install: append a full `insert:` row (an mcp-client plugin row)
/// inside the HK managed block of the home `cordis.patch.yml`. Global scope
/// only — `mcp_config_path_for(Project)` stays `None` for dsh. User text
/// outside the block is byte-preserved, exactly as in the P0 toggle.
pub(super) fn deploy_mcp_server_dsh_cordis(
    config_path: &Path,
    entry: &McpServerEntry,
) -> Result<(), HkError> {
    use crate::adapter::dsh::DshAdapter;

    let (user_text, mut block) = read_and_split_home_patch(config_path)?;

    let server_name = sanitize_dsh_server_name(&entry.name)?;
    // Collision domain is the STORED `serverName`, so re-installing the same
    // ORIGINAL name sanitizes to the same key and is caught here — one row,
    // never a silent second one (the `_hk_name` round-trip only affects what
    // the READER reports, never how rows are keyed).
    if DshAdapter::mcp_enabled_in_text(&user_text).contains_key(&server_name)
        || block.insert_server_names().contains(&server_name)
    {
        // Name the original input too when sanitizing changed it — the
        // caller may otherwise not recognize the colliding name as theirs.
        let from = if server_name == entry.name {
            String::new()
        } else {
            format!(" (from '{}')", entry.name)
        };
        return Err(HkError::Validation(format!(
            "dsh already has an MCP server named '{server_name}'{from} in {}",
            config_path.display()
        )));
    }
    // Generated row id: mcp-<server-name>, kebab. Collision with ANY existing
    // row id is an error — a duplicate id would make dsh treat the second
    // definition as a malformed collision. The id namespace spans every
    // layer dsh composes, not just this file: profile patches are applied
    // BEFORE the home patch, so a profile row with the same id collides just
    // as hard. Checked here, in the ONE place that generates ids.
    let row_id = format!("mcp-{}", server_name.to_lowercase().replace('_', "-"));
    let profile_row_ids: std::collections::HashSet<String> = config_path
        .parent()
        .map(DshAdapter::profile_patch_texts)
        .unwrap_or_default()
        .iter()
        .flat_map(|text| DshAdapter::row_ids_in_text(text))
        .collect();
    if DshAdapter::row_ids_in_text(&user_text).contains(&row_id)
        || block.toggles.contains_key(&row_id)
        || block.insert_row_ids().contains(&row_id)
        || profile_row_ids.contains(&row_id)
    {
        return Err(HkError::Validation(format!(
            "row id '{row_id}' already exists in the dsh patch layers of {} — \
             rename the server or the existing row",
            config_path.display()
        )));
    }
    block.inserts.push(build_dsh_insert_row(&row_id, &server_name, entry));
    write_dsh_patch(config_path, &user_text, &block)
}

/// dsh MCP removal: HK-inserted rows (inside the managed block) are removed;
/// user-authored rows keep the Validation refusal — HK never rewrites user
/// YAML. An absent name is a no-op, matching every other format.
pub(super) fn remove_mcp_server_dsh_cordis(config_path: &Path, server_name: &str) -> Result<(), HkError> {
    use crate::adapter::dsh::DshAdapter;
    // The block stores the SANITIZED serverName; removal must map to it.
    let server_name = &normalize_dsh_server_name(server_name);
    let (user_text, mut block) = read_and_split_home_patch(config_path)?;
    if block.remove_insert(server_name) {
        return write_dsh_patch(config_path, &user_text, &block);
    }
    if DshAdapter::mcp_enabled_in_text(&user_text).contains_key(server_name) {
        return Err(HkError::Validation(format!(
            "'{server_name}' is a user-authored row in cordis.patch.yml; \
             HarnessKit never rewrites user YAML — remove the row in the file itself"
        )));
    }
    Ok(())
}

/// Shared prologue of every dsh home-patch writer: read the file, split off
/// the HK managed block, and name the file in any block-corruption error.
///
/// - Absent file = dsh has not seeded its template yet; start from the valid
///   empty form (`[]`). Any other IO error must surface, not be mistaken for
///   an empty file. The toggles never write this synthesized text — an empty
///   patch has no rows, so their row lookup fails first with "not found" —
///   while the install writer proceeds and creates the file, which is exactly
///   the desired first-install behavior. The remove writer relies on the
///   same synthesis for its idempotent no-op: an absent file has no rows, so
///   removal finds nothing and returns Ok instead of an IO error.
/// - The block parser is path-agnostic, so this call site owns naming the
///   file and the remediation hint on `ConfigCorrupted`.
pub(super) fn read_and_split_home_patch(home_patch: &Path) -> Result<(String, DshManagedBlock), HkError> {
    let original = match std::fs::read_to_string(home_patch) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "[]\n".to_string(),
        Err(e) => return Err(e.into()),
    };
    split_dsh_managed_block(&original).map_err(|e| match e {
        HkError::ConfigCorrupted(msg) => HkError::ConfigCorrupted(format!(
            "{msg} (in {}; fix or remove the content between the \
             '>>> managed by HarnessKit' markers)",
            home_patch.display()
        )),
        other => other,
    })
}

/// Split file text into (user text without the managed block, structured
/// block model). The block body is parsed as YAML: HK owns every byte
/// inside the markers, so content it would not itself render is a hard
/// `ConfigCorrupted` — refusing to write beats silently discarding block
/// entries. `split_inclusive` keeps user lines byte-exact (including CRLF).
///
/// Unbalanced markers are a hard `ConfigCorrupted` error: a BEGIN without a
/// matching END would otherwise swallow every user line to EOF (and the
/// rewritten file could still parse as a valid YAML sequence, so the
/// caller's post-edit guard would not catch the loss).
pub(super) fn split_dsh_managed_block(text: &str) -> Result<(String, DshManagedBlock), HkError> {
    let mut user = String::new();
    let mut body = String::new();
    let mut in_block = false;
    for raw in text.split_inclusive('\n') {
        let line = raw.trim();
        if line == DSH_BLOCK_BEGIN {
            if in_block {
                return Err(HkError::ConfigCorrupted(
                    "unbalanced HarnessKit managed-block markers: \
                     nested BEGIN marker inside the managed block"
                        .into(),
                ));
            }
            in_block = true;
            continue;
        }
        if line == DSH_BLOCK_END {
            if !in_block {
                return Err(HkError::ConfigCorrupted(
                    "unbalanced HarnessKit managed-block markers: \
                     END marker without a preceding BEGIN"
                        .into(),
                ));
            }
            in_block = false;
            continue;
        }
        if in_block {
            body.push_str(raw);
        } else {
            user.push_str(raw);
        }
    }
    if in_block {
        return Err(HkError::ConfigCorrupted(
            "unbalanced HarnessKit managed-block markers: \
             BEGIN marker without a matching END"
                .into(),
        ));
    }
    Ok((user, parse_dsh_block_body(&body)?))
}

/// Parse the marker-stripped block body into the structured model.
pub(super) fn parse_dsh_block_body(body: &str) -> Result<DshManagedBlock, HkError> {
    let mut block = DshManagedBlock::default();
    if body.trim().is_empty() {
        return Ok(block);
    }
    let corrupted = |detail: String| {
        HkError::ConfigCorrupted(format!(
            "HarnessKit managed block in cordis.patch.yml is not valid: {detail}"
        ))
    };
    let doc: serde_yaml::Value =
        serde_yaml::from_str(body).map_err(|e| corrupted(e.to_string()))?;
    let Some(items) = doc.as_sequence() else {
        return Err(corrupted("block body is not a YAML list".into()));
    };
    // Entries must carry EXACTLY the keys HK itself renders. Extra keys in an
    // id-targeted entry are LIVE dsh patch semantics (they would patch the
    // target row), so silently dropping them on re-render would alter the
    // user's effective config — hard error instead.
    let extra_keys = |map: &serde_yaml::Mapping, allowed: &[&str]| -> String {
        map.keys()
            .map(|k| k.as_str().unwrap_or("<non-string key>").to_string())
            .filter(|k| !allowed.contains(&k.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    for item in items {
        let Some(map) = item.as_mapping() else {
            return Err(corrupted("block entry is not a mapping".into()));
        };
        if let Some(rows) = map.get("insert").and_then(|v| v.as_sequence()) {
            if map.len() != 1 {
                return Err(corrupted(format!(
                    "insert group has keys besides insert: {}",
                    extra_keys(map, &["insert"])
                )));
            }
            for row in rows {
                let Some(rm) = row.as_mapping() else {
                    return Err(corrupted("insert row is not a mapping".into()));
                };
                block.inserts.push(rm.clone());
            }
            continue;
        }
        // `as_bool()` rejects `disabled: null`, which the READER
        // (adapter::dsh::yaml_disabled) accepts as `false` per upstream: HK
        // owns every byte between the markers and only ever writes literal
        // booleans, so a null in here means the block was hand-edited or
        // corrupted — refuse it rather than guess.
        match (
            map.get("id").and_then(|v| v.as_str()),
            map.get("disabled").and_then(|v| v.as_bool()),
        ) {
            (Some(id), Some(disabled)) => {
                if map.len() != 2 {
                    return Err(corrupted(format!(
                        "toggle entry has keys besides id/disabled: {}",
                        extra_keys(map, &["id", "disabled"])
                    )));
                }
                block.toggles.insert(id.to_string(), disabled);
            }
            _ => {
                return Err(corrupted(
                    "block entry is neither an {id, disabled} toggle nor an insert group".into(),
                ))
            }
        }
    }
    Ok(block)
}

/// Reassemble user text + managed block. Structural rules (unchanged from P0):
/// - Block present → any lone `[]` placeholder line is dropped (it can't
///   coexist with block-style entries in one document).
/// - Block absent → if the remaining text has no non-comment content, append
///   `[]` (an empty/comment-only patch file is a dsh boot error).
///
/// Rendering is deterministic: toggles sorted by id (BTreeMap) in the P0
/// byte format, then insert groups in insertion order with fixed key order
/// (serde_yaml Mapping preserves insertion order).
pub(super) fn render_dsh_patch(user_text: &str, block: &DshManagedBlock) -> String {
    if block.is_empty() {
        let has_content = user_text
            .lines()
            .any(|l| !l.trim().is_empty() && !l.trim().starts_with('#') && l.trim() != "[]");
        let has_placeholder = user_text.lines().any(|l| l.trim() == "[]");
        if has_content || has_placeholder {
            return user_text.to_string();
        }
        let mut out = user_text.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("[]\n");
        return out;
    }

    let mut body = String::new();
    for (id, disabled) in &block.toggles {
        body.push_str(&format!("- id: {id}\n  disabled: {disabled}\n"));
    }
    for row in &block.inserts {
        let mut group = serde_yaml::Mapping::new();
        group.insert(
            serde_yaml::Value::from("insert"),
            serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(row.clone())]),
        );
        let rendered = serde_yaml::to_string(&serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::Mapping(group),
        ]))
        .expect("HK-built YAML mapping always serializes");
        body.push_str(&rendered);
    }

    // Drop `[]` placeholder lines byte-preservingly (keep every other raw line).
    let mut out = String::new();
    for raw in user_text.split_inclusive('\n') {
        if raw.trim() != "[]" {
            out.push_str(raw);
        }
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(DSH_BLOCK_BEGIN);
    out.push('\n');
    out.push_str(&body);
    out.push_str(DSH_BLOCK_END);
    out.push('\n');
    out
}

/// Re-parse guard + atomic write shared by every dsh patch writer: the
/// edited text must stay a valid top-level YAML sequence, else nothing is
/// written (a broken file would make dsh keep last-good config and silently
/// ignore all future edits).
pub(super) fn write_dsh_patch(
    path: &Path,
    user_text: &str,
    block: &DshManagedBlock,
) -> Result<(), HkError> {
    let new_text = render_dsh_patch(user_text, block);
    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&new_text);
    if !matches!(parsed, Ok(serde_yaml::Value::Sequence(_))) {
        return Err(HkError::ConfigCorrupted(format!(
            "refusing to write {}: edited content is not a YAML list",
            path.display()
        )));
    }
    atomic_write(path, &new_text)
}


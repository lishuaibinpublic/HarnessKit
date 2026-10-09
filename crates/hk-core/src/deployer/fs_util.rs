//! Atomic and lock-protected JSON/JSONC file editing.

use super::*;

pub(super) fn read_or_create_json(path: &Path) -> Result<serde_json::Value, HkError> {
    if path.exists() {
        let content = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    } else {
        Ok(serde_json::json!({}))
    }
}

#[allow(dead_code)]
fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), HkError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

/// Write content to a file atomically: write to a temp file, then rename.
pub(super) fn atomic_write(path: &Path, content: &str) -> Result<(), HkError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Convert a `serde_json::Value` into the `CstInputValue` shape that
/// `jsonc-parser`'s CST mutation API expects. Used by the OpenCode and
/// OpenClaw write paths to feed existing serde-shaped entries (read off McpServerEntry, restored
/// off SQLite undo log, etc.) through CST `append` / `set_value`.
///
/// Note on key ordering: `serde_json::Value::Object` maps to
/// `serde_json::Map`, which is alphabetically sorted unless the
/// `preserve_order` feature is enabled (it isn't, here). New entries
/// therefore land with alphabetized keys — the same behavior as the
/// existing `to_string_pretty` path, so this isn't a regression.
pub(super) fn to_cst_input(v: &serde_json::Value) -> jsonc_parser::cst::CstInputValue {
    use jsonc_parser::cst::CstInputValue;
    match v {
        serde_json::Value::Null => CstInputValue::Null,
        serde_json::Value::Bool(b) => CstInputValue::Bool(*b),
        serde_json::Value::Number(n) => CstInputValue::Number(n.to_string()),
        serde_json::Value::String(s) => CstInputValue::String(s.clone()),
        serde_json::Value::Array(arr) => {
            CstInputValue::Array(arr.iter().map(to_cst_input).collect())
        }
        serde_json::Value::Object(obj) => CstInputValue::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), to_cst_input(v)))
                .collect(),
        ),
    }
}

/// Read-modify-write a jsonc-flavored config file with an exclusive advisory
/// file lock, preserving comments and formatting outside the modified area.
///
/// Mirrors `locked_modify_json`'s lock-and-rewrite semantics (no rename, so
/// the advisory lock isn't dropped mid-write), but parses with the CST API
/// instead of `serde_json::Value`. The closure receives the root `CstObject`
/// and operates on it via `get` / `append` / `object_value_or_set` / etc.
/// Comments and whitespace surrounding unmodified entries are kept verbatim.
///
/// jsonc-parser's default options already accept the full JSON5 surface
/// (single quotes, unquoted keys, trailing commas, hex numbers), so this one
/// helper serves OpenCode's `opencode.json(c)` and OpenClaw's JSON5
/// `openclaw.json` alike. Other agents' formats stay on `locked_modify_json`
/// (strict JSON).
pub(super) fn locked_modify_jsonc<F>(path: &Path, modify: F) -> Result<(), HkError>
where
    F: FnOnce(&jsonc_parser::cst::CstObject) -> Result<(), HkError>,
{
    use jsonc_parser::cst::CstRootNode;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock_exclusive()?;

    let mut content = String::new();
    (&file).read_to_string(&mut content)?;
    // Empty file → seed with "{}" so CstRootNode::parse always sees an
    // object root. Avoids bailing on a freshly-created config file whose
    // first write would otherwise be the root entry itself.
    let seed = if content.is_empty() {
        "{}"
    } else {
        content.as_str()
    };

    let cst = CstRootNode::parse(seed, &Default::default())
        .map_err(|e| HkError::ConfigCorrupted(format!("Failed to parse jsonc: {e}")))?;
    // Fail fast if root is non-object (e.g. user wrote `[1,2,3]` at top
    // level). `object_value_or_set` would silently destroy the array — we
    // refuse to do that.
    let root_obj = cst
        .object_value()
        .ok_or_else(|| HkError::ConfigCorrupted("Config root is not an object".into()))?;

    modify(&root_obj)?;

    let output = cst.to_string();
    (&file).seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    (&file).write_all(output.as_bytes())?;
    (&file).flush()?;

    file.unlock()?;
    Ok(())
}

/// Read-modify-write a JSON config file with an exclusive advisory file lock.
pub(super) fn locked_modify_json<F>(path: &Path, modify: F) -> Result<(), HkError>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), HkError>,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock_exclusive()?;

    let mut content = String::new();
    (&file).read_to_string(&mut content)?;
    let mut config: serde_json::Value = if content.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&content)?
    };

    modify(&mut config)?;

    let output = serde_json::to_string_pretty(&config)?;
    (&file).seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    (&file).write_all(output.as_bytes())?;
    (&file).flush()?;

    file.unlock()?;
    Ok(())
}


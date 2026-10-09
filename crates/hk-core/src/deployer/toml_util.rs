//! TOML table/array editing helpers.

use super::*;
use crate::deployer::fs_util::*;

pub(super) fn read_toml_table(path: &Path) -> Result<toml::Table, HkError> {
    if !path.exists() {
        return Ok(toml::Table::new());
    }
    let content = std::fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(toml::Table::new());
    }
    content
        .parse::<toml::Table>()
        .map_err(|e| HkError::ConfigCorrupted(format!("Failed to parse TOML config: {e}")))
}

pub(super) fn modify_toml_table(
    path: &Path,
    f: impl FnOnce(&mut toml::Table) -> Result<(), HkError>,
) -> Result<(), HkError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut doc = read_toml_table(path)?;
    f(&mut doc)?;
    atomic_write(
        path,
        &toml::to_string_pretty(&doc).map_err(|e| HkError::Internal(e.to_string()))?,
    )
}

pub(super) fn toml_string_array(table: &toml::Table, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn set_string_array(table: &mut toml::Table, key: &str, values: &[String]) {
    if values.is_empty() {
        table.remove(key);
    } else {
        table.insert(
            key.into(),
            toml::Value::Array(values.iter().map(|s| toml::Value::String(s.clone())).collect()),
        );
    }
}

pub(super) fn remove_string_from_array(table: &mut toml::Table, key: &str, value: &str) {
    let mut values = toml_string_array(table, key);
    values.retain(|v| v != value);
    set_string_array(table, key, &values);
}


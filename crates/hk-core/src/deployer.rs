use crate::HkError;
use crate::adapter::{HookEntry, HookFormat, McpFormat, McpServerEntry, McpTransport, RemoteMcpSchema};
use fs2::FileExt;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

mod dsh;
mod fs_util;
mod hooks;
mod mcp;
mod skill;
mod toggles;
mod toml_util;
#[cfg(test)]
mod tests;

pub use dsh::{
    set_dsh_mcp_enabled, set_dsh_plugin_enabled,
};
pub use hooks::{
    deploy_hook, read_hook_config, remove_hook, restore_hook, set_kiro_hook_enabled,
};
pub use mcp::{
    build_path_for_command, deploy_mcp_server, ensure_path_injection, read_mcp_server_config, remove_mcp_server, resolve_command_path, restore_mcp_server, sanitize_mcp_name, set_openclaw_mcp_enabled,
};
pub use skill::{
    deploy_skill,
};
pub use toggles::{
    read_plugin_config, remove_codex_plugin_entry, remove_gemini_extension_entry, remove_grok_plugin_lists, remove_plugin_entry, remove_vscode_plugin_entry, restore_plugin_entry, set_codex_plugin_enabled, set_gemini_extension_enabled, set_grok_hook_enabled, set_grok_mcp_enabled, set_grok_plugin_enabled, set_hermes_mcp_enabled, set_hermes_plugin_enabled, set_kiro_mcp_enabled, set_omp_mcp_enabled, set_plugin_enabled, set_vscode_plugin_enabled,
};
pub(crate) use dsh::normalize_dsh_server_name;

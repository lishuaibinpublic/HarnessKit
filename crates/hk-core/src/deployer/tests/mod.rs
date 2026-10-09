//! Shared fixtures and submodule layout for deployer tests.

use super::*;
use crate::deployer::dsh::*;
use crate::deployer::fs_util::*;
use crate::deployer::hooks::*;
use crate::deployer::mcp::*;
use crate::deployer::skill::*;
use crate::deployer::toggles::*;

/// A representative adapter for each MCP format, so deploy tests can
/// exercise `deploy_mcp_server`'s real dispatch (format + remote schema).
fn test_adapter(format: McpFormat) -> Box<dyn crate::adapter::AgentAdapter> {
use crate::adapter::*;
let home = std::path::PathBuf::from("/nonexistent");
match format {
    McpFormat::McpServers => Box::new(claude::ClaudeAdapter::with_home(home)),
    McpFormat::Servers => Box::new(copilot::CopilotAdapter::with_home(home)),
    McpFormat::Toml => Box::new(codex::CodexAdapter::with_home(home)),
    McpFormat::Opencode => Box::new(opencode::OpencodeAdapter::with_home(home)),
    McpFormat::HermesYaml => Box::new(hermes::HermesAdapter::with_home(home)),
    McpFormat::DshCordis => Box::new(dsh::DshAdapter::with_home(home)),
    McpFormat::GrokToml => Box::new(grok::GrokAdapter::with_home(home)),
    McpFormat::OpenClawJson5 => Box::new(openclaw::OpenClawAdapter::with_home(home)),
}
}

fn remote_entry(transport: McpTransport) -> McpServerEntry {
McpServerEntry {
    name: "linear".into(),
    transport,
    url: Some("https://mcp.linear.app/mcp".into()),
    headers: [("Authorization".to_string(), "Bearer tok".to_string())].into(),
    ..Default::default()
}
}

mod dsh;
mod fs_util;
mod hooks;
mod mcp;
mod skill;
mod toggles;

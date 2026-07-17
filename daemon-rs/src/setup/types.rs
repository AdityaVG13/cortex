// SPDX-License-Identifier: MIT
use std::path::PathBuf;

// ─── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DetectedTool {
    pub name: &'static str,
    pub agent_name: &'static str,
    pub config_path: Option<PathBuf>,
    pub config_method: ConfigMethod,
}

#[derive(Debug, Clone)]
pub enum ConfigMethod {
    /// Write MCP server entry to a JSON config file
    JsonMerge,
    /// Write MCP server entry to a TOML config file
    TomlMerge,
    /// Run a CLI command (e.g., `claude mcp add`)
    CliCommand {
        program: &'static str,
        args: &'static [&'static str],
    },
    /// Show manual instructions to the user
    #[allow(dead_code)]
    Manual(String),
}

#[derive(Debug)]
pub enum StepResult {
    Ok(String),
    Warn(String),
    Fail(String),
}

impl StepResult {
    pub(crate) fn icon(&self) -> &str {
        match self {
            StepResult::Ok(_) => "[OK]",
            StepResult::Warn(_) => "[!!]",
            StepResult::Fail(_) => "[FAIL]",
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            StepResult::Ok(m) | StepResult::Warn(m) | StepResult::Fail(m) => m,
        }
    }
}

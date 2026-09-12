//! Process edge: CLI, MCP stdio, hooks, and setup.
//! Brain types live in `cortex-kernel` and `cortex-logic`. Import those crates
//! directly from hosts and contracts.

#![forbid(unsafe_code)]

pub mod cli;
pub mod handlers;
pub mod hook_boot;
pub mod mcp_native;
pub mod prompt_inject;
pub mod setup;

pub use cli::run_daemon;

pub(crate) use cortex_kernel::{auth, compaction, compiler, crystallize, db, runtime, state, CortexRuntime};
pub(crate) use cortex_logic::{clockwork, eval};

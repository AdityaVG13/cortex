//! Daemon-owned adapters: MCP, health presentation, boot, and presence.
//! Store, recall, and operations live in `cortex_kernel::handlers`.
pub mod auth;
pub mod boot;
pub mod health;
pub mod mcp;
pub use auth::{register_agent_presence, SourceIdentity};

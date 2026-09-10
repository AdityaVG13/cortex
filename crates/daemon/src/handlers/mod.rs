//! Transport-free handler APIs shared by the supervisor and MCP edge.
pub use cortex_kernel::handlers::{
    estimate_tokens, estimate_tokens_from_chars, event_log, feedback, log_event, mutate, now_iso, operations, parse_duration_to_seconds, parse_json_array,
    parse_timestamp_ms, recall, redact_secrets, redaction, store, truncate_chars, ResponseStatus,
};
pub mod auth;
pub mod boot;
pub mod health;
pub mod mcp;
pub use auth::{register_agent_presence, SourceIdentity};

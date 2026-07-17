mod dispatch;
mod handler;
mod permissions;
mod queries;
mod rpc;
mod session;
#[cfg(test)]
mod tests;
mod tools;
pub(crate) use dispatch::mcp_dispatch;
pub use handler::handle_mcp_message_with_caller;
pub(crate) use permissions::{
    enforce_client_permission, mcp_session_description, mcp_session_owner_id,
    normalize_mcp_agent_label, normalize_permission_client_id, parse_client_permission,
    refresh_mcp_session_presence, required_permission_for_tool, source_agent_for_tool,
    source_client_for_permissions, source_model_for_tool, ClientPermission, McpPresenceDisposition,
};
pub(crate) use queries::{clear_served_scope_for_boot, fetch_last_call};
pub(crate) use rpc::{
    arg_f64, arg_i64, arg_str, arg_usize, mcp_error_with_data, mcp_resource_payload,
    mcp_resource_read_result, mcp_resource_uris, mcp_resources, tool_name_suggestions,
    wrap_mcp_tool_result, wrap_mcp_tool_result_verbose,
};
pub use rpc::{mcp_error, mcp_success};
pub(crate) use session::upsert_mcp_session;
pub use tools::mcp_tools;

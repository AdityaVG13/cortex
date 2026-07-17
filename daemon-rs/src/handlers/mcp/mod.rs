mod dispatch;
mod handler;
mod permissions;
mod queries;
mod rpc;
#[cfg(test)]
mod tests;
mod tools;
pub(crate) use dispatch::mcp_dispatch;
pub use handler::handle_mcp_message_with_caller;
pub(crate) use permissions::{enforce_client_permission, required_permission_for_tool, ClientPermission};
#[cfg(test)]
pub(crate) use permissions::{normalize_permission_client_id, parse_client_permission};
pub(crate) use queries::fetch_last_call;
pub(crate) use rpc::{
    arg_i64, arg_str, arg_usize, mcp_error_with_data, mcp_resource_payload, mcp_resource_read_result, mcp_resource_uris, mcp_resources, tool_name_suggestions,
    wrap_mcp_tool_result, wrap_mcp_tool_result_verbose,
};
pub use rpc::{mcp_error, mcp_success};
pub use tools::mcp_tools;

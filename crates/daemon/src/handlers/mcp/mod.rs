mod dispatch;
mod handler;
mod modern;
mod permissions;
mod queries;
mod rpc;

mod tools;
pub(crate) use dispatch::mcp_dispatch;
pub use handler::handle_mcp_message_with_caller;
pub(crate) use modern::{
    RequestEra, RetryInput, apply_input_responses, cacheable_result, client_supports_elicitation_form, complete_argument, complete_result, discover_result,
    input_required_result, mcp_prompts, mcp_resource_templates, missing_field_of, prompt_messages, request_client_capabilities, request_era,
    supported_versions,
};
pub(crate) use permissions::{ClientPermission, enforce_client_permission, normalize_permission_client_id, required_permission_for_tool};

pub(crate) use queries::fetch_last_call;
pub(crate) use rpc::{
    arg_i64, arg_str, arg_usize, mcp_error_with_data, mcp_resource_payload, mcp_resource_read_result, mcp_resource_uris, mcp_resources, tool_name_suggestions,
    wrap_mcp_tool_result, wrap_mcp_tool_result_verbose,
};
pub use rpc::{mcp_error, mcp_success};
pub use tools::{legacy_mcp_tools, mcp_tools, removed_tool_replacements};

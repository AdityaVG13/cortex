use super::{
    mcp_dispatch, mcp_error, mcp_error_with_data, mcp_resource_payload, mcp_resource_read_result, mcp_resource_uris, mcp_resources,
    mcp_success, mcp_tools, required_permission_for_tool, tool_name_suggestions, wrap_mcp_tool_result, wrap_mcp_tool_result_verbose,
};
use crate::handlers::SourceIdentity;
use crate::state::RuntimeState;
use serde_json::{json, Value};
pub async fn handle_mcp_message_with_caller(
    state: &RuntimeState, msg: &Value, caller_id: Option<i64>, source: Option<&SourceIdentity>,
) -> Option<Value> {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    if !msg.is_object() {
        return Some(mcp_error(id, -32600, "Invalid JSON-RPC request"));
    }
    match msg.get("jsonrpc").and_then(|v| v.as_str()) {
        Some("2.0") => {}
        Some(_) => return Some(mcp_error(id, -32600, "Invalid JSON-RPC version")),
        None => return Some(mcp_error(id, -32600, "Missing JSON-RPC version")),
    }
    let Some(method) = msg.get("method").and_then(|v| v.as_str()) else {
        return Some(mcp_error(id, -32600, "Missing JSON-RPC method"));
    };
    match method {
        "initialize" => Some(mcp_success(
            id,
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":true},"resources":{"listChanged":
true}},"serverInfo":{"name":"cortex","version":env!("CARGO_PKG_VERSION")}}),
        )),
        "notifications/initialized" => None,
        "tools/list" => Some(mcp_success(id, json!({"tools":mcp_tools()}))),
        "resources/list" => Some(mcp_success(id, json!({"resources":mcp_resources()}))),
        "resources/read" => {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let uri = params.get("uri").and_then(Value::as_str).map(str::trim).unwrap_or_default();
            if uri.is_empty() {
                return Some(mcp_error_with_data(
                    id,
                    -32602,
                    "Missing resource URI",
                    json
!({"errorType":"MISSING_RESOURCE_URI","availableResources":mcp_resource_uris(),"fixHint":
"Call resources/list, then pass one of the returned uri values to resources/read."}),
                ));
            }
            match mcp_resource_payload(uri) {
                Some(payload) => Some(mcp_success(id, mcp_resource_read_result(uri, payload))),
                None => Some(mcp_error_with_data(
                    id,
                    -32602,
                    &format!("Unknown resource URI: {uri}"),
                    json!({"errorType":"UNKNOWN_RESOURCE","provided":uri,"availableResources":mcp_resource_uris(),
"fixHint":"Call resources/list to discover valid Cortex MCP resource URIs."}),
                )),
            }
        }
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            if tool_name.is_empty() {
                return Some(mcp_error_with_data(
                    id,
                    -32602,
                    "Missing tool name",
                    json!({"errorType":"MISSING_TOOL_NAME","fixHint":
"Call tools/list or read cortex://tooling/tools, then pass params.name exactly.","availableToolCount":mcp_tools().len()}),
                ));
            }
            if required_permission_for_tool(tool_name).is_none() {
                return Some(mcp_error_with_data(
                    id,
                    -32601,
                    &format!("Unknown tool: {tool_name}"),
                    json!({"errorType":"UNKNOWN_TOOL","provided":tool_name,"suggestions":tool_name_suggestions(tool_name),"discoveryHint":
"Call tools/list for full schemas or read cortex://tooling/tools for a compact catalog.","availableToolCount":mcp_tools().len()}),
                ));
            }
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match mcp_dispatch(state, caller_id, tool_name, &args, source).await {
                Ok(result) => {
                    let wrapped = if tool_name == "cortex_health" || tool_name == "cortex_digest" {
                        wrap_mcp_tool_result_verbose(state, result)
                    } else {
                        wrap_mcp_tool_result(state, result)
                    };
                    Some(mcp_success(id, wrapped))
                }
                Err(err) => Some(mcp_success(
                    id,
                    json!({
"content":[{"type":"text","text":json!({"error":err}).to_string()}],"isError":true}),
                )),
            }
        }
        _ => {
            if msg.get("id").is_some() {
                Some(mcp_error(id, -32601, &format!("Method not found: {method}")))
            } else {
                None
            }
        }
    }
}

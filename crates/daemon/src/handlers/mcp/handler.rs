use super::{
    RequestEra, RetryInput, apply_input_responses, cacheable_result, client_supports_elicitation_form, complete_argument, complete_result, discover_result,
    input_required_result, mcp_dispatch, mcp_error, mcp_error_with_data, mcp_prompts, mcp_resource_payload, mcp_resource_read_result, mcp_resource_templates,
    mcp_resource_uris, mcp_resources, mcp_success, mcp_tools, missing_field_of, prompt_messages, request_client_capabilities, request_era,
    required_permission_for_tool, supported_versions, tool_name_suggestions, wrap_mcp_tool_result, wrap_mcp_tool_result_verbose,
};
use crate::handlers::SourceIdentity;
use crate::state::RuntimeState;
use serde_json::{Value, json};

/// Era framing for a success result: modern requests get `resultType` plus
/// server identity; legacy shapes pass through byte-identical.
fn era_frame(result: Value, modern: bool) -> Value {
    if modern { complete_result(result) } else { result }
}

/// Era framing for a catalog result: modern requests additionally get the
/// required cacheability hint.
fn era_catalog(result: Value, modern: bool) -> Value {
    if modern { cacheable_result(complete_result(result)) } else { result }
}

pub async fn handle_mcp_message_with_caller(
    cx: &asupersync::Cx, state: &RuntimeState, msg: &Value, caller_id: Option<i64>, source: Option<&SourceIdentity>,
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
    // Team-mode caller gate, split by surface idiom: `tools/call` denials are
    // `isError` envelopes produced by dispatch (caller-scoped key required);
    // every other surface keeps the top-level refusal.
    if state.team_mode && caller_id.is_none() && method != "tools/call" {
        return Some(mcp_error(id, -32000, "Team mode requires a local owner"));
    }
    // Per-request era. An explicit unsupported version is
    // `UnsupportedProtocolVersionError` (-32022) with the exact spec data
    // shape. Notifications (no id) never learn their era: a notification gets
    // no reply at all, so version validation would have nowhere to go.
    let modern = match (msg.get("id").is_some(), request_era(msg)) {
        (true, Err(requested)) => {
            return Some(mcp_error_with_data(id, -32022, "Unsupported protocol version", json!({"supported": supported_versions(), "requested": requested})));
        }
        (true, Ok(era)) => era == RequestEra::Modern,
        (false, _) => false,
    };
    match method {
        // `initialize` always selects legacy semantics (spec dual-era): the
        // handshake revision we speak is 2024-11-05, byte-identical. Modern
        // clients never send this; they probe `server/discover` instead.
        "initialize" => Some(mcp_success(
            id,
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":true},"resources":{"listChanged":true}},"serverInfo":{"name":"cortex","version":env!("CARGO_PKG_VERSION")}}),
        )),
        "notifications/initialized" => None,
        // 2026-07-28 discovery: versions, capabilities, instructions. The
        // result implies modern framing regardless of request `_meta`.
        "server/discover" => Some(mcp_success(id, discover_result())),
        // `ping` was removed in 2026-07-28 but existed in the legacy era;
        // answering it is a harmless courtesy to older clients.
        "ping" => Some(mcp_success(id, era_frame(json!({}), modern))),
        // `logging/setLevel` was removed in 2026-07-28 (per-request
        // `logLevel`, which needs no server action here — this server never
        // emits log notifications). The informative -32601 stands for callers
        // from either era.
        "logging/setLevel" => Some(mcp_error_with_data(
            id,
            -32601,
            "logging/setLevel is not supported",
            json!({"errorType":"LOGGING_LEVEL_UNSUPPORTED","fixHint":"This server does not implement client-controlled logging levels."}),
        )),
        // `notifications/cancelled` is the only client notification in
        // 2026-07-28. Accepted; all current ops are ms-scale, so there is
        // never anything in flight to cancel. Every other notification falls
        // through to silent acceptance below.
        "notifications/cancelled" => None,
        "tools/list" => Some(mcp_success(id, era_catalog(json!({"tools":mcp_tools()}), modern))),
        "resources/list" => Some(mcp_success(id, era_catalog(json!({"resources":mcp_resources()}), modern))),
        "resources/templates/list" => Some(mcp_success(id, era_catalog(json!({"resourceTemplates":mcp_resource_templates()}), modern))),
        // `resources/subscribe` was removed in 2026-07-28; the informative
        // error stands for legacy-era callers.
        "resources/subscribe" | "resources/unsubscribe" => Some(mcp_error_with_data(
            id,
            -32601,
            &format!("{method} is not supported"),
            json!({"errorType":"SUBSCRIPTIONS_UNSUPPORTED","fixHint":"Poll resources/read; this server has no live-update push channel."}),
        )),
        // `subscriptions/listen` needs a long-lived push stream, which this
        // one-message-per-call transport cannot hold — and there would be
        // nothing to push: the catalogs it could watch never change.
        // Answered, not faked: poll the catalog methods.
        "subscriptions/listen" => Some(mcp_error_with_data(
            id,
            -32601,
            "subscriptions/listen is not supported",
            json!({"errorType":"SUBSCRIPTIONS_UNSUPPORTED","fixHint":"Poll tools/list, resources/list, or resources/read; this server holds no subscription streams."}),
        )),
        "prompts/list" => Some(mcp_success(id, era_catalog(json!({"prompts":mcp_prompts()}), modern))),
        "prompts/get" => {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match prompt_messages(name, &args) {
                Some(rendered) => Some(mcp_success(id, era_frame(rendered, modern))),
                None => Some(mcp_error_with_data(
                    id,
                    -32602,
                    &format!("Unknown prompt: {name}"),
                    json!({"errorType":"UNKNOWN_PROMPT","provided":name,"availablePrompts":mcp_prompts().iter().filter_map(|p| p.get("name").and_then(Value::as_str)).collect::<Vec<_>>()}),
                )),
            }
        }
        "completion/complete" => {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let ref_type = params.pointer("/ref/type").and_then(Value::as_str).unwrap_or_default();
            let ref_name = params.pointer("/ref/name").or_else(|| params.pointer("/ref/uri")).and_then(Value::as_str).unwrap_or_default();
            let arg_name = params.pointer("/argument/name").and_then(Value::as_str).unwrap_or_default();
            let value = params.pointer("/argument/value").and_then(Value::as_str).unwrap_or_default();
            let values = complete_argument(ref_type, ref_name, arg_name, value);
            Some(mcp_success(id, era_frame(json!({"completion":{"values":values,"hasMore":false}}), modern)))
        }
        "resources/read" => {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let uri = params.get("uri").and_then(Value::as_str).and_then(cortex_logic::protocol::nonempty_str).unwrap_or_default();
            if uri.is_empty() {
                return Some(mcp_error_with_data(
                    id,
                    -32602,
                    "Missing resource URI",
                    json!({"errorType":"MISSING_RESOURCE_URI","availableResources":mcp_resource_uris(),"fixHint":"Call resources/list, then pass one of the returned uri values to resources/read."}),
                ));
            }
            match mcp_resource_payload(uri) {
                Some(payload) => Some(mcp_success(id, era_catalog(mcp_resource_read_result(uri, payload), modern))),
                None => Some(mcp_error_with_data(
                    id,
                    -32602,
                    &format!("Unknown resource URI: {uri}"),
                    json!({"errorType":"UNKNOWN_RESOURCE","provided":uri,"availableResources":mcp_resource_uris(),"fixHint":"Call resources/list to discover valid Cortex MCP resource URIs."}),
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
                    json!({"errorType":"MISSING_TOOL_NAME","fixHint":"Call tools/list or read cortex://tooling/tools, then pass params.name exactly.","availableToolCount":mcp_tools().len()}),
                ));
            }
            if required_permission_for_tool(tool_name).is_none() {
                let mut data = json!({"errorType":"UNKNOWN_TOOL","provided":tool_name,"suggestions":tool_name_suggestions(tool_name),"discoveryHint":"Call tools/list for full schemas or read cortex://tooling/tools for a compact catalog.","availableToolCount":mcp_tools().len()});
                if let Some(replacement) = super::removed_tool_replacements().get(tool_name).and_then(Value::as_str) {
                    data["removed"] = json!(true);
                    data["replacement"] = json!(replacement);
                    data["fixHint"] = json!(format!("Removed tool `{tool_name}`; use `{replacement}`."));
                }
                return Some(mcp_error_with_data(id, -32601, &format!("Unknown tool: {tool_name}"), data));
            }
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // MRTR retry fold: elicited values merge into the arguments before
            // dispatch. A refusal ends the call with a terminal envelope.
            let mut answered: Vec<String> = Vec::new();
            let mut args = args;
            match apply_input_responses(&args, &params) {
                Err(protocol) => {
                    return Some(mcp_error_with_data(
                        id,
                        -32602,
                        &protocol,
                        json!({"errorType":"INVALID_INPUT_RESPONSES","fixHint":"Echo the inputRequests keys with accept/decline/cancel actions and content matching the requested schema."}),
                    ));
                }
                Ok(Some(RetryInput::Declined { field })) => {
                    let payload = json!({"status":"invalid_request","error":format!("caller declined to provide `{field}`; the call cannot proceed without it"),"field":field});
                    return Some(mcp_success(id, era_frame(wrap_mcp_tool_result(payload), modern)));
                }
                Ok(Some(RetryInput::Merged { args: merged, answered: newly })) => {
                    args = merged;
                    answered = newly;
                }
                Ok(None) => {}
            }
            match mcp_dispatch(cx, state, caller_id, tool_name, &args, source).await {
                Ok(result) => {
                    // MRTR elicitation: a flat missing field becomes a form
                    // request when the call is modern and the client declared
                    // form support. A field answered on this very retry that
                    // still fails falls through to the plain envelope — one
                    // prompt per field, never a second.
                    if modern && let Some((field, message)) = missing_field_of(&result) {
                        let caps = request_client_capabilities(msg);
                        if !answered.iter().any(|a| a == &field) && client_supports_elicitation_form(&caps) {
                            return Some(mcp_success(id, input_required_result(&field, &message)));
                        }
                    }
                    let wrapped = if tool_name == "cortex_health" || tool_name == "cortex_digest" {
                        wrap_mcp_tool_result_verbose(state, result)
                    } else {
                        wrap_mcp_tool_result(result)
                    };
                    Some(mcp_success(id, era_frame(wrapped, modern)))
                }
                Err(err) => {
                    let payload = json!({"error": err});
                    Some(mcp_success(
                        id,
                        era_frame(json!({"content":[{"type":"text","text":payload.to_string()}],"isError":true,"structuredContent":payload}), modern),
                    ))
                }
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

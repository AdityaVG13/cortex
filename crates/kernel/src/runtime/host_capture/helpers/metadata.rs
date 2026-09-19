use super::*;
use serde_json::Value;

fn require_record_version(v: &Value) -> Result<(), String> {
    if field(v, "version")? != "2.1.260" {
        Err("unsupported_host_record_version".into())
    } else {
        Ok(())
    }
}

pub(in crate::runtime::host_capture) fn metadata_kind(
    grant: &HostCaptureGrant,
    context: &HostCaptureContext,
    line: &[u8],
) -> Result<Option<&'static str>, String> {
    if grant.adapter_version != CLAUDE_2_1_260_ADAPTER {
        return Ok(None);
    }
    let v = parse_host_json(line)?;
    if field(&v, "sessionId")? != context.session_id {
        return Err("host_session_mismatch".into());
    }
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return Err("unsupported_host_sidechain".into());
    }
    if v.get("version")
        .is_some_and(|version| version.as_str() != Some("2.1.260"))
    {
        return Err("unsupported_host_record_version".into());
    }
    match v.get("type").and_then(Value::as_str) {
        Some("queue-operation") => match field(&v, "operation")? {
            "enqueue" => {
                field(&v, "content")?;
                Ok(Some("queue_enqueue"))
            }
            "dequeue" => Ok(Some("queue_dequeue")),
            _ => Err("unsupported_host_queue_operation".into()),
        },
        Some("atis-latch") => {
            field(&v, "atis")?;
            Ok(Some("atis_latch"))
        }
        Some("last-prompt") => {
            field(&v, "lastPrompt")?;
            label(field(&v, "leafUuid")?)?;
            Ok(Some("last_prompt"))
        }
        Some("attachment") => {
            require_record_version(&v)?;
            Ok(
                match v
                    .get("attachment")
                    .and_then(|a| a.get("type"))
                    .and_then(Value::as_str)
                {
                    Some("total_tokens_reminder") => Some("token_budget_attachment"),
                    Some("hook_additional_context") => Some("hook_context_attachment"),
                    _ => None,
                },
            )
        }
        Some("assistant") => {
            require_record_version(&v)?;
            let Some(message) = v.get("message") else {
                return Ok(None);
            };
            if message.get("role").and_then(Value::as_str) != Some("assistant")
                || message.get("stop_reason").and_then(Value::as_str) != Some("tool_use")
            {
                return Ok(None);
            }
            let Some(blocks) = message
                .get("content")
                .and_then(Value::as_array)
                .filter(|b| b.len() == 1)
            else {
                return Ok(None);
            };
            let block = &blocks[0];
            if block.get("type").and_then(Value::as_str) != Some("tool_use")
                || block.get("name").and_then(Value::as_str) != Some("Bash")
            {
                return Ok(None);
            }
            label(field(block, "id")?)?;
            field(
                block.get("input").ok_or("unsupported_host_tool_input")?,
                "command",
            )?;
            Ok(Some("bash_request"))
        }
        _ => Ok(None),
    }
}

use serde_json::{json, Value};

use crate::handlers::feedback::{build_agent_feedback_stats_payload, record_agent_feedback_from_value};
use crate::handlers::health::{build_digest, build_health_payload};
use crate::handlers::mutate::{grant_permission, list_permissions, revoke_permission};
use crate::handlers::recall::{execute_semantic_recall, execute_unified_recall, RecallContext};
use crate::handlers::SourceIdentity;
use crate::state::RuntimeState;

use super::{arg_i64, arg_str, arg_usize, enforce_client_permission, fetch_last_call};

fn require_arg<'a>(args: &'a Value, keys: &[&str], label: &str) -> Result<&'a str, String> {
    arg_str(args, keys).ok_or_else(|| format!("Missing required argument: {label}"))
}

pub(crate) async fn mcp_dispatch(
    state: &RuntimeState, caller_id: Option<i64>, tool_name: &str, args: &Value, source: Option<&SourceIdentity>,
) -> Result<Value, String> {
    if state.team_mode && caller_id.is_none() {
        return Err("Team mode MCP calls require a caller-scoped ctx_ API key".to_string());
    }
    enforce_client_permission(state, caller_id, tool_name, args, source).await?;
    let owner_id = if state.team_mode { caller_id.unwrap_or_default() } else { 0 };
    match tool_name {
        "cortex_health" => Ok(build_health_payload(state, false).await),
        "cortex_digest" => {
            let conn = state.db_read.lock().await;
            build_digest(&conn)
        }
        "cortex_recall" | "cortex_peek" => {
            let query = require_arg(args, &["query", "q"], "query")?;
            let budget = arg_usize(args, &["budget", "b"]).unwrap_or(if tool_name == "cortex_peek" { 0 } else { 320 });
            let k = arg_usize(args, &["k", "limit"]).unwrap_or(10);
            let agent = arg_str(args, &["agent", "source_agent"]).unwrap_or_else(|| source.map(|identity| identity.agent.as_str()).unwrap_or("mcp"));
            let ctx = RecallContext::from_caller(caller_id, state);
            execute_unified_recall(state, query, budget, k, agent, &ctx, arg_str(args, &["source_prefix", "sourcePrefix"])).await
        }
        "cortex_semantic_recall" => {
            let query = require_arg(args, &["query", "q"], "query")?;
            let k = arg_usize(args, &["k", "limit"]).unwrap_or(10);
            let budget = arg_usize(args, &["budget", "b"]).unwrap_or(200);
            let agent = arg_str(args, &["agent", "source_agent"]).unwrap_or("mcp");
            let ctx = RecallContext::from_caller(caller_id, state);
            execute_semantic_recall(state, query, budget, k, agent, &ctx, arg_str(args, &["source_prefix", "sourcePrefix"])).await
        }
        "cortex_agent_feedback_record" => {
            let conn = state.db.lock().await;
            record_agent_feedback_from_value(&conn, owner_id, args, source.map(|identity| identity.agent.as_str()).unwrap_or("mcp"))
        }
        "cortex_agent_feedback_stats" => {
            let conn = state.db_read.lock().await;
            build_agent_feedback_stats_payload(
                &conn,
                owner_id,
                arg_i64(args, &["horizonDays", "horizon_days"]).unwrap_or(30),
                arg_usize(args, &["limit"]).unwrap_or(400),
                arg_str(args, &["taskClass", "task_class"]),
                arg_str(args, &["agent", "source_agent"]),
            )
        }
        "cortex_permissions_list" => {
            let conn = state.db_read.lock().await;
            list_permissions(&conn, owner_id).map(|permissions| json!({"permissions":permissions}))
        }
        "cortex_permissions_grant" => {
            let client = require_arg(args, &["client", "client_id"], "client")?;
            let permission = require_arg(args, &["permission"], "permission")?;
            let scope = arg_str(args, &["scope"]).unwrap_or("*");
            let conn = state.db.lock().await;
            grant_permission(&conn, owner_id, client, permission, scope, arg_str(args, &["grantedBy", "granted_by"]).unwrap_or("mcp"))?;
            Ok(json!({"granted":true,"client":client,"permission":permission,"scope":scope}))
        }
        "cortex_permissions_revoke" => {
            let client = require_arg(args, &["client", "client_id"], "client")?;
            let permission = require_arg(args, &["permission"], "permission")?;
            let scope = arg_str(args, &["scope"]).unwrap_or("*");
            let conn = state.db.lock().await;
            revoke_permission(&conn, owner_id, client, permission, scope).map(|revoked| json!({"revoked":revoked}))
        }
        "cortex_lastCall" => {
            let conn = state.db_read.lock().await;
            fetch_last_call(&conn, arg_str(args, &["kind"]), arg_str(args, &["agent", "source_agent"]), &RecallContext::from_caller(caller_id, state))
        }
        _ => Ok(json!({"ok":true,"tool":tool_name})),
    }
}

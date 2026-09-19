use serde_json::{Value, json};
/// Model-facing tools: the eight semantic operations. Legacy tool names stay
/// callable (mapped in `operations::Operation::from_tool_name`) but are not
/// advertised, so hosts that serialize every schema into the prompt pay for
/// eight descriptions, not twenty-six.
pub fn mcp_tools() -> Vec<Value> {
    cortex_kernel::handlers::operations::tool_schemas()
}

/// Operator/administrative tools that still have a real dispatcher path.
/// Names without an implementation must not appear here — discovery is
/// either advertised (tools/list) or reachable-by-name-and-working, never
/// a schema for a dead route.
pub fn legacy_mcp_tools() -> Vec<Value> {
    vec![
        json!({"name":"cortex_boot","description":
"Legacy alias of cortex_orient. Situation brief for a session or task. Prefer cortex_orient in new clients.",
"inputSchema":{"type":"object","properties":{"task":{"type":"string","description":"What you are trying to do"},"thread":{"type":"string"},"budget":{"type":"number"},"evidence":{"type":"string","enum":["brief","support","exact"]}}}}),
        json!({"name":"cortex_peek","description":
"Lightweight check: returns source names and relevance scores only (no excerpts). Prefer cortex_query with a small budget.",
"inputSchema":{"type":"object","properties":{"query":{"type":"string","description":"Search query text"},"limit":{"type":"number",
"description":"Max results (default 10)"}},"required":["query"]}}),
        json!({"name":"cortex_recall","description":
"Clock-Quorum Recall over write, truth, task, and history evidence. Prefer cortex_query with a profile.",
"inputSchema":{"type":"object","properties":{"query":{"type":"string","description":"Search query text"},"budget":{"type":
"number"},"k":{"type":"number"},"agent":{"type":"string"}},"required":["query"]}}),
        json!({"name":"cortex_semantic_recall",
"description":"Named Clock-Quorum Recall surface. Same engine as cortex_recall; no embedding model.","inputSchema":{
"type":"object","properties":{"query":{"type":"string","description":"Search query text"},"budget":{"type":"number"},
"k":{"type":"number"},"agent":{"type":"string"}},"required":["query"]}}),
        json!({"name":"cortex_store",
"description":"Store a decision with conflict detection. Prefer cortex_commit.","inputSchema":{"type":"object","properties":{
"decision":{"type":"string"},"context":{"type":"string"},"type":{"type":"string"},
"source_agent":{"type":"string"},"confidence":{"type":"number"},
"ttl_seconds":{"type":"number"},
"retention_class":{"type":"string","enum":["durable","operational","audit","ephemeral"]}},"required":[
"decision"]}}),
        json!({"name":"cortex_unfold","description":
        "Expand selected memory/decision sources to full text. Prefer cortex_expand with a View alias + receipt.",
        "inputSchema":{"type":"object","properties":{"sources":{"type":"array","items":{"type":"string"}}},"required":["sources"]}}
        ),
        json!({"name":"cortex_health","description":
"Check Cortex system health: DB stats, memory counts.","inputSchema":{"type":"object","properties":{}}}),
        json!({"name":
"cortex_digest","description":
"Daily health digest: memory counts, activity, top recalls. Use to check if the brain is compounding.",
"inputSchema":{"type":"object","properties":{}}}),
        json!({"name":"cortex_agent_feedback_record","description":
"Record task outcome telemetry. Prefer cortex_feedback.","inputSchema":{"type":"object","properties":{
"agent":{"type":"string"},"taskClass":{"type":"string"},"outcome":{"type":"string","enum":["success","partial","failure"]},
"outcomeScore":{"type":"number"},"qualityScore":{"type":"number"},"latencyMs":{"type":"number"},
"retries":{"type":"number"},"tokensUsed":{"type":"number"},"memorySources":{"type":"array","items":{"type":"string"}},
"notes":{"type":"string"}},"required":["outcome"]}}),
        json!({"name":"cortex_agent_feedback_stats","description":
"Summarize reliability trends from recorded agent outcome telemetry.","inputSchema":{"type":"object","properties":{
"horizonDays":{"type":"number"},"limit":{"type":"number"},"taskClass":{"type":"string"},"agent":{"type":"string"}}}}),
        json!({"name":"cortex_conflicts_resolve","description":
"Resolve a conflict by selecting a winner. Prefer cortex_resolve.","inputSchema":{"type":"object","properties":{
"winnerId":{"type":"number"},"keepId":{"type":"number"},
"action":{"type":"string","enum":["keep","merge","archive"]},
"supersededId":{"type":"number"},"loserId":{"type":"number"},
"conflictId":{"type":"string"},
"notes":{"type":"string"},"resolvedBy":{"type":"string"}},"required":["action"]}}),
        json!({"name":"cortex_focus_start","description":
"Start a focus session (context checkpoint). Prefer cortex_checkpoint.","inputSchema":{"type":"object","properties":{
"label":{"type":"string"},"agent":{"type":"string"}},"required":["label"]}}),
        json!({"name":"cortex_focus_end","description":
"End a focus session and consolidate. Prefer cortex_checkpoint.","inputSchema":{"type":"object","properties":{
"label":{"type":"string"},"agent":{"type":"string"}},"required":["label"]}}),
        json!({
"name":"cortex_permissions_list","description":"List MCP client permission grants for the current owner scope.","inputSchema":{
"type":"object","properties":{}}}),
        json!({"name":"cortex_permissions_grant","description":
"Grant a client permission (`read`, `write`, `admin`) for a scope (`*` by default).","inputSchema":{"type":"object","properties":{
"client":{"type":"string"},"permission":{"type":"string","enum":["read","write","admin"]},
"scope":{"type":"string"}},"required":["client","permission"]}}),
        json!({"name":"cortex_permissions_revoke","description":
"Revoke a previously granted client permission for a scope.","inputSchema":{"type":"object","properties":{"client":{"type":
"string"},"permission":{"type":"string","enum":["read","write","admin"]},"scope":{"type":"string"}},"required":["client","permission"]}}),
        json!
({"name":"cortex_lastCall","description":
"Fetch the latest memory, decision, or event added to Cortex, with optional kind/agent filters.","inputSchema":{"type":"object",
"properties":{"kind":{"type":"string"},"agent":{"type":"string"}}}}),
    ]
}

/// Historical tool names that no longer have a dispatcher. Kept so capabilities
/// and suggestions can explain the replacement instead of a vague unknown tool.
pub fn removed_tool_replacements() -> Value {
    json!({
        "cortex_boot_audit": "cortex_orient",
        "cortex_diary": "cortex_checkpoint",
        "cortex_forget": "cortex_resolve or retention_class on cortex_commit",
        "cortex_reconnect": "restart the local cortex mcp process; no daemon session exists",
        "cortex_recall_policy_explain": "cortex_query evidence=support",
        "cortex_focus_status": "cortex_checkpoint action=status",
        "cortex_conflicts_list": "cortex_query profile=conflicts",
        "cortex_conflicts_get": "cortex_expand or cortex_query profile=conflicts",
        "cortex_consensus_promote": "cortex_resolve",
        "cortex_memory_decay_run": "cortex maintain",
        "cortex_eval_run": "cortex eval CLI"
    })
}

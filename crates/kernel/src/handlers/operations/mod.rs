//! The eight semantic operations: capabilities, orient, query, expand,
//! commit, checkpoint, resolve, feedback. Transport-agnostic: MCP, HTTP and
//! the library call `dispatch` with the same arguments. Legacy tool names map
//! onto these operations during the transition.

mod bridge;
mod checkpoint;
mod closure;
mod commit;
mod compare;
mod evidence;
mod expand;
mod lens;
mod resolve;

pub mod recipes;
mod view;

pub use closure::{ClosureItem, DependencyRole, add_relation, close_revision};
pub use view::{Card, Coverage, PresenceInputs, View};

pub(crate) use crate::protocol::{arg_bool, arg_i64, arg_list, arg_str, arg_usize};
pub(crate) use bridge::{attach_assembly_evidence, attach_observation_evidence};
pub(crate) use lens::run_lens;

use crate::db::records;
use crate::lens::{EvidenceDepth, LensProfile, NeedFrame};
use crate::protocol::{Envelope, KNOWN_OPERATIONS, ResponseStatus};
use crate::state::RuntimeState;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Capabilities,
    Orient,
    Query,
    Expand,
    Commit,
    Checkpoint,
    Resolve,
    Feedback,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Capabilities => "capabilities",
            Self::Orient => "orient",
            Self::Query => "query",
            Self::Expand => "expand",
            Self::Commit => "commit",
            Self::Checkpoint => "checkpoint",
            Self::Resolve => "resolve",
            Self::Feedback => "feedback",
        }
    }
    pub fn tool_name(self) -> String {
        format!("cortex_{}", self.as_str())
    }
    /// Semantic operation for a tool name, including legacy aliases.
    pub fn from_tool_name(name: &str) -> Option<Self> {
        Some(match name {
            "cortex_capabilities" | "capabilities" | "cortex_health" => Self::Capabilities,
            "cortex_orient" | "orient" | "cortex_boot" => Self::Orient,
            "cortex_query"
            | "query"
            | "cortex_recall"
            | "cortex_peek"
            | "cortex_semantic_recall" => Self::Query,
            "cortex_expand" | "expand" | "cortex_unfold" => Self::Expand,
            "cortex_commit" | "commit" | "cortex_store" => Self::Commit,
            "cortex_checkpoint" | "checkpoint" | "cortex_focus_start" | "cortex_focus_end" => {
                Self::Checkpoint
            }
            "cortex_resolve" | "resolve" | "cortex_conflicts_resolve" => Self::Resolve,
            "cortex_feedback" | "feedback" | "cortex_agent_feedback_record" => Self::Feedback,
            _ => return None,
        })
    }
    pub fn is_write(self) -> bool {
        matches!(
            self,
            Self::Commit | Self::Checkpoint | Self::Resolve | Self::Feedback
        )
    }
    pub fn requires_authority(self) -> bool {
        matches!(self, Self::Resolve)
    }
}

/// Model-facing tool schemas: short, workflow-oriented, expert controls kept
/// out of the description.
pub fn tool_schemas() -> Vec<Value> {
    let profiles: Vec<&str> = LensProfile::ALL.iter().map(|p| p.as_str()).collect();
    let mut schemas = vec![
        json!({"name":"cortex_capabilities","description":"What this brain can do: operations, Lens profiles, response statuses, epochs and adapter capabilities. Cache by version.","inputSchema":{"type":"object","properties":{}}}),
        json!({"name":"cortex_orient","description":"Situation brief for a task or Thread: constraints, known facts with their limits, failed attempts, open work, unresolved conflicts, evidence handles. Surfaces attributed observations and, after an explicit route rebuild, evidence-closed assembly bundles. Call once when you start.","inputSchema":{"type":"object","properties":{"task":{"type":"string","description":"What you are trying to do"},"thread":{"type":"string","description":"Thread id or label (optional)"},"paths":{"type":"array","items":{"type":"string"},"description":"Project roots for this task"},"cwd":{"type":"string","description":"Working directory treated as a project root"},"budget":{"type":"number","description":"Output budget in bytes (default 2000)"},"evidence":{"type":"string","enum":["brief","support","exact"]},"observation_scope":{"type":"string","description":"Observation and assembly scope (default project)"},"observations":{"type":"boolean","description":"Include attributed observation hits (default true)"},"assemblies":{"type":"boolean","description":"Include compiled assembly bundles when cue routes are enabled (default true)"}}}}),
        json!({"name":"cortex_query","description":"Ask memory a question with a profile: answer, changes, attempts, procedures, conflicts, uncertainty, compare, history, audit, map. Returns Cards with epistemic status, applicability and expansion handles; leads are separate from supported answers. Attributed observations and compiled assemblies appear in separate sections, never as Cards.","inputSchema":{"type":"object","properties":{"need":{"type":"string","description":"The question or need"},"profile":{"type":"string","enum":profiles},"needs":{"type":"array","items":{"type":"string"},"description":"Typed needs: current_constraints, open_obligations, last_verified_outcome, failed_attempts, conflicts, as_known, changes, procedures"},"thread":{"type":"string"},"time":{"type":"string","description":"valid_at instant for historical views"},"budget":{"type":"number"},"evidence":{"type":"string","enum":["brief","support","exact"]},"paths":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string"},"symbols":{"type":"array","items":{"type":"string"}},"observation_scope":{"type":"string"},"observations":{"type":"boolean"},"assemblies":{"type":"boolean"}},"required":["need"]}}),
        json!({"name":"cortex_expand","description":"Exact source for a Card alias (m1, m2 …) from a View you received, a logical reference, an observation ref (obs:<source_id>), an assembly (asm:<id>), or a revision (rev:<id>). Aliases are valid only with their receipt.","inputSchema":{"type":"object","properties":{"alias":{"type":"string"},"receipt":{"type":"string","description":"receipt id from the View"},"reference":{"type":"string","description":"decision::12, obs:<source_id>, asm:<id>, or rev:<id>"}}}}),
        json!({"name":"cortex_commit","description":"Deposit one or more entries (decisions, observations, attempts) atomically with an idempotency key. Optional evidence[] cites obs:<source_id> refs (promoted_from); unknown cites fail closed. Returns a Receipt with the durability vector; return_view answers the write with a fresh View.","inputSchema":{"type":"object","properties":{"entries":{"type":"array","items":{"type":"object","properties":{"local_id":{"type":"string"},"text":{"type":"string"},"kind":{"type":"string"},"context":{"type":"string"},"paths":{"type":"array","items":{"type":"string"}},"thread":{"type":"string"}},"required":["text"]}},"decision":{"type":"string","description":"Shorthand for a single decision entry"},"paths":{"type":"array","items":{"type":"string"},"description":"Project roots recorded on this Deposit"},"cwd":{"type":"string"},"thread":{"type":"string"},"idempotency_key":{"type":"string"},"return_view":{"type":"boolean"},"retention_class":{"type":"string","enum":["durable","operational","audit","ephemeral"]},"evidence":{"type":"array","items":{"type":"string"},"description":"obs:<source_id> citations promoted into this Deposit"}}}}),
        json!({"name":"cortex_checkpoint","description":"Durable Thread state: checkpoint (goal, state), obligations (create / transition / verify with a checker predicate on an artifact / revalidate on a new artifact), attempts (inputs, artifacts, exit status, failure), or status. A successor or post-compaction context resumes from this, not from your transcript.","inputSchema":{"type":"object","properties":{"thread":{"type":"string"},"action":{"type":"string","enum":["checkpoint","status","obligation","transition","verify","revalidate","attempt"]},"goal":{"type":"string"},"state":{"type":"object"},"note":{"type":"string"},"title":{"type":"string"},"predicate":{},"obligation":{"type":"string"},"to":{"type":"string"},"artifact":{"type":"string"},"checker":{"type":"string"},"passed":{"type":"boolean"},"attempt":{"type":"object"}},"required":["thread"]}}),
        json!({"name":"cortex_resolve","description":"Resolve competing heads of a record with authority and rationale. Creates a resolution revision; rejected evidence is kept.","inputSchema":{"type":"object","properties":{"record":{"type":"string"},"considered":{"type":"array","items":{"type":"string"}},"rationale":{"type":"string"},"body":{"type":"object"},"keepId":{"type":"number","description":"Legacy conflict resolution: decision id to keep"},"action":{"type":"string","description":"Legacy: keep|merge|archive"}}}}),
        json!({"name":"cortex_feedback","description":"Report a task outcome (success|partial|failure) and which memory sources were actually used, so usefulness statistics stay separate from truth.","inputSchema":{"type":"object","properties":{"outcome":{"type":"string","enum":["success","partial","failure"]},"taskClass":{"type":"string"},"memorySources":{"type":"array","items":{"type":"string"}},"qualityScore":{"type":"number"},"notes":{"type":"string"}},"required":["outcome"]}}),
    ];
    // 2026-07-28 tool metadata: every result carries a JSON `structuredContent`
    // object, reads are side-effect free, and writes are additive-only
    // (rejected evidence is kept; nothing is destroyed) but not idempotent
    // without an explicit idempotency key. Every tool is closed-world: a
    // memory brain touches only its own store, never external entities.
    for schema in &mut schemas {
        let (title, annotations) = match schema.get("name").and_then(Value::as_str) {
            Some("cortex_capabilities") => (
                "Capabilities",
                json!({"readOnlyHint": true, "openWorldHint": false}),
            ),
            Some("cortex_orient") => (
                "Orient",
                json!({"readOnlyHint": true, "openWorldHint": false}),
            ),
            Some("cortex_query") => (
                "Query memory",
                json!({"readOnlyHint": true, "openWorldHint": false}),
            ),
            Some("cortex_expand") => (
                "Expand evidence",
                json!({"readOnlyHint": true, "openWorldHint": false}),
            ),
            Some("cortex_commit") => (
                "Commit",
                json!({"destructiveHint": false, "idempotentHint": false, "openWorldHint": false}),
            ),
            Some("cortex_checkpoint") => (
                "Checkpoint",
                json!({"destructiveHint": false, "idempotentHint": false, "openWorldHint": false}),
            ),
            Some("cortex_resolve") => (
                "Resolve",
                json!({"destructiveHint": false, "idempotentHint": false, "openWorldHint": false}),
            ),
            Some("cortex_feedback") => (
                "Feedback",
                json!({"destructiveHint": false, "idempotentHint": false, "openWorldHint": false}),
            ),
            _ => continue,
        };
        if let Some(map) = schema.as_object_mut() {
            map.insert("title".to_string(), Value::String(title.into()));
            map.insert("outputSchema".to_string(), json!({"type": "object"}));
            map.insert("annotations".to_string(), annotations);
        }
    }
    schemas
}

pub async fn capabilities(cx: &asupersync::Cx, state: &RuntimeState) -> Result<Value, String> {
    let (brain_id, restore_epoch, policy_epoch) = {
        let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
        records::brain_epochs(&conn)
    };
    Ok(
        json!({"operations":KNOWN_OPERATIONS,"profiles":LensProfile::ALL.iter().map(|p| p.as_str()).collect::<Vec<_>>(),"needs":["current_constraints","open_obligations","last_verified_outcome","failed_attempts","conflicts","as_known","changes","procedures","unverified","compare","audit","map","answer","recipe:<name>"],"statuses":["ok","partial","no_match","ambiguous","needs_more_budget","projection_pending","resnapshot_required","unavailable","denied","outcome_unknown","invalid_request"],"evidence":["brief","support","exact"],"brain":{"id":brain_id,"restore_epoch":restore_epoch,"policy_epoch":policy_epoch,"team_mode":state.team_mode},"durability_profile":crate::db::DurabilityProfile::from_env().as_str(),"adapter_capabilities":crate::adapter::CapabilityManifest::native().to_json(),"adapters":{"claude-code-plugin":crate::adapter::CapabilityManifest::claude_code_plugin().to_json(),"tools_only":crate::adapter::CapabilityManifest::tools_only("mcp-tools").to_json()},"hook_decisions":["NOOP","DELIVER","QUERY_REQUIRED","PROJECTION_PENDING","UNAVAILABLE"],"zero_token_actions":crate::adapter::ZERO_TOKEN_ACTIONS,"protocol_version":"1","legacy_tool_aliases":{"cortex_boot":"orient","cortex_recall":"query","cortex_peek":"query","cortex_semantic_recall":"query","cortex_store":"commit","cortex_unfold":"expand","cortex_conflicts_resolve":"resolve","cortex_focus_start":"checkpoint","cortex_focus_end":"checkpoint","cortex_agent_feedback_record":"feedback","cortex_health":"capabilities"},"observation_bridge":{"query_orient_field":"observations","scope_arg":"observation_scope","opt_out":"observations=false","expand_refs":["obs:<source_id>"],"epistemic":"attributed_observation","promote":{"commit_arg":"evidence","values":["obs:<source_id>"],"relationship":"promoted_from","note":"Explicit Deposit citation only; capture never auto-promotes."},"note":"Attributed observations are not CQR Cards and never change admission. Caller paths keep path-scoped sources in that repository; the project bucket stays unscoped. Library lens attaches the same observations field beside results."},"assembly_bridge":{"query_orient_field":"assemblies","scope_arg":"observation_scope","opt_out":"assemblies=false","expand_refs":["asm:<assembly_id>","rev:<revision_id>"],"enabled_by":"rebuild_assembly_routes","note":"Compiled bundles stay off until cue routes are rebuilt. Caller paths keep path-scoped bundles in that repository; a default project compile does not leak them. Library lens attaches the same assemblies field beside results. Ranking does not change CQR Cards or epistemic status."},"removed_tools":{"cortex_boot_audit":"cortex_orient","cortex_diary":"cortex_checkpoint","cortex_forget":"cortex_resolve or retention_class on cortex_commit","cortex_reconnect":"restart the local cortex mcp process","cortex_recall_policy_explain":"cortex_query evidence=support","cortex_focus_status":"cortex_checkpoint action=status","cortex_conflicts_list":"cortex_query profile=conflicts","cortex_conflicts_get":"cortex_expand","cortex_consensus_promote":"cortex_resolve","cortex_memory_decay_run":"cortex maintain CLI","cortex_eval_run":"cortex eval CLI"}}),
    )
}

pub struct Caller<'a> {
    pub owner_id: Option<i64>,
    pub agent: &'a str,
    pub principal: String,
}

pub(super) fn invalid_request(error: impl Into<String>) -> Value {
    json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": error.into()})
}

pub(super) fn invalid_field(error: impl Into<String>, field: &str) -> Value {
    json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": error.into(), "field": field})
}

pub(super) fn unavailable(error: impl ToString) -> Value {
    json!({"status": ResponseStatus::Unavailable.as_str(), "error": error.to_string()})
}

pub(super) fn no_match(error: impl Into<String>) -> Value {
    json!({"status": ResponseStatus::NoMatch.as_str(), "error": error.into()})
}

pub async fn dispatch(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: Caller<'_>,
    op: Operation,
    args: &Value,
) -> Result<Value, String> {
    match op {
        Operation::Capabilities => capabilities(cx, state).await,
        Operation::Orient | Operation::Query => read_view(cx, state, &caller, op, args).await,
        Operation::Expand => expand::expand(cx, state, &caller, args).await,
        Operation::Commit => commit::commit(cx, state, &caller, args).await,
        Operation::Checkpoint => checkpoint::checkpoint(cx, state, &caller, args).await,
        Operation::Resolve => resolve::resolve(cx, state, &caller, args).await,
        Operation::Feedback => feedback(cx, state, &caller, args).await,
    }
}

async fn read_view(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    op: Operation,
    args: &Value,
) -> Result<Value, String> {
    let profile = lens::profile_for(op, args);
    if profile == LensProfile::Compare {
        return compare::compare(cx, state, caller, args).await;
    }
    if let Some(recipe) = arg_list(args, &["needs"])
        .iter()
        .find_map(|n| n.strip_prefix("recipe:").map(str::to_string))
        .or_else(|| arg_str(args, &["recipe"]).map(str::to_string))
    {
        return recipes::run_recipe(cx, state, caller, &recipe, args).await;
    }
    let text = arg_str(args, &["need", "task", "query", "q"]).unwrap_or("");
    let needs = arg_list(args, &["needs"]);
    let evidence = EvidenceDepth::parse(arg_str(args, &["evidence"]));
    let frame = NeedFrame::build(profile, text, &needs, evidence);
    if frame.text.is_empty()
        && frame.profile != LensProfile::Orient
        && frame.profile != LensProfile::Map
    {
        return Ok(invalid_field("need is required", "need"));
    }
    let budget = arg_usize(args, &["budget"]).unwrap_or(2000);
    let view = run_lens(cx, state, caller, &frame, args, budget).await?;
    let mut out = view.to_json();
    if let Some(thread) = arg_str(args, &["thread"]) {
        let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
        if let Ok(summary) = crate::db::threads::thread_summary(&conn, thread) {
            out["continuation"] = lens::continuation_from(&summary, &view);
            out["thread"] = summary;
        }
    }
    bridge::attach_observation_evidence(
        cx,
        state,
        text,
        args,
        &mut out,
        matches!(op, Operation::Orient),
    )
    .await;
    bridge::attach_assembly_evidence(cx, state, text, args, &mut out).await;
    Ok(out)
}

async fn feedback(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    let owner = if state.team_mode {
        caller
            .owner_id
            .ok_or_else(|| "Team mode requires a local owner".to_string())?
    } else {
        0
    };
    let sp = crate::db::SqliteSavepoint::enter(&*conn, "feedback_op").map_err(|e| e.to_string())?;
    let mut out = crate::handlers::feedback::record_agent_feedback_from_value(
        &conn,
        owner,
        args,
        caller.agent,
    )?;
    let receipt =
        arg_str(args, &["receipt", "prior_view_receipt", "receipt_id"]).map(str::to_string);
    let exposed = receipt
        .as_deref()
        .map(|r| crate::db::feedback_ledger::exposed_from_receipt(&conn, r))
        .transpose()?
        .unwrap_or_default();
    let used = arg_list(args, &["memorySources", "memory_sources", "used"]);
    let fb = crate::db::feedback_ledger::OutcomeFeedback {
        scope: arg_str(args, &["scope"]).unwrap_or("default").to_string(),
        task_family: arg_str(args, &["taskClass", "task_class", "task_family"])
            .unwrap_or("general")
            .to_string(),
        task: arg_str(args, &["task", "notes"]).map(str::to_string),
        prior_view_receipt: receipt,
        selected_action: arg_str(args, &["selected_action", "action"]).map(str::to_string),
        outcome: out["outcome"].as_str().unwrap_or("partial").to_string(),
        exposed,
        used,
        harmful_reuse: arg_bool(args, &["harmful_reuse", "harmfulReuse"]).unwrap_or(false),
        wrong_scope: arg_bool(args, &["wrong_scope", "wrongScope"]).unwrap_or(false),
        agent: caller.agent.to_string(),
    };
    let id = crate::db::feedback_ledger::record(&conn, &fb)?;
    sp.release().map_err(|e| e.to_string())?;
    out["ledger"] = json!({"id": id, "exposed": fb.exposed.len(), "used": fb.used.len(), "scope": fb.scope, "task_family": fb.task_family});
    Ok(out)
}

/// Validate an envelope-shaped request from a raw package caller.
pub fn validate_envelope(value: &Value) -> Result<Envelope, String> {
    let envelope: Envelope =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid envelope: {e}"))?;
    envelope.validate().map_err(|e| e.to_string())?;
    Ok(envelope)
}

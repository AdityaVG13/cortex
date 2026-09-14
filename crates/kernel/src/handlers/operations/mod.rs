//! The eight semantic operations: capabilities, orient, query, expand,
//! commit, checkpoint, resolve, feedback. Transport-agnostic: MCP, HTTP and
//! the library call `dispatch` with the same arguments. Legacy tool names map
//! onto these operations during the transition.

mod closure;

pub mod recipes;
mod view;

pub use closure::{add_relation, close_revision, ClosureItem, DependencyRole};
pub use view::{Card, Coverage, PresenceInputs, View};

use crate::db::records;
use crate::handlers::recall::{execute_unified_recall, unfold_source, RecallContext};
use crate::lens::{EvidenceDepth, LensProfile, NeedFrame};
use crate::protocol::{Envelope, ResponseStatus, KNOWN_OPERATIONS};
use crate::state::RuntimeState;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::collections::BTreeSet;

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

fn arg_str<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| args.get(k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
/// MCP JSON-RPC clients send integers as i64, u64, whole floats, or decimal
/// strings. `as_i64()` alone dropped `"100"` / `100.0`, so `budget` silently
/// fell through to the 2000-byte default.
fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| {
            value.as_f64().and_then(|x| {
                (x.is_finite() && x.fract() == 0.0).then_some(x as i64)
            })
        })
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}
fn arg_usize(args: &Value, keys: &[&str]) -> Option<usize> {
    keys.iter()
        .find_map(|k| args.get(*k).and_then(json_i64).and_then(|v| usize::try_from(v).ok()))
}
fn arg_bool(args: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|k| args.get(k).and_then(Value::as_bool))
}
fn arg_list(args: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|k| args.get(k))
        .map(|v| match v {
            Value::Array(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            Value::String(s) => s
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        })
        .unwrap_or_default()
}

fn looks_like_fs_path(s: &str) -> bool {
    s.contains('/') || s.contains('\\')
}

fn cwd_root(args: &Value) -> Option<String> {
    arg_str(args, &["cwd", "cwd_path", "working_directory"])
        .filter(|s| looks_like_fs_path(s))
        .map(str::to_string)
}

fn commit_paths(args: &Value, entry: &Value) -> Vec<String> {
    let mut paths = arg_list(args, &["paths"]);
    paths.extend(arg_list(entry, &["paths"]));
    for src in [args, entry] {
        if let Some(cwd) = cwd_root(src) {
            if !paths.iter().any(|p| p == &cwd) {
                paths.push(cwd);
            }
        }
    }
    paths
}

/// Model-facing tool schemas: short, workflow-oriented, expert controls kept
/// out of the description.
pub fn tool_schemas() -> Vec<Value> {
    let profiles: Vec<&str> = LensProfile::ALL.iter().map(|p| p.as_str()).collect();
    vec![
        json!({"name":"cortex_capabilities","description":"What this brain can do: operations, Lens profiles, response statuses, epochs and adapter capabilities. Cache by version.","inputSchema":{"type":"object","properties":{}}}),
        json!({"name":"cortex_orient","description":"Situation brief for a task or Thread: constraints, known facts with their limits, failed attempts, open work, unresolved conflicts, evidence handles. Surfaces attributed observations and, after an explicit route rebuild, evidence-closed assembly bundles. Call once when you start.","inputSchema":{"type":"object","properties":{"task":{"type":"string","description":"What you are trying to do"},"thread":{"type":"string","description":"Thread id or label (optional)"},"paths":{"type":"array","items":{"type":"string"},"description":"Project roots for this task"},"cwd":{"type":"string","description":"Working directory treated as a project root"},"budget":{"type":"number","description":"Output budget in bytes (default 2000)"},"evidence":{"type":"string","enum":["brief","support","exact"]},"observation_scope":{"type":"string","description":"Observation and assembly scope (default project)"},"observations":{"type":"boolean","description":"Include attributed observation hits (default true)"},"assemblies":{"type":"boolean","description":"Include compiled assembly bundles when cue routes are enabled (default true)"}}}}),
        json!({"name":"cortex_query","description":"Ask memory a question with a profile: answer, changes, attempts, procedures, conflicts, uncertainty, compare, history, audit, map. Returns Cards with epistemic status, applicability and expansion handles; leads are separate from supported answers. Attributed observations and compiled assemblies appear in separate sections, never as Cards.","inputSchema":{"type":"object","properties":{"need":{"type":"string","description":"The question or need"},"profile":{"type":"string","enum":profiles},"needs":{"type":"array","items":{"type":"string"},"description":"Typed needs: current_constraints, open_obligations, last_verified_outcome, failed_attempts, conflicts, as_known, changes, procedures"},"thread":{"type":"string"},"time":{"type":"string","description":"valid_at instant for historical views"},"budget":{"type":"number"},"evidence":{"type":"string","enum":["brief","support","exact"]},"paths":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string"},"symbols":{"type":"array","items":{"type":"string"}},"observation_scope":{"type":"string"},"observations":{"type":"boolean"},"assemblies":{"type":"boolean"}},"required":["need"]}}),
        json!({"name":"cortex_expand","description":"Exact source for a Card alias (m1, m2 …) from a View you received, a logical reference, an observation ref (obs:<source_id>), an assembly (asm:<id>), or a revision (rev:<id>). Aliases are valid only with their receipt.","inputSchema":{"type":"object","properties":{"alias":{"type":"string"},"receipt":{"type":"string","description":"receipt id from the View"},"reference":{"type":"string","description":"decision::12, obs:<source_id>, asm:<id>, or rev:<id>"}}}}),
        json!({"name":"cortex_commit","description":"Deposit one or more entries (decisions, observations, attempts) atomically with an idempotency key. Optional evidence[] cites obs:<source_id> refs (promoted_from); unknown cites fail closed. Returns a Receipt with the durability vector; return_view answers the write with a fresh View.","inputSchema":{"type":"object","properties":{"entries":{"type":"array","items":{"type":"object","properties":{"local_id":{"type":"string"},"text":{"type":"string"},"kind":{"type":"string"},"context":{"type":"string"},"paths":{"type":"array","items":{"type":"string"}},"thread":{"type":"string"}},"required":["text"]}},"decision":{"type":"string","description":"Shorthand for a single decision entry"},"paths":{"type":"array","items":{"type":"string"},"description":"Project roots recorded on this Deposit"},"cwd":{"type":"string"},"thread":{"type":"string"},"idempotency_key":{"type":"string"},"return_view":{"type":"boolean"},"retention_class":{"type":"string","enum":["durable","operational","audit","ephemeral"]},"evidence":{"type":"array","items":{"type":"string"},"description":"obs:<source_id> citations promoted into this Deposit"}}}}),
        json!({"name":"cortex_checkpoint","description":"Durable Thread state: checkpoint (goal, state), obligations (create / transition / verify with a checker predicate on an artifact / revalidate on a new artifact), attempts (inputs, artifacts, exit status, failure), or status. A successor or post-compaction context resumes from this, not from your transcript.","inputSchema":{"type":"object","properties":{"thread":{"type":"string"},"action":{"type":"string","enum":["checkpoint","status","obligation","transition","verify","revalidate","attempt"]},"goal":{"type":"string"},"state":{"type":"object"},"note":{"type":"string"},"title":{"type":"string"},"predicate":{},"obligation":{"type":"string"},"to":{"type":"string"},"artifact":{"type":"string"},"checker":{"type":"string"},"passed":{"type":"boolean"},"attempt":{"type":"object"}},"required":["thread"]}}),
        json!({"name":"cortex_resolve","description":"Resolve competing heads of a record with authority and rationale. Creates a resolution revision; rejected evidence is kept.","inputSchema":{"type":"object","properties":{"record":{"type":"string"},"considered":{"type":"array","items":{"type":"string"}},"rationale":{"type":"string"},"body":{"type":"object"},"keepId":{"type":"number","description":"Legacy conflict resolution: decision id to keep"},"action":{"type":"string","description":"Legacy: keep|merge|archive"}}}}),
        json!({"name":"cortex_feedback","description":"Report a task outcome (success|partial|failure) and which memory sources were actually used, so usefulness statistics stay separate from truth.","inputSchema":{"type":"object","properties":{"outcome":{"type":"string","enum":["success","partial","failure"]},"taskClass":{"type":"string"},"memorySources":{"type":"array","items":{"type":"string"}},"qualityScore":{"type":"number"},"notes":{"type":"string"}},"required":["outcome"]}}),
    ]
}

pub async fn capabilities(cx: &asupersync::Cx, state: &RuntimeState) -> Result<Value, String> {
    let (brain_id, restore_epoch, policy_epoch) = {
        let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
        records::brain_epochs(&conn)
    };
    Ok(json!({
        "operations": KNOWN_OPERATIONS,
        "profiles": LensProfile::ALL.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        "needs": ["current_constraints","open_obligations","last_verified_outcome","failed_attempts","conflicts","as_known","changes","procedures","unverified","compare","audit","map","answer","recipe:<name>"],
        "statuses": ["ok","partial","no_match","ambiguous","needs_more_budget","projection_pending","resnapshot_required","unavailable","denied","outcome_unknown","invalid_request"],
        "evidence": ["brief","support","exact"],
        "brain": {"id": brain_id, "restore_epoch": restore_epoch, "policy_epoch": policy_epoch, "team_mode": state.team_mode},
        "durability_profile": crate::db::DurabilityProfile::from_env().as_str(),
        "adapter_capabilities": crate::adapter::CapabilityManifest::native().to_json(),
        "adapters": {"claude-code-plugin": crate::adapter::CapabilityManifest::claude_code_plugin().to_json(), "tools_only": crate::adapter::CapabilityManifest::tools_only("mcp-tools").to_json()},
        "hook_decisions": ["NOOP","DELIVER","QUERY_REQUIRED","PROJECTION_PENDING","UNAVAILABLE"],
        "zero_token_actions": crate::adapter::ZERO_TOKEN_ACTIONS,
        "protocol_version": "1",
        "legacy_tool_aliases": {"cortex_boot":"orient","cortex_recall":"query","cortex_peek":"query","cortex_semantic_recall":"query","cortex_store":"commit","cortex_unfold":"expand","cortex_conflicts_resolve":"resolve","cortex_focus_start":"checkpoint","cortex_focus_end":"checkpoint","cortex_agent_feedback_record":"feedback","cortex_health":"capabilities"},
        "observation_bridge": {
            "query_orient_field": "observations",
            "scope_arg": "observation_scope",
            "opt_out": "observations=false",
            "expand_refs": ["obs:<source_id>"],
            "epistemic": "attributed_observation",
            "promote": {
                "commit_arg": "evidence",
                "values": ["obs:<source_id>"],
                "relationship": "promoted_from",
                "note": "Explicit Deposit citation only; capture never auto-promotes."
            },
            "note": "Attributed observations are not CQR Cards and never change admission. Caller paths keep path-scoped sources in that repository; the project bucket stays unscoped. Library lens attaches the same observations field beside results."
        },
        "assembly_bridge": {
            "query_orient_field": "assemblies",
            "scope_arg": "observation_scope",
            "opt_out": "assemblies=false",
            "expand_refs": ["asm:<assembly_id>", "rev:<revision_id>"],
            "enabled_by": "rebuild_assembly_routes",
            "note": "Compiled bundles stay off until cue routes are rebuilt. Caller paths keep path-scoped bundles in that repository; a default project compile does not leak them. Library lens attaches the same assemblies field beside results. Ranking does not change CQR Cards or epistemic status."
        },
        "removed_tools": {
            "cortex_boot_audit": "cortex_orient",
            "cortex_diary": "cortex_checkpoint",
            "cortex_forget": "cortex_resolve or retention_class on cortex_commit",
            "cortex_reconnect": "restart the local cortex mcp process",
            "cortex_recall_policy_explain": "cortex_query evidence=support",
            "cortex_focus_status": "cortex_checkpoint action=status",
            "cortex_conflicts_list": "cortex_query profile=conflicts",
            "cortex_conflicts_get": "cortex_expand",
            "cortex_consensus_promote": "cortex_resolve",
            "cortex_memory_decay_run": "cortex maintain CLI",
            "cortex_eval_run": "cortex eval CLI"
        }
    }))
}

pub struct Caller<'a> {
    pub owner_id: Option<i64>,
    pub agent: &'a str,
    pub principal: String,
}

fn profile_for(op: Operation, args: &Value) -> LensProfile {
    if op == Operation::Orient {
        return LensProfile::Orient;
    }
    arg_str(args, &["profile"])
        .and_then(LensProfile::parse)
        .unwrap_or(LensProfile::Answer)
}

async fn run_lens(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    frame: &NeedFrame,
    args: &Value,
    budget_bytes: usize,
) -> Result<View, String> {
    let mut ctx = RecallContext::from_caller(caller.owner_id, state);
    ctx.paths.extend(arg_list(args, &["paths"]));
    ctx.paths.extend(frame.handles.paths.iter().cloned());
    if let Some(cwd) = cwd_root(args) {
        if !ctx.paths.iter().any(|p| p == &cwd) {
            ctx.paths.push(cwd);
        }
    }
    ctx.symbols.extend(arg_list(args, &["symbols"]));
    ctx.symbols.extend(frame.handles.symbols.iter().cloned());
    ctx.as_of = arg_str(args, &["time", "as_of", "valid_at"]).map(str::to_string);
    ctx.session_id = arg_str(args, &["thread"]).map(str::to_string);
    // History and audit search the cold partition too; every other profile
    // discloses it as not searched.
    ctx.include_cold = matches!(frame.profile, LensProfile::History | LensProfile::Audit)
        || arg_bool(args, &["include_cold"]).unwrap_or(false);
    let query_text = if frame.text.is_empty() {
        frame
            .needs
            .iter()
            .map(|n| n.label())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        frame.text.clone()
    };
    // The recall engine's own excerpt budget must not pre-cut candidates:
    // selection against the caller's byte budget happens in the View.
    let token_budget = 4096usize;
    let _ = budget_bytes;
    let payload = execute_unified_recall(
        cx,
        state,
        &query_text,
        token_budget,
        12,
        caller.agent,
        &ctx,
        None,
    )
    .await?;
    let leads = if frame.profile.high_assurance() {
        Vec::new()
    } else {
        let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
        collect_leads(&conn, &query_text, &ctx, &payload)
    };
    let mut view = View::from_recall(frame, &payload, budget_bytes);
    view.leads = leads;
    // Host-attested presence: suppression is a transport saving decided per
    // exact (revision, representation) under matching epochs. Anything else
    // is delivered self-contained.
    let presence: Option<crate::protocol::ContextPresence> = args
        .get("context_presence")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());
    let attested_brain = args["context_presence"]["brain_epoch"]
        .as_str()
        .map(str::to_string);
    let attested_policy = args["context_presence"]["policy_epoch"]
        .as_str()
        .map(str::to_string);
    // The current invocation's context epoch is supplied by the host
    // separately from the attestation's epoch. Without it presence is
    // unknown and delivery is self-contained.
    let context_epoch = arg_str(args, &["context_epoch"]).map(str::to_string);
    let presence = if context_epoch.is_some() {
        presence
    } else {
        None
    };
    view.presence = Some(PresenceInputs {
        presence,
        attested_brain,
        attested_policy,
        context_epoch,
    });
    view.change_cursor_in = arg_str(args, &["change_cursor"]).map(str::to_string);
    // Evidence closure + alias binding happen on the write connection so the
    // receipt row and the closure read the same frontier.
    let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    if frame.evidence == EvidenceDepth::Exact {
        for card in &mut view.cards {
            card.exact_text = unfold_source(&conn, &card.reference, &ctx).and_then(|v| {
                v["text"]
                    .as_str()
                    .or(v["fullText"].as_str())
                    .map(str::to_string)
            });
        }
    }
    view.close_evidence(&conn).map_err(|e| e.to_string())?;
    view.persist_receipt(&conn, &caller.principal)
        .map_err(|e| e.to_string())?;
    Ok(view)
}

/// Continuation contract for a successor, a post-compaction context or an
/// opaque host: goal, constraints, verified outputs, unfinished work,
/// blockers, still-applicable failed attempts, conflicts, last checkpoint
/// and remaining evidence needs — self-contained, typed, redundancy-free.
fn continuation_from(summary: &Value, view: &View) -> Value {
    let empty = Vec::new();
    let obligations = summary["obligations"].as_array().unwrap_or(&empty);
    let attempts = summary["attempts"].as_array().unwrap_or(&empty);
    let by_state = |states: &[&str]| -> Vec<Value> {
        obligations
            .iter()
            .filter(|o| states.contains(&o["state"].as_str().unwrap_or("")))
            .cloned()
            .collect()
    };
    json!({
        "goal": summary["checkpoint"]["goal"],
        "constraints": view.cards.iter().filter(|c| c.label.starts_with('c')).map(|c| json!({"label": c.label, "statement": c.statement, "exceptions": c.exceptions})).collect::<Vec<_>>(),
        "verified_outputs": by_state(&["verified_complete"]),
        "unfinished_work": by_state(&["proposed", "ready", "in_progress", "reopened"]),
        "blockers": {"obligations": by_state(&["blocked"]), "checkpoint": summary["checkpoint"]["state"]["blockers"]},
        "failed_attempts": attempts.iter().filter(|a| a["kind"] == "failure").cloned().collect::<Vec<_>>(),
        "conflicts": view.cards.iter().filter(|c| c.epistemic == "contested").map(|c| json!({"label": c.label, "statement": c.statement, "exceptions": c.exceptions})).collect::<Vec<_>>(),
        "last_checkpoint": summary["checkpoint"],
        "needs_revalidation": by_state(&["reopened"]),
        "evidence_needs": view.coverage.unmet,
        "self_contained": true
    })
}

/// Leads: candidates the collectors found but the admission law did not
/// support. Bounded, labeled, never mixed into Cards.
fn collect_leads(
    conn: &rusqlite::Connection,
    query_text: &str,
    ctx: &RecallContext,
    payload: &Value,
) -> Vec<Value> {
    const MAX_LEADS: usize = 3;
    let admitted: std::collections::HashSet<&str> = payload["results"]
        .as_array()
        .map(|r| r.iter().filter_map(|i| i["source"].as_str()).collect())
        .unwrap_or_default();
    let Ok(trace) = crate::handlers::recall::run_budget_recall_trace_with_query_vector(
        conn, query_text, 0, 24, None, ctx, None, None, false,
    ) else {
        return Vec::new();
    };
    trace
        .candidate_pool
        .iter()
        .filter(|item| !admitted.contains(item.source.as_str()))
        .take(MAX_LEADS)
        .map(|item| json!({"reference": item.source, "hint": item.excerpt.chars().take(80).collect::<String>(), "why": "candidate route without independent support; expand to inspect", "supported": false}))
        .collect()
}

fn observation_cues(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn observation_cues_are_path_only(text: &str, paths: &[String]) -> bool {
    let query_cues = observation_cues(text);
    if query_cues.is_empty() {
        return true;
    }
    if paths.is_empty() {
        return false;
    }
    let mut path_cues = BTreeSet::new();
    for path in paths {
        path_cues.extend(observation_cues(path));
    }
    !path_cues.is_empty() && query_cues.is_subset(&path_cues)
}

/// Attributed observations, separate from CQR Cards.
///
/// Capture is not factual endorsement: hits stay in `observations` with
/// role/source attribution and an `obs:<source_id>` expand handle. They never
/// enter `cards` or the admission law. Caller `paths` / `cwd` restrict
/// path-scoped sources; the unscoped `project` bucket stays visible.
pub(crate) async fn attach_observation_evidence(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    text: &str,
    args: &Value,
    out: &mut Value,
    recent_ok: bool,
) {
    if arg_bool(args, &["observations", "include_observations"]).unwrap_or(true) == false {
        return;
    }
    let extra_scope = arg_str(args, &["observation_scope", "scope"]);
    let scope = extra_scope.unwrap_or("project");
    let paths = commit_paths(args, &json!({}));
    let learned = arg_bool(args, &["observations_learned"]).unwrap_or(false);
    let runtime = crate::CortexRuntime::from_state(state.clone());
    let recent =
        recent_ok && (text.trim().is_empty() || observation_cues_are_path_only(text, &paths));
    let query = if text.trim().is_empty() {
        arg_list(args, &["paths", "symbols"])
            .into_iter()
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        text.to_string()
    };
    if !recent && query.trim().is_empty() {
        out["observations"] = json!({
            "status": "no_match",
            "scope": scope,
            "items": [],
            "count": 0,
            "projection_pending": 0,
            "note": "No observation cues; pass need/task or paths."
        });
        return;
    }
    let pulled = if recent {
        runtime
            .recent_observations_for_paths(cx, &paths, extra_scope, 8, 16 * 1024)
            .await
    } else {
        runtime
            .query_observations_for_paths(
                cx,
                &query,
                &paths,
                extra_scope,
                8,
                16 * 1024,
                learned,
            )
            .await
    };
    match pulled {
        Ok(pv) => {
            let items: Vec<Value> = pv
                .evidence
                .iter()
                .map(|e| {
                    let preview: String = e.text.chars().take(200).collect();
                    json!({
                        "source_id": e.source_id,
                        "source_key": e.source_key,
                        "role": e.role,
                        "route": e.route,
                        "preview": preview,
                        "expand": format!("obs:{}", e.source_id),
                        "trust": {
                            "kind": "attributed_observation",
                            "instruction": false,
                            "privilege": "none",
                            "provenance": e.source_key
                        }
                    })
                })
                .collect();
            let count = items.len();
            out["observations"] = json!({
                "status": pv.status,
                "scope": scope,
                "items": items,
                "count": count,
                "projection_pending": pv.projection_pending,
                "note": "Attributed observations, not CQR facts. Expand obs:<source_id> for exact text."
            });
        }
        Err(err) => {
            let status = if err.contains("stop") || err.contains("permission") {
                "denied"
            } else {
                "unavailable"
            };
            out["observations"] = json!({
                "status": status,
                "scope": scope,
                "items": [],
                "count": 0,
                "projection_pending": 0,
                "error": err,
                "note": "Observation bridge incomplete; CQR Cards are unchanged."
            });
        }
    }
}

/// Evidence-closed assembly bundles. Separate from CQR Cards. Omitted while
/// cue routes are disabled so default recall is unchanged.
pub(crate) async fn attach_assembly_evidence(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    text: &str,
    args: &Value,
    out: &mut Value,
) {
    if arg_bool(args, &["assemblies", "include_assemblies"]).unwrap_or(true) == false {
        return;
    }
    let extra_scope = arg_str(args, &["observation_scope", "scope"]);
    let scope = extra_scope.unwrap_or("project");
    let paths = commit_paths(args, &json!({}));
    let runtime = crate::CortexRuntime::from_state(state.clone());
    let query = if text.trim().is_empty() {
        arg_list(args, &["paths", "symbols"])
            .into_iter()
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        text.to_string()
    };
    let cues = crate::runtime::assembly::tokenize_cues(&query);
    let presence: Option<crate::protocol::ContextPresence> = args
        .get("context_presence")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());
    let attested_brain = args["context_presence"]["brain_epoch"]
        .as_str()
        .map(str::to_string);
    let attested_policy = args["context_presence"]["policy_epoch"]
        .as_str()
        .map(str::to_string);
    let context_epoch = arg_str(args, &["context_epoch"]).unwrap_or("");
    let presence = if context_epoch.is_empty() { None } else { presence };
    match runtime
        .compile_assemblies_for_paths(
            cx,
            &paths,
            extra_scope,
            &cues,
            4,
            presence.as_ref(),
            attested_brain.as_deref(),
            attested_policy.as_deref(),
            context_epoch,
        )
        .await
    {
        Ok(compiled) if compiled.status == "disabled" => {}
        Ok(compiled) => {
            out["assemblies"] = json!({
                "status": compiled.status,
                "scope": compiled.scope,
                "bundles": compiled.bundles,
                "brief": compiled.brief,
                "note": "Evidence-closed assembly bundles, not CQR Cards. Expand asm:<id> for exact members."
            });
        }
        Err(err) => {
            out["assemblies"] = json!({
                "status": "unavailable",
                "scope": scope,
                "bundles": [],
                "brief": "",
                "error": err,
                "note": "Assembly compiler incomplete; CQR Cards are unchanged."
            });
        }
    }
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
        Operation::Orient | Operation::Query => {
            let profile = profile_for(op, args);
            if profile == LensProfile::Compare {
                return compare(cx, state, &caller, args).await;
            }
            if let Some(recipe) = arg_list(args, &["needs"])
                .iter()
                .find_map(|n| n.strip_prefix("recipe:").map(str::to_string))
                .or_else(|| arg_str(args, &["recipe"]).map(str::to_string))
            {
                return run_recipe(cx, state, &caller, &recipe, args).await;
            }
            let text = arg_str(args, &["need", "task", "query", "q"]).unwrap_or("");
            let needs = arg_list(args, &["needs"]);
            let evidence = EvidenceDepth::parse(arg_str(args, &["evidence"]));
            let frame = NeedFrame::build(profile, text, &needs, evidence);
            if frame.text.is_empty()
                && frame.profile != LensProfile::Orient
                && frame.profile != LensProfile::Map
            {
                return Ok(
                    json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "need is required", "field": "need"}),
                );
            }
            let budget = arg_usize(args, &["budget"]).unwrap_or(2000);
            let view = run_lens(cx, state, &caller, &frame, args, budget).await?;
            let mut out = view.to_json();
            if let Some(thread) = arg_str(args, &["thread"]) {
                let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
                if let Ok(summary) = crate::db::threads::thread_summary(&conn, thread) {
                    out["continuation"] = continuation_from(&summary, &view);
                    out["thread"] = summary;
                }
            }
            attach_observation_evidence(
                cx,
                state,
                text,
                args,
                &mut out,
                matches!(op, Operation::Orient),
            )
            .await;
            attach_assembly_evidence(cx, state, text, args, &mut out).await;
            Ok(out)
        }
        Operation::Expand => expand(cx, state, &caller, args).await,
        Operation::Commit => commit(cx, state, &caller, args).await,
        Operation::Checkpoint => checkpoint(cx, state, &caller, args).await,
        Operation::Resolve => resolve(cx, state, &caller, args).await,
        Operation::Feedback => {
            let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
            let owner = if state.team_mode {
                caller
                    .owner_id
                    .ok_or_else(|| "Team mode requires a local owner".to_string())?
            } else {
                0
            };
            let sp = crate::db::SqliteSavepoint::enter(&*conn, "feedback_op")
                .map_err(|e| e.to_string())?;
            let mut out = crate::handlers::feedback::record_agent_feedback_from_value(
                &conn,
                owner,
                args,
                caller.agent,
            )?;
            // Separation ledger: exposure comes from the prior View receipt,
            // use from the caller; they are never the same list.
            let receipt =
                arg_str(args, &["receipt", "prior_view_receipt", "receipt_id"]).map(str::to_string);
            let exposed = match receipt.as_deref() {
                Some(r) => crate::db::feedback_ledger::exposed_from_receipt(&conn, r)?,
                None => Vec::new(),
            };
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
    }
}

/// Typed promotion: no global threshold; each rule names its authority,
/// population, exclusions and revocation triggers in the promoted revision.
async fn promote_op(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    use crate::db::promotion::{promote, Promotion, PromotionRule};
    let Some(rule) = arg_str(args, &["rule"]).and_then(PromotionRule::parse) else {
        return Ok(
            json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "promote.rule must be preference | checker_result | procedure | cross_project_lesson", "field": "promote.rule"}),
        );
    };
    let text = arg_str(args, &["text"]).unwrap_or("");
    if text.is_empty() {
        return Ok(
            json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "promote.text is required", "field": "promote.text"}),
        );
    }
    let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    records::ensure_authoritative_schema(&conn).map_err(|e| e.to_string())?;
    let ack = crate::runtime::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(&conn));
    let sp = crate::db::SqliteSavepoint::enter(&*conn, "promote").map_err(|e| e.to_string())?;
    let seq =
        records::append_commit(&conn, &caller.principal, None, ack).map_err(|e| e.to_string())?;
    let result = promote(
        &conn,
        seq,
        Promotion {
            rule,
            principal: &caller.principal,
            agent: caller.agent,
            authority: arg_str(args, &["authority"]),
            sources: arg_list(args, &["sources"]),
            text,
            preconditions: args.get("preconditions").cloned().unwrap_or(Value::Null),
            target_scope: arg_str(args, &["target_scope", "scope"]),
        },
    );
    match result {
        Ok((record, body)) => {
            sp.release().map_err(|e| e.to_string())?;
            Ok(json!({"status": ResponseStatus::Ok.as_str(), "promoted": record, "body": body}))
        }
        Err(err) => Ok(json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": err})),
    }
}

/// Recipe execution through compiled reads. Named templates only, or an
/// agent-proposed plan under `plan` (validated, replayed, its annotation
/// bytes counted). Unknown names fall back to ordinary recall.
async fn run_recipe(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    recipe: &str,
    args: &Value,
) -> Result<Value, String> {
    let params = args.get("params").cloned().unwrap_or(json!({}));
    let environment = arg_str(args, &["environment", "artifact"])
        .unwrap_or("unspecified")
        .to_string();
    let (steps, outputs, source) = if recipe == "proposed" {
        let plan = args.get("plan").cloned().unwrap_or(Value::Null);
        match recipes::validate_proposed(&plan) {
            Ok((s, o)) => (
                s,
                o,
                json!({"kind": "agent_proposed", "annotation_bytes": plan.to_string().len(), "attributed_to": caller.agent}),
            ),
            Err(err) => {
                return Ok(
                    json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": err, "field": "plan"}),
                )
            }
        }
    } else {
        match recipes::template(recipe, &params) {
            Some((s, o)) => (s, o, json!({"kind": "template", "name": recipe})),
            None => {
                // Not a recipe we know: ordinary recall, never an invented plan.
                let frame = NeedFrame::build(
                    LensProfile::Answer,
                    arg_str(args, &["need", "task", "query"]).unwrap_or(recipe),
                    &[],
                    EvidenceDepth::Brief,
                );
                let view = run_lens(
                    cx,
                    state,
                    caller,
                    &frame,
                    args,
                    arg_usize(args, &["budget"]).unwrap_or(2000),
                )
                .await?;
                let mut out = view.to_json();
                out["recipe"] =
                    json!({"requested": recipe, "status": "unknown_recipe_fell_back_to_recall"});
                return Ok(out);
            }
        }
    };
    let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    let limits = crate::recipe::Limits::default();
    match crate::db::compiled::run_compiled(
        &conn,
        &caller.principal,
        recipe,
        &steps,
        &outputs,
        &params,
        &environment,
        limits,
    ) {
        Ok((value, cached, result)) => Ok(json!({
            "status": ResponseStatus::Ok.as_str(),
            "profile": "recipe",
            "recipe": {"name": recipe, "source": source, "operator_versions": result.operator_versions, "cached": cached, "environment": environment, "params": params},
            "values": value["values"],
            "guards": {"negative": value["guards"], "positive": value["positive"], "brain_epoch": result.brain_epoch, "policy_epoch": result.policy_epoch},
            "work": result.work,
            "interpretation": "typed evidence only; unrecognised questions are not mapped onto a recipe"
        })),
        Err(err) => Ok(
            json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": err, "recipe": recipe}),
        ),
    }
}

fn compare_kind_is_ident(kind: &str) -> bool {
    let mut chars = kind.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `compare`: typed alignment of two records — fields, validity, status,
/// revision heads, conflict relations and a token-level text delta. No
/// prose semantics are manufactured; what differs is listed, not judged.
async fn compare(cx: &asupersync::Cx, state: &RuntimeState, caller: &Caller<'_>, args: &Value) -> Result<Value, String> {
    let refs = arg_list(args, &["compare", "references", "refs"]);
    if refs.len() != 2 {
        return Ok(
            json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "compare needs exactly two references (decision::N or alias with receipt)", "field": "compare"}),
        );
    }
    let ctx = RecallContext::from_caller(caller.owner_id, state);
    let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
    let mut sides = Vec::new();
    for reference in &refs {
        let Some((kind, id)) = reference.split_once("::") else {
            return Ok(
                json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": format!("`{reference}` is not a logical reference"), "field": "compare"}),
            );
        };
        let Ok(id_num) = id.parse::<i64>() else {
            return Ok(
                json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": format!("`{reference}` has a non-numeric id"), "field": "compare"}),
            );
        };
        let Some(source) = unfold_source(&conn, reference, &ctx) else {
            return Ok(
                json!({"status": ResponseStatus::NoMatch.as_str(), "error": format!("`{reference}` is not readable in your scope")}),
            );
        };
        let (table, address_ns) = if kind == "decision" {
            ("decisions", "decision")
        } else if compare_kind_is_ident(kind) {
            ("memories", "memory")
        } else {
            return Ok(
                json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": format!("`{reference}` is not a logical reference"), "field": "compare"}),
            );
        };
        let (kind_col, status, retention, created, valid_from, valid_until) = conn
            .query_row(
                &format!("SELECT COALESCE(type, ?2), status, COALESCE(retention_class,'operational'), created_at, valid_from, valid_until FROM {table} WHERE id = ?1"),
                rusqlite::params![id_num, kind],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?)),
            )
            .map_err(|e| e.to_string())?;
        let record_id =
            records::record_for_legacy(&conn, address_ns, id_num).map_err(|e| e.to_string())?;
        let heads = record_id
            .as_ref()
            .map(|r| records::heads(&conn, r))
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let conflicts: Vec<Value> = if kind == "decision" {
            let mut stmt = conn.prepare("SELECT id, classification, status, source_decision_id, target_decision_id FROM decision_conflicts WHERE source_decision_id = ?1 OR target_decision_id = ?1 ORDER BY id").map_err(|e| e.to_string())?;
            let rows: Vec<Value> = stmt.query_map(rusqlite::params![id_num], |r| Ok(json!({"conflict": r.get::<_, i64>(0)?, "classification": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?, "source": r.get::<_, Option<i64>>(3)?, "target": r.get::<_, i64>(4)?}))).map_err(|e| e.to_string())?.flatten().collect();
            rows
        } else {
            Vec::new()
        };
        sides.push(json!({"reference": reference, "text": source["text"], "kind": kind_col, "status": status, "retention": retention, "created_at": created, "valid_from": valid_from, "valid_until": valid_until, "record": record_id, "heads": heads, "conflicts": conflicts}));
    }
    let a_words: std::collections::BTreeSet<String> = sides[0]["text"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let b_words: std::collections::BTreeSet<String> = sides[1]["text"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let mut differing_fields = Vec::new();
    for field in ["kind", "status", "retention", "valid_from", "valid_until"] {
        if sides[0][field] != sides[1][field] {
            differing_fields
                .push(json!({"field": field, "a": sides[0][field], "b": sides[1][field]}));
        }
    }
    let linked = sides[0]["conflicts"].as_array().unwrap().iter().any(|c| {
        let other = refs[1]
            .split_once("::")
            .and_then(|(_, id)| id.parse::<i64>().ok());
        c["source"].as_i64() == other || c["target"].as_i64() == other
    });
    let ordered = sides[0]["created_at"].as_str() <= sides[1]["created_at"].as_str();
    Ok(json!({
        "status": ResponseStatus::Ok.as_str(),
        "profile": "compare",
        "rule": "compare/1",
        "sides": sides,
        "comparison": {
            "differing_fields": differing_fields,
            "text": {"only_in_a": a_words.difference(&b_words).collect::<Vec<_>>(), "only_in_b": b_words.difference(&a_words).collect::<Vec<_>>(), "shared": a_words.intersection(&b_words).count()},
            "directly_related": linked,
            "chronological": if ordered { "a_before_b" } else { "b_before_a" },
            "interpretation": "field and token alignment only; semantic judgement is the reader's and must be attributed"
        }
    }))
}

async fn expand(cx: &asupersync::Cx, state: &RuntimeState, caller: &Caller<'_>, args: &Value) -> Result<Value, String> {
    // Observation references are exact attributed evidence, not CQR sources.
    if let Some(raw) = arg_str(args, &["reference", "source", "ref"]) {
        if let Some(assembly_id) = raw.strip_prefix("asm:") {
            let runtime = crate::CortexRuntime::from_state(state.clone());
            return match runtime.get_assembly(cx, assembly_id).await {
                Ok(stored) => match runtime.expand_assembly(cx, assembly_id).await {
                    Ok(members) => Ok(json!({
                        "status": ResponseStatus::Ok.as_str(),
                        "reference": raw,
                        "representation": "exact",
                        "assembly": {
                            "id": stored.id,
                            "revision_id": stored.revision_id,
                            "kind": stored.kind,
                            "scope": stored.scope,
                            "members": stored.members.iter().zip(members).map(|(spec, body)| json!({
                                "role": spec.role.as_str(),
                                "revision_id": spec.revision_id,
                                "expand": format!("rev:{}", spec.revision_id),
                                "body": body
                            })).collect::<Vec<_>>(),
                            "trust": {
                                "kind": "assembly_membership",
                                "instruction": false,
                                "privilege": "none",
                                "provenance": stored.id
                            }
                        }
                    })),
                    Err(err) => Ok(json!({
                        "status": ResponseStatus::Unavailable.as_str(),
                        "reference": raw,
                        "error": err
                    })),
                },
                Err(err) => Ok(json!({
                    "status": if err.contains("missing") {
                        ResponseStatus::NoMatch.as_str()
                    } else {
                        ResponseStatus::Unavailable.as_str()
                    },
                    "reference": raw,
                    "error": err
                })),
            };
        }
        if let Some(revision_id) = raw.strip_prefix("rev:") {
            let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
            return match records::revision_body(&conn, revision_id).map_err(|e| e.to_string())? {
                Some(body) => Ok(json!({
                    "status": ResponseStatus::Ok.as_str(),
                    "reference": raw,
                    "representation": "exact",
                    "revision": revision_id,
                    "body": body
                })),
                None => Ok(json!({
                    "status": ResponseStatus::NoMatch.as_str(),
                    "reference": raw,
                    "error": "revision_missing"
                })),
            };
        }
        if let Some(source_id) = raw
            .strip_prefix("obs:")
            .or_else(|| raw.strip_prefix("observation::"))
        {
            let runtime = crate::CortexRuntime::from_state(state.clone());
            return match runtime.read_observation(cx, source_id).await {
                Ok(obs) => Ok(json!({
                    "status": ResponseStatus::Ok.as_str(),
                    "reference": raw,
                    "representation": "exact",
                    "source": {
                        "source_id": obs.source_id,
                        "source_key": obs.source_key,
                        "generation": obs.generation,
                        "role": obs.role,
                        "event_key": obs.event_key,
                        "text": obs.text,
                        "observed_at": obs.observed_at,
                        "trust": {
                            "kind": "attributed_observation",
                            "instruction": false,
                            "privilege": "none",
                            "provenance": obs.source_key
                        }
                    }
                })),
                Err(err) => {
                    let status = if err.contains("not_authorized") || err.contains("unavailable") {
                        ResponseStatus::Unavailable.as_str()
                    } else {
                        ResponseStatus::NoMatch.as_str()
                    };
                    Ok(json!({
                        "status": status,
                        "reference": raw,
                        "error": err,
                        "trust": {"kind": "attributed_observation"}
                    }))
                }
            };
        }
    }
    let ctx = RecallContext::from_caller(caller.owner_id, state);
    let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
    if let Some(alias) = arg_str(args, &["alias", "m"]) {
        let Some(receipt) = arg_str(args, &["receipt", "receipt_id"]) else {
            return Ok(
                json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": format!("alias `{alias}` is scoped to a receipt; pass the receipt id from the View"), "field": "receipt"}),
            );
        };
        let (_, restore_epoch, _) = records::brain_epochs(&conn);
        let bound: Option<(String, String, String, i64)> = conn
            .query_row(
                "SELECT a.record_id, a.revision_id, r.brain_epoch, r.through_sequence FROM view_aliases a JOIN view_receipts r ON r.receipt_id = a.receipt_id WHERE a.receipt_id = ?1 AND a.alias = ?2 AND r.principal_id = ?3",
                rusqlite::params![receipt, alias, caller.principal],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        let Some((record_id, revision_id, epoch, through_sequence)) = bound else {
            return Ok(
                json!({"status": ResponseStatus::NoMatch.as_str(), "error": format!("alias `{alias}` is not bound under receipt `{receipt}` for this principal")}),
            );
        };
        if epoch != restore_epoch {
            return Ok(
                json!({"status": ResponseStatus::ResnapshotRequired.as_str(), "error": "alias minted under a previous restore epoch"}),
            );
        }
        // Revocation fence: a View minted before an erasure is not deliverable.
        if let Err(fence) = crate::db::erasure::fence_check(&conn, through_sequence) {
            return Ok(fence);
        }
        if crate::db::erasure::is_erased(&conn, &record_id) {
            return Ok(
                json!({"status": ResponseStatus::NoMatch.as_str(), "error": "record erased", "record": record_id, "representation": "tombstone"}),
            );
        }
        let body = records::revision_body(&conn, &revision_id).map_err(|e| e.to_string())?;
        let legacy = legacy_reference_for(&conn, &record_id);
        let source = legacy
            .as_deref()
            .and_then(|reference| unfold_source(&conn, reference, &ctx));
        let mut out = json!({"status": ResponseStatus::Ok.as_str(), "alias": alias, "record": record_id, "revision": revision_id, "revision_body": body, "source": source, "representation": "exact"});
        if let Some(decision_id) = record_id
            .rsplit(':')
            .next()
            .and_then(|s| s.parse::<i64>().ok())
        {
            let observations = observations_for_decision(&conn, decision_id);
            if !observations.is_empty() {
                out["evidence"] = json!({"observations": observations, "note": "Promoted from attributed observations; exact text remains expandable via obs:<id>."});
            }
        }
        return Ok(out);
    }
    if let Some(reference) = arg_str(args, &["reference", "source", "ref"]) {
        if let Some(decision_id) = reference
            .strip_prefix("decision::")
            .and_then(|s| s.parse::<i64>().ok())
        {
            let observations = observations_for_decision(&conn, decision_id);
            match unfold_source(&conn, reference, &ctx) {
                Some(source) => {
                    let mut out = json!({"status": ResponseStatus::Ok.as_str(), "reference": reference, "source": source, "representation": "exact"});
                    if !observations.is_empty() {
                        out["evidence"] = json!({"observations": observations, "note": "Promoted from attributed observations; exact text remains expandable via obs:<id>."});
                    }
                    return Ok(out);
                }
                None => {
                    return Ok(
                        json!({"status": ResponseStatus::NoMatch.as_str(), "reference": reference, "error": "no readable source for that reference in your scope"}),
                    );
                }
            }
        }
        return match unfold_source(&conn, reference, &ctx) {
            Some(source) => Ok(
                json!({"status": ResponseStatus::Ok.as_str(), "reference": reference, "source": source, "representation": "exact"}),
            ),
            None => Ok(
                json!({"status": ResponseStatus::NoMatch.as_str(), "reference": reference, "error": "no readable source for that reference in your scope"}),
            ),
        };
    }
    Ok(
        json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "pass alias+receipt or reference", "field": "alias"}),
    )
}

fn legacy_reference_for(conn: &rusqlite::Connection, record_id: &str) -> Option<String> {
    conn.query_row("SELECT namespace, address FROM addresses WHERE record_id = ?1 AND scheme = 'legacy' LIMIT 1", [record_id], |r| {
        Ok(format!("{}::{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })
    .ok()
}

/// Cited V5 observations on a Deposit. Promotion is explicit: capture never
/// becomes a fact by itself, and an unknown/unauthorized cite fails closed.
const OBSERVATION_EVIDENCE_DDL: &str = "
CREATE TABLE IF NOT EXISTS decision_observation_evidence (
  decision_id INTEGER NOT NULL,
  source_id TEXT NOT NULL,
  principal TEXT NOT NULL,
  source_key TEXT NOT NULL,
  role TEXT NOT NULL,
  relationship TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY(decision_id, source_id)
);";

fn observation_evidence_ids(args: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for key in ["evidence", "from_observations", "observation_evidence"] {
        if let Some(Value::Array(items)) = args.get(key) {
            for item in items {
                let raw = item.as_str().unwrap_or_default().trim();
                if raw.is_empty() {
                    continue;
                }
                let id = raw
                    .strip_prefix("obs:")
                    .or_else(|| raw.strip_prefix("observation::"))
                    .unwrap_or(raw);
                if !id.is_empty() {
                    ids.push(id.to_string());
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn link_observation_evidence(
    conn: &rusqlite::Connection,
    principal: &str,
    decision_ids: &[i64],
    source_ids: &[String],
) -> Result<Vec<Value>, Value> {
    if source_ids.is_empty() {
        return Ok(Vec::new());
    }
    conn.execute_batch(OBSERVATION_EVIDENCE_DDL)
        .map_err(|e| json!({"status": ResponseStatus::Unavailable.as_str(), "error": e.to_string()}))?;
    let now = crate::handlers::now_iso();
    let mut linked = Vec::new();
    for source_id in source_ids {
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT e.source_key, g.role FROM observation_events e \
                 JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key \
                 WHERE e.principal=?1 AND e.source_id=?2 \
                 AND g.enabled=1 AND g.role!='delivery_only' \
                 AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id)",
                rusqlite::params![principal, source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| json!({"status": ResponseStatus::Unavailable.as_str(), "error": e.to_string()}))?;
        let Some((source_key, role)) = row else {
            return Err(json!({
                "status": ResponseStatus::InvalidRequest.as_str(),
                "error": format!("observation `{source_id}` is not authorized, disabled, retracted, or missing"),
                "field": "evidence",
                "source_id": source_id
            }));
        };
        for decision_id in decision_ids {
            conn.execute(
                "INSERT OR IGNORE INTO decision_observation_evidence \
                 (decision_id,source_id,principal,source_key,role,relationship,created_at) \
                 VALUES(?1,?2,?3,?4,?5,'promoted_from',?6)",
                rusqlite::params![decision_id, source_id, principal, source_key, role, now],
            )
            .map_err(|e| json!({"status": ResponseStatus::Unavailable.as_str(), "error": e.to_string()}))?;
        }
        linked.push(json!({
            "source_id": source_id,
            "source_key": source_key,
            "role": role,
            "relationship": "promoted_from",
            "expand": format!("obs:{source_id}")
        }));
    }
    Ok(linked)
}

fn decision_id_from_outcome(outcome: &crate::runtime::DepositOutcome) -> Option<i64> {
    outcome
        .target_id
        .or_else(|| outcome.entry.get("id").and_then(Value::as_i64))
}

fn observations_for_decision(conn: &rusqlite::Connection, decision_id: i64) -> Vec<Value> {
    conn.execute_batch(OBSERVATION_EVIDENCE_DDL).ok();
    let Ok(mut stmt) = conn.prepare(
        "SELECT l.source_id,l.source_key,l.role,l.relationship,r.body_json \
         FROM decision_observation_evidence l \
         JOIN observation_events e ON e.source_id=l.source_id AND e.principal=l.principal \
         JOIN revisions r ON r.revision_id=e.revision_id \
         WHERE l.decision_id=?1 ORDER BY l.source_id",
    ) else {
        return Vec::new();
    };
    let rows = stmt
        .query_map([decision_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|_| ())
        .into_iter()
        .flatten()
        .flatten()
        .collect::<Vec<_>>();
    rows.into_iter()
        .filter_map(|(source_id, source_key, role, relationship, body_json)| {
            let body: Value = serde_json::from_str(&body_json).ok()?;
            let text = body
                .pointer("/observation/text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(json!({
                "source_id": source_id,
                "source_key": source_key,
                "role": role,
                "relationship": relationship,
                "text": text,
                "expand": format!("obs:{source_id}"),
                "trust": {
                    "kind": "attributed_observation",
                    "instruction": false,
                    "privilege": "none",
                    "provenance": source_key
                }
            }))
        })
        .collect()
}

async fn commit(cx: &asupersync::Cx, state: &RuntimeState, caller: &Caller<'_>, args: &Value) -> Result<Value, String> {
    use crate::handlers::store::DecisionProvenance;
    use crate::runtime::{deposit_decision, DepositInput};
    if let Some(promotion) = args.get("promote") {
        return promote_op(cx, state, caller, promotion).await;
    }
    let evidence_ids = observation_evidence_ids(args);
    let mut entries: Vec<Value> = args
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        if let Some(text) = arg_str(args, &["decision", "text"]) {
            entries.push(json!({"local_id": "decision", "text": text, "context": args.get("context").cloned().unwrap_or(Value::Null), "kind": args.get("type").cloned().unwrap_or(Value::Null)}));
        }
    }
    if entries.is_empty() {
        return Ok(
            json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "commit needs entries[] or decision", "field": "entries"}),
        );
    }
    let idempotency_key = arg_str(args, &["idempotency_key", "idempotencyKey"]).map(str::to_string);
    let retention = arg_str(args, &["retention_class", "retentionClass"])
        .and_then(crate::api_types::RetentionClass::parse);
    let request_id = arg_str(args, &["request_id"])
        .unwrap_or("mcp-commit")
        .to_string();
    let mut conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    state.drain_deferred(&conn);
    // All entries succeed or none: an outer savepoint around per-entry deposits.
    // Panic, SQL error, and client-error returns all roll the batch back.
    let (receipts, captures, assigned, linked) = match crate::db::with_savepoint_mut(
        &mut *conn,
        "commit_batch",
        |conn| {
            let mut receipts = Vec::new();
            let mut captures = Vec::new();
            let mut assigned = serde_json::Map::new();
            for (index, entry) in entries.iter().enumerate() {
                let Some(text) = entry
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                else {
                    return Err(json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": format!("entries[{index}].text is required"), "field": format!("entries[{index}].text")}));
                };
                let local_id = entry
                    .get("local_id")
                    .and_then(Value::as_str)
                    .unwrap_or("entry")
                    .to_string();
                let key = idempotency_key.as_ref().map(|k| format!("{k}#{local_id}"));
                let outcome = deposit_decision(
                    conn,
                    DepositInput {
                        request_id: &request_id,
                        idempotency_key: key,
                        principal: caller.principal.clone(),
                        text,
                        context: entry
                            .get("context")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        entry_type: entry
                            .get("kind")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| Some("decision".into())),
                        source_agent: caller.agent.to_string(),
                        provenance: DecisionProvenance::from_fields(
                            caller.agent,
                            arg_str(args, &["source_model"]),
                            arg_str(args, &["reasoning_depth"]),
                        ),
                        confidence: args.get("confidence").and_then(Value::as_f64),
                        ttl_seconds: None,
                        retention_class: retention,
                        anchors: Vec::new(),
                        paths: commit_paths(args, entry),
                        thread: arg_str(entry, &["thread"])
                            .or_else(|| arg_str(args, &["thread"]))
                            .map(str::to_string),
                        fields: entry.get("fields").cloned(),
                        owner_id: caller.owner_id,
                        benchmark: false,
                    },
                );
                match outcome {
                    Ok(outcome) => {
                        for (name, id) in &outcome.receipt.entries {
                            assigned.insert(format!("{local_id}.{name}"), json!(id));
                        }
                        captures.push(json!({"entry": local_id, "capture": outcome.capture}));
                        receipts.push(outcome);
                    }
                    Err(err) => {
                        let status = if err.to_string().starts_with("idempotency_conflict") {
                            ResponseStatus::InvalidRequest
                        } else {
                            ResponseStatus::Unavailable
                        };
                        return Err(json!({
                            "status": status.as_str(),
                            "error": err.to_string(),
                            "entry": local_id
                        }));
                    }
                }
            }
            // Cited observations must authorize before the Deposit commits. A failed
            // cite rolls the batch back: promotion is never half-applied.
            let decision_ids: Vec<i64> = receipts.iter().filter_map(decision_id_from_outcome).collect();
            let obs_principal = crate::CortexRuntime::from_state(state.clone())
                .observation_principal()
                .unwrap_or_else(|_| caller.principal.clone());
            let linked = match link_observation_evidence(
                conn,
                &obs_principal,
                &decision_ids,
                &evidence_ids,
            ) {
                Ok(linked) => linked,
                Err(err) => return Err(err),
            };
            Ok((receipts, captures, assigned, linked))
        },
        |e| json!({"status": ResponseStatus::Unavailable.as_str(), "error": e.to_string()}),
    ) {
        Ok(batch) => batch,
        Err(err) => return Ok(err),
    };
    let last = receipts.last().map(|o| o.receipt.clone());
    let mut receipt_json = json!(last);
    receipt_json["entries"] = Value::Object(assigned);
    let mut response = json!({"status": ResponseStatus::Ok.as_str(), "receipt": receipt_json, "captures": captures, "stored": receipts.len(), "legacy_entries": receipts.iter().map(|o| o.entry.clone()).collect::<Vec<_>>(), "evidence": {"relationship": "promoted_from", "linked": linked, "note": "Cited observations remain attributed evidence; the decision is an explicit Deposit."}});
    drop(conn);
    if arg_bool(args, &["return_view"]).unwrap_or(false) {
        let text = entries
            .iter()
            .filter_map(|e| e.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        let frame = NeedFrame::build(LensProfile::Answer, &text, &[], EvidenceDepth::Brief);
        let view = run_lens(
            cx,
            state,
            caller,
            &frame,
            args,
            arg_usize(args, &["budget"]).unwrap_or(1200),
        )
        .await?;
        let mut view_json = view.to_json();
        // The View states whether it sits at this commit's frontier or at a
        // later coherent snapshot; a concurrent change is visible as such.
        let commit_frontier = last
            .as_ref()
            .and_then(|r| r.durability.local_commit.clone());
        let view_frontier: Option<crate::protocol::Frontier> = view
            .frontier
            .clone()
            .and_then(|f| serde_json::from_value(f).ok());
        let at_commit = commit_frontier
            .as_ref()
            .zip(view_frontier.as_ref())
            .map(|(c, v)| c == v)
            .unwrap_or(false);
        view_json["at_commit_frontier"] = json!(at_commit);
        view_json["commit_frontier"] = json!(commit_frontier);
        response["view"] = view_json;
    }
    Ok(response)
}

async fn checkpoint(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    use crate::db::threads;
    let Some(thread) = arg_str(args, &["thread", "label"]) else {
        return Ok(
            json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "thread is required", "field": "thread"}),
        );
    };
    let action = arg_str(args, &["action"]).unwrap_or("checkpoint");
    let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    records::ensure_authoritative_schema(&conn).map_err(|e| e.to_string())?;
    if action == "status" {
        return Ok(
            json!({"status": ResponseStatus::Ok.as_str(), "thread": threads::thread_summary(&conn, thread).map_err(|e| e.to_string())?}),
        );
    }
    let ack = crate::runtime::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(&conn));
    let sp = crate::db::SqliteSavepoint::enter(&*conn, "checkpoint_op").map_err(|e| e.to_string())?;
    let result = (|| -> Result<Value, String> {
        let seq = records::append_commit(&conn, &caller.principal, None, ack)
            .map_err(|e| e.to_string())?;
        let thread_id = threads::ensure_thread(&conn, seq, thread).map_err(|e| e.to_string())?;
        let frontier = crate::store_spi::sqlite::current_frontier(&conn);
        match action {
            "checkpoint" => {
                let goal = arg_str(args, &["goal"]).unwrap_or("");
                let body = json!({"thread": thread_id, "goal": goal, "state": args.get("state").cloned().unwrap_or(json!({})), "note": arg_str(args, &["note"]), "agent": caller.agent});
                let record_id = format!("checkpoint:{thread_id}");
                let parents = records::heads(&conn, &record_id).map_err(|e| e.to_string())?;
                let revision = records::append_revision(
                    &conn,
                    seq,
                    records::NewRevision {
                        record_id: &record_id,
                        kind: "checkpoint",
                        retention: "durable",
                        body,
                        epistemic_status: "asserted",
                        parents: &parents,
                        replace_parents: true,
                        representation_version: "checkpoint/1",
                    },
                )
                .map_err(|e| e.to_string())?;
                conn.execute(
                    "INSERT OR IGNORE INTO thread_members (thread_id, record_id, role) VALUES (?1, ?2, 'checkpoint')",
                    rusqlite::params![thread_id, record_id],
                )
                .map_err(|e| e.to_string())?;
                Ok(
                    json!({"status": ResponseStatus::Ok.as_str(), "thread": thread_id, "checkpoint": revision, "durable": true, "frontier": frontier, "ack_profile": ack}),
                )
            }
            "obligation" => {
                let Some(title) = arg_str(args, &["title"]) else {
                    return Ok(
                        json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "title is required", "field": "title"}),
                    );
                };
                let predicate = args.get("predicate").cloned().unwrap_or(json!({}));
                let record =
                    threads::create_obligation(&conn, seq, thread, title, predicate, caller.agent)
                        .map_err(|e| e.to_string())?;
                Ok(
                    json!({"status": ResponseStatus::Ok.as_str(), "thread": thread_id, "obligation": record, "state": "proposed", "frontier": frontier}),
                )
            }
            "transition" => {
                let (Some(record), Some(to)) = (
                    arg_str(args, &["obligation", "record"]),
                    arg_str(args, &["to", "state"]),
                ) else {
                    return Ok(
                        json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "obligation and to are required", "field": "obligation"}),
                    );
                };
                match threads::transition_obligation(
                    &conn,
                    seq,
                    record,
                    to,
                    caller.agent,
                    arg_str(args, &["note"]),
                ) {
                    Ok(revision) => Ok(
                        json!({"status": ResponseStatus::Ok.as_str(), "obligation": record, "state": to, "revision": revision, "frontier": frontier}),
                    ),
                    Err(err) => {
                        Ok(json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": err}))
                    }
                }
            }
            "verify" => {
                let Some(record) = arg_str(args, &["obligation", "record"]) else {
                    return Ok(
                        json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "obligation is required", "field": "obligation"}),
                    );
                };
                let predicate = arg_str(args, &["predicate"]).unwrap_or("");
                let artifact = arg_str(args, &["artifact"]).unwrap_or("");
                let checker = arg_str(args, &["checker"]).unwrap_or(caller.agent);
                let passed = arg_bool(args, &["passed"]).unwrap_or(false);
                if predicate.is_empty() || artifact.is_empty() {
                    return Ok(
                        json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "verify needs predicate and artifact", "field": "predicate"}),
                    );
                }
                match threads::verify_obligation(
                    &conn,
                    seq,
                    record,
                    predicate,
                    artifact,
                    checker,
                    passed,
                    arg_str(args, &["authority"]),
                ) {
                    Ok(revision) => Ok(
                        json!({"status": ResponseStatus::Ok.as_str(), "obligation": record, "state": "verified_complete", "revision": revision, "frontier": frontier}),
                    ),
                    Err(err) => {
                        Ok(json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": err}))
                    }
                }
            }
            "revalidate" => {
                let (Some(record), Some(artifact)) = (
                    arg_str(args, &["obligation", "record"]),
                    arg_str(args, &["artifact"]),
                ) else {
                    return Ok(
                        json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "obligation and artifact are required", "field": "obligation"}),
                    );
                };
                let reopened = threads::reopen_if_artifact_changed(
                    &conn,
                    seq,
                    record,
                    artifact,
                    caller.agent,
                )?;
                Ok(
                    json!({"status": ResponseStatus::Ok.as_str(), "obligation": record, "reopened": reopened.is_some(), "revision": reopened, "state": threads::obligation_state(&conn, record).map_err(|e| e.to_string())?}),
                )
            }
            "attempt" => {
                let body = args.get("attempt").cloned().unwrap_or_else(|| args.clone());
                let record = threads::record_attempt(
                    &conn,
                    seq,
                    thread,
                    arg_str(args, &["obligation"]),
                    body,
                    caller.agent,
                )
                .map_err(|e| e.to_string())?;
                Ok(
                    json!({"status": ResponseStatus::Ok.as_str(), "thread": thread_id, "attempt": record, "frontier": frontier}),
                )
            }
            other => Ok(
                json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": format!("unknown checkpoint action `{other}`; known: checkpoint, status, obligation, transition, verify, revalidate, attempt"), "field": "action"}),
            ),
        }
    })();
    match &result {
        Ok(v) if v["status"] == "ok" => sp.release().map_err(|e| e.to_string())?,
        _ => {}
    }
    result
}

async fn resolve(cx: &asupersync::Cx, state: &RuntimeState, caller: &Caller<'_>, args: &Value) -> Result<Value, String> {
    if let (Some(record), Some(rationale)) =
        (arg_str(args, &["record"]), arg_str(args, &["rationale"]))
    {
        let considered = arg_list(args, &["considered", "heads"]);
        let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
        let current = records::heads(&conn, record).map_err(|e| e.to_string())?;
        let considered = if considered.is_empty() {
            current.clone()
        } else {
            considered
        };
        if current.is_empty() {
            return Ok(
                json!({"status": ResponseStatus::NoMatch.as_str(), "error": format!("record `{record}` has no heads")}),
            );
        }
        let ack =
            crate::runtime::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(&conn));
        let seq = records::append_commit(&conn, &caller.principal, None, ack)
            .map_err(|e| e.to_string())?;
        return match records::resolve_heads(
            &conn,
            seq,
            record,
            &considered,
            &caller.principal,
            rationale,
            args.get("body").cloned().unwrap_or(json!({})),
        ) {
            Ok(revision) => Ok(
                json!({"status": ResponseStatus::Ok.as_str(), "record": record, "resolution": revision, "considered": considered, "unresolved_heads": current.iter().filter(|h| !considered.contains(h)).collect::<Vec<_>>()}),
            ),
            Err(err) => Ok(
                json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": err.to_string()}),
            ),
        };
    }
    // Legacy conflict resolution (keepId + action) is retained as an alias.
    // Advertised MCP names include winnerId/loserId; JSON-RPC may send
    // those ids as floats or decimal strings, same as budget/horizonDays.
    let keep_id = ["keepId", "keep_id", "winnerId", "winner_id"]
        .iter()
        .find_map(|k| args.get(*k).and_then(json_i64));
    let action = arg_str(args, &["action"]).unwrap_or("");
    let Some(keep_id) = keep_id else {
        return Ok(
            json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": "resolve needs record+rationale (or legacy keepId+action)", "field": "record"}),
        );
    };
    let superseded_id = ["supersededId", "superseded_id", "loserId", "loser_id"]
        .iter()
        .find_map(|k| args.get(*k).and_then(json_i64));
    let mut conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    match crate::handlers::mutate::resolve_decision_with_metadata(
        &mut conn,
        keep_id,
        action,
        superseded_id,
        crate::handlers::mutate::ResolutionMetadata,
    ) {
        Ok(mut payload) => {
            payload["status"] = json!(ResponseStatus::Ok.as_str());
            Ok(payload)
        }
        Err(err) => Ok(json!({"status": ResponseStatus::InvalidRequest.as_str(), "error": err})),
    }
}

/// Validate an envelope-shaped request from a raw package caller.
pub fn validate_envelope(value: &Value) -> Result<Envelope, String> {
    let envelope: Envelope =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid envelope: {e}"))?;
    envelope.validate().map_err(|e| e.to_string())?;
    Ok(envelope)
}

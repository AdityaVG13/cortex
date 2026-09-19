use super::{Caller, arg_bool, arg_list, arg_str, commit};
use crate::handlers::recall::{RecallContext, execute_unified_recall, unfold_source};
use crate::lens::{EvidenceDepth, LensProfile, NeedFrame};
use crate::state::RuntimeState;
use serde_json::{Value, json};

pub(super) fn profile_for(op: super::Operation, args: &Value) -> LensProfile {
    if op == super::Operation::Orient {
        return LensProfile::Orient;
    }
    arg_str(args, &["profile"])
        .and_then(LensProfile::parse)
        .unwrap_or(LensProfile::Answer)
}

pub(crate) async fn run_lens(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    frame: &NeedFrame,
    args: &Value,
    budget_bytes: usize,
) -> Result<super::View, String> {
    let mut ctx = RecallContext::from_caller(caller.owner_id, state);
    ctx.paths.extend(arg_list(args, &["paths"]));
    ctx.paths.extend(frame.handles.paths.iter().cloned());
    if let Some(cwd) = commit::cwd_root(args) {
        if !ctx.paths.iter().any(|p| p == &cwd) {
            ctx.paths.push(cwd);
        }
    }
    ctx.symbols.extend(arg_list(args, &["symbols"]));
    ctx.symbols.extend(frame.handles.symbols.iter().cloned());
    ctx.as_of = arg_str(args, &["time", "as_of", "valid_at"]).map(str::to_string);
    ctx.session_id = arg_str(args, &["thread"]).map(str::to_string);
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
    let mut view = super::View::from_recall(frame, &payload, budget_bytes);
    view.leads = leads;
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
    let context_epoch = arg_str(args, &["context_epoch"]).map(str::to_string);
    let presence = if context_epoch.is_some() {
        presence
    } else {
        None
    };
    view.presence = Some(super::PresenceInputs {
        presence,
        attested_brain,
        attested_policy,
        context_epoch,
    });
    view.change_cursor_in = arg_str(args, &["change_cursor"]).map(str::to_string);
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

pub(super) fn continuation_from(summary: &Value, view: &super::View) -> Value {
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
    json!({"goal":summary["checkpoint"]["goal"],"constraints":view.cards.iter().filter(|c| c.label.starts_with('c')).map(|c| json!({"label":c.label,"statement":c.statement,"exceptions":c.exceptions})).collect::<Vec<_>>(),"verified_outputs":by_state(&["verified_complete"]),"unfinished_work":by_state(&["proposed","ready","in_progress","reopened"]),"blockers":{"obligations":by_state(&["blocked"]),"checkpoint":summary["checkpoint"]["state"]["blockers"]},"failed_attempts":attempts.iter().filter(|a| a["kind"]=="failure").cloned().collect::<Vec<_>>(),"conflicts":view.cards.iter().filter(|c| c.epistemic=="contested").map(|c| json!({"label":c.label,"statement":c.statement,"exceptions":c.exceptions})).collect::<Vec<_>>(),"last_checkpoint":summary["checkpoint"],"needs_revalidation":by_state(&["reopened"]),"evidence_needs":view.coverage.unmet,"self_contained":true})
}

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
    trace.candidate_pool.iter().filter(|item| !admitted.contains(item.source.as_str())).take(MAX_LEADS).map(|item| json!({"reference": item.source, "hint": item.excerpt.chars().take(80).collect::<String>(), "why": "candidate route without independent support; expand to inspect", "supported": false})).collect()
}

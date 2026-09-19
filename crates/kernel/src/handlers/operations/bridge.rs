//! Observation and assembly evidence attached beside CQR Cards.

use super::{arg_bool, arg_list, arg_str, commit};
use crate::handlers::alnum_underscore_lower_set as observation_cues;
use crate::state::RuntimeState;
use serde_json::{Value, json};
use std::collections::BTreeSet;

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

fn query_or_path_cues(text: &str, args: &Value) -> String {
    if text.trim().is_empty() {
        arg_list(args, &["paths", "symbols"]).join(" ")
    } else {
        text.to_string()
    }
}

/// Attributed observations, separate from CQR Cards.
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
    let paths = commit::commit_paths(args, &json!({}));
    let learned = arg_bool(args, &["observations_learned"]).unwrap_or(false);
    let runtime = crate::CortexRuntime::from_state(state.clone());
    let recent =
        recent_ok && (text.trim().is_empty() || observation_cues_are_path_only(text, &paths));
    let query = query_or_path_cues(text, args);
    if !recent && query.trim().is_empty() {
        out["observations"] = json!({"status":"no_match","scope":scope,"items":[],"count":0,"projection_pending":0,"note":"No observation cues; pass need/task or paths."});
        return;
    }
    let pulled = if recent {
        runtime
            .recent_observations_for_paths(cx, &paths, extra_scope, 8, 16 * 1024)
            .await
    } else {
        runtime
            .query_observations_for_paths(cx, &query, &paths, extra_scope, 8, 16 * 1024, learned)
            .await
    };
    match pulled {
        Ok(pv) => {
            let items: Vec<Value> = pv.evidence.iter().map(|e| { let preview: String = e.text.chars().take(200).collect(); json!({"source_id":e.source_id,"source_key":e.source_key,"role":e.role,"route":e.route,"preview":preview,"expand":format!("obs:{}",e.source_id),"trust":{"kind":"attributed_observation","instruction":false,"privilege":"none","provenance":e.source_key}}) }).collect();
            let count = items.len();
            out["observations"] = json!({"status":pv.status,"scope":scope,"items":items,"count":count,"projection_pending":pv.projection_pending,"note":"Attributed observations, not CQR facts. Expand obs:<source_id> for exact text."});
        }
        Err(err) => {
            let status = if err.contains("stop") || err.contains("permission") {
                "denied"
            } else {
                "unavailable"
            };
            out["observations"] = json!({"status":status,"scope":scope,"items":[],"count":0,"projection_pending":0,"error":err,"note":"Observation bridge incomplete; CQR Cards are unchanged."});
        }
    }
}

/// Evidence-closed assembly bundles. Separate from CQR Cards.
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
    let paths = commit::commit_paths(args, &json!({}));
    let runtime = crate::CortexRuntime::from_state(state.clone());
    let query = query_or_path_cues(text, args);
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
    let presence = if context_epoch.is_empty() {
        None
    } else {
        presence
    };
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
            out["assemblies"] = json!({"status":compiled.status,"scope":compiled.scope,"bundles":compiled.bundles,"brief":compiled.brief,"note":"Evidence-closed assembly bundles, not CQR Cards. Expand asm:<id> for exact members."});
        }
        Err(err) => {
            out["assemblies"] = json!({"status":"unavailable","scope":scope,"bundles":[],"brief":"","error":err,"note":"Assembly compiler incomplete; CQR Cards are unchanged."});
        }
    }
}

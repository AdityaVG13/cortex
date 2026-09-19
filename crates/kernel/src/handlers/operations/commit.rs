//! Deposit and typed promote. Cite authorization is a read; the savepoint
//! covers deposits and cite inserts only.

use super::evidence::{AuthorizedCites, decision_id_from_outcome, observation_evidence_ids};
use super::{
    Caller, arg_bool, arg_list, arg_str, arg_usize, invalid_field, invalid_request, run_lens,
    unavailable,
};
use crate::db::records;
use crate::lens::{EvidenceDepth, NeedFrame};
use crate::protocol::{ResponseStatus, nonempty_str};
use crate::state::RuntimeState;
use serde_json::{Value, json};

pub(crate) use crate::handlers::cwd_root;

pub(crate) fn commit_paths(args: &Value, entry: &Value) -> Vec<String> {
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

struct PreparedEntry {
    text: String,
    paths: Vec<String>,
    local_id: String,
    entry: Value,
}

/// Typed promotion: no global threshold; each rule names its authority,
/// population, exclusions and revocation triggers in the promoted revision.
async fn promote_op(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    use crate::db::promotion::{Promotion, PromotionRule, promote};
    let Some(rule) = arg_str(args, &["rule"]).and_then(PromotionRule::parse) else {
        return Ok(invalid_field(
            "promote.rule must be preference | checker_result | procedure | cross_project_lesson",
            "promote.rule",
        ));
    };
    let text = arg_str(args, &["text"]).unwrap_or("");
    if text.is_empty() {
        return Ok(invalid_field("promote.text is required", "promote.text"));
    }
    let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    records::ensure_authoritative_schema(&conn).map_err(|e| e.to_string())?;
    let sp = crate::db::SqliteSavepoint::enter(&*conn, "promote").map_err(|e| e.to_string())?;
    let seq = records::append_ack_commit(&conn, &caller.principal)?;
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
        Err(err) => Ok(invalid_request(err)),
    }
}

pub(crate) async fn commit(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    use crate::handlers::store::DecisionProvenance;
    use crate::runtime::{DepositInput, deposit_decision};
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
        return Ok(invalid_field(
            "commit needs entries[] or decision",
            "entries",
        ));
    }
    let mut prepared = Vec::with_capacity(entries.len());
    for (index, entry) in entries.into_iter().enumerate() {
        let Some(text) = entry
            .get("text")
            .and_then(Value::as_str)
            .and_then(nonempty_str)
        else {
            return Ok(invalid_field(
                format!("entries[{index}].text is required"),
                &format!("entries[{index}].text"),
            ));
        };
        let local_id = entry
            .get("local_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("entry{index}"));
        let paths = commit_paths(args, &entry);
        prepared.push(PreparedEntry {
            text: text.to_string(),
            paths,
            local_id,
            entry,
        });
    }
    let path_sets: Vec<Vec<String>> = prepared.iter().map(|entry| entry.paths.clone()).collect();
    let view_text = prepared
        .iter()
        .map(|entry| entry.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let idempotency_key = arg_str(args, &["idempotency_key", "idempotencyKey"]).map(str::to_string);
    let retention = arg_str(args, &["retention_class", "retentionClass"])
        .and_then(crate::api_types::RetentionClass::parse);
    let request_id = arg_str(args, &["request_id"])
        .unwrap_or("mcp-commit")
        .to_string();
    let obs_principal = crate::CortexRuntime::from_state(state.clone())
        .observation_principal()
        .unwrap_or_else(|_| caller.principal.clone());
    let mut conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    state.drain_deferred(&conn);
    let cites = match AuthorizedCites::authorize(&conn, &obs_principal, &path_sets, &evidence_ids) {
        Ok(cites) => cites,
        Err(err) => return Ok(err),
    };
    let batch = crate::db::with_savepoint_mut(
        &mut *conn,
        "commit_batch",
        |conn| {
            let mut receipts = Vec::new();
            let mut captures = Vec::new();
            let mut assigned = serde_json::Map::new();
            for entry in prepared {
                let key = idempotency_key
                    .as_ref()
                    .map(|k| format!("{}#{}", k, entry.local_id));
                let outcome = deposit_decision(
                    conn,
                    DepositInput {
                        request_id: &request_id,
                        idempotency_key: key,
                        principal: caller.principal.clone(),
                        text: &entry.text,
                        context: entry
                            .entry
                            .get("context")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        entry_type: entry
                            .entry
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
                        paths: entry.paths,
                        evidence: evidence_ids.clone(),
                        thread: arg_str(&entry.entry, &["thread"])
                            .or_else(|| arg_str(args, &["thread"]))
                            .map(str::to_string),
                        fields: entry.entry.get("fields").cloned(),
                        owner_id: caller.owner_id,
                        benchmark: false,
                    },
                );
                match outcome {
                    Ok(outcome) => {
                        for (name, id) in &outcome.receipt.entries {
                            assigned.insert(format!("{}.{name}", entry.local_id), json!(id));
                        }
                        captures.push(json!({"entry": entry.local_id, "capture": outcome.capture}));
                        receipts.push(outcome);
                    }
                    Err(err) => {
                        let status = if err.to_string().starts_with("idempotency_conflict") {
                            ResponseStatus::InvalidRequest
                        } else {
                            ResponseStatus::Unavailable
                        };
                        return Err(
                            json!({"status": status.as_str(), "error": err.to_string(), "entry": entry.local_id}),
                        );
                    }
                }
            }
            let decision_ids: Vec<i64> = receipts
                .iter()
                .filter_map(decision_id_from_outcome)
                .collect();
            let linked = cites.insert(conn, &obs_principal, &decision_ids)?;
            Ok((receipts, captures, assigned, linked))
        },
        |e| unavailable(e),
    );
    let (receipts, captures, assigned, linked) = match batch {
        Ok(v) => v,
        Err(err) => return Ok(err),
    };
    let last = receipts.last().map(|o| o.receipt.clone());
    let mut receipt_json = json!(last);
    receipt_json["entries"] = Value::Object(assigned);
    let mut response = json!({"status": ResponseStatus::Ok.as_str(), "receipt": receipt_json, "captures": captures, "stored": receipts.len(), "legacy_entries": receipts.iter().map(|o| o.entry.clone()).collect::<Vec<_>>(), "evidence": {"relationship": "promoted_from", "linked": linked, "note": "Cited observations remain attributed evidence; the decision is an explicit Deposit."}});
    drop(conn);
    if arg_bool(args, &["return_view"]).unwrap_or(false) {
        let frame = NeedFrame::build(
            crate::lens::LensProfile::Answer,
            &view_text,
            &[],
            EvidenceDepth::Brief,
        );
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

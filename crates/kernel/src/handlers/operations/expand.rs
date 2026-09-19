use super::evidence::observations_for_decision;
use super::{Caller, arg_str, invalid_field, no_match};
use crate::db::records;
use crate::handlers::recall::{RecallContext, unfold_source};
use crate::protocol::ResponseStatus;
use crate::state::RuntimeState;
use serde_json::{Value, json};

fn expand_fail(status: &'static str, raw: &str, error: impl Into<String>) -> Value {
    json!({"status": status, "reference": raw, "error": error.into()})
}

pub(super) async fn expand(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    if let Some(raw) = arg_str(args, &["reference", "source", "ref"]) {
        if let Some(out) = expand_prefixed(cx, state, raw).await? {
            return Ok(out);
        }
    }
    expand_alias_or_source(cx, state, caller, args).await
}

async fn expand_prefixed(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    raw: &str,
) -> Result<Option<Value>, String> {
    if let Some(id) = raw.strip_prefix("asm:") {
        return Ok(Some(expand_assembly(cx, state, raw, id).await?));
    }
    if let Some(id) = raw.strip_prefix("rev:") {
        return Ok(Some(expand_revision(cx, state, raw, id).await?));
    }
    if let Some(id) = raw
        .strip_prefix("obs:")
        .or_else(|| raw.strip_prefix("observation::"))
    {
        return Ok(Some(expand_observation(cx, state, raw, id).await?));
    }
    Ok(None)
}

async fn expand_assembly(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    raw: &str,
    assembly_id: &str,
) -> Result<Value, String> {
    let runtime = crate::CortexRuntime::from_state(state.clone());
    match runtime.get_assembly(cx, assembly_id).await {
        Ok(stored) => match runtime.expand_assembly(cx, assembly_id).await {
            Ok(members) => Ok(
                json!({"status":ResponseStatus::Ok.as_str(),"reference":raw,"representation":"exact","assembly":{"id":stored.id,"revision_id":stored.revision_id,"kind":stored.kind,"scope":stored.scope,"members":stored.members.iter().zip(members).map(|(spec, body)| json!({"role":spec.role.as_str(),"revision_id":spec.revision_id,"expand":format!("rev:{}", spec.revision_id),"body":body})).collect::<Vec<_>>(),"trust":{"kind":"assembly_membership","instruction":false,"privilege":"none","provenance":stored.id}}}),
            ),
            Err(err) => Ok(expand_fail(ResponseStatus::Unavailable.as_str(), raw, err)),
        },
        Err(err) => Ok(expand_fail(
            if err.contains("missing") {
                ResponseStatus::NoMatch.as_str()
            } else {
                ResponseStatus::Unavailable.as_str()
            },
            raw,
            err,
        )),
    }
}

async fn expand_revision(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    raw: &str,
    revision_id: &str,
) -> Result<Value, String> {
    let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
    match records::revision_body(&conn, revision_id).map_err(|e| e.to_string())? {
        Some(body) => Ok(
            json!({"status":ResponseStatus::Ok.as_str(),"reference":raw,"representation":"exact","revision":revision_id,"body":body}),
        ),
        None => Ok(expand_fail(
            ResponseStatus::NoMatch.as_str(),
            raw,
            "revision_missing",
        )),
    }
}

async fn expand_observation(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    raw: &str,
    source_id: &str,
) -> Result<Value, String> {
    let runtime = crate::CortexRuntime::from_state(state.clone());
    match runtime.read_observation(cx, source_id).await {
        Ok(obs) => Ok(
            json!({"status":ResponseStatus::Ok.as_str(),"reference":raw,"representation":"exact","source":{"source_id":obs.source_id,"source_key":obs.source_key,"generation":obs.generation,"role":obs.role,"event_key":obs.event_key,"text":obs.text,"observed_at":obs.observed_at,"trust":{"kind":"attributed_observation","instruction":false,"privilege":"none","provenance":obs.source_key}}}),
        ),
        Err(err) => {
            let status = if err.contains("not_authorized") || err.contains("unavailable") {
                ResponseStatus::Unavailable.as_str()
            } else {
                ResponseStatus::NoMatch.as_str()
            };
            let mut out = expand_fail(status, raw, err);
            out["trust"] = json!({"kind": "attributed_observation"});
            Ok(out)
        }
    }
}

async fn expand_alias_or_source(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    let ctx = RecallContext::from_caller(caller.owner_id, state);
    let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
    if let Some(alias) = arg_str(args, &["alias", "m"]) {
        return expand_alias(&conn, caller, args, alias, &ctx);
    }
    if let Some(reference) = arg_str(args, &["reference", "source", "ref"]) {
        return expand_logical(&conn, reference, &ctx);
    }
    Ok(invalid_field("pass alias+receipt or reference", "alias"))
}

fn expand_alias(
    conn: &rusqlite::Connection,
    caller: &Caller<'_>,
    args: &Value,
    alias: &str,
    ctx: &RecallContext,
) -> Result<Value, String> {
    let Some(receipt) = arg_str(args, &["receipt", "receipt_id"]) else {
        return Ok(invalid_field(
            format!("alias `{alias}` is scoped to a receipt; pass the receipt id from the View"),
            "receipt",
        ));
    };
    let (_, restore_epoch, _) = records::brain_epochs(conn);
    let bound: Option<(String, String, String, i64)> = conn.query_row("SELECT a.record_id, a.revision_id, r.brain_epoch, r.through_sequence FROM view_aliases a JOIN view_receipts r ON r.receipt_id = a.receipt_id WHERE a.receipt_id = ?1 AND a.alias = ?2 AND r.principal_id = ?3", rusqlite::params![receipt, alias, caller.principal], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).ok();
    let Some((record_id, revision_id, epoch, through_sequence)) = bound else {
        return Ok(no_match(format!(
            "alias `{alias}` is not bound under receipt `{receipt}` for this principal"
        )));
    };
    if epoch != restore_epoch {
        return Ok(
            json!({"status": ResponseStatus::ResnapshotRequired.as_str(), "error": "alias minted under a previous restore epoch"}),
        );
    }
    if let Err(fence) = crate::db::erasure::fence_check(conn, through_sequence) {
        return Ok(fence);
    }
    if crate::db::erasure::is_erased(conn, &record_id) {
        return Ok(
            json!({"status": ResponseStatus::NoMatch.as_str(), "error": "record erased", "record": record_id, "representation": "tombstone"}),
        );
    }
    let body = records::revision_body(conn, &revision_id).map_err(|e| e.to_string())?;
    let legacy = legacy_reference_for(conn, &record_id);
    let source = legacy
        .as_deref()
        .and_then(|reference| unfold_source(conn, reference, ctx));
    let mut out = json!({"status": ResponseStatus::Ok.as_str(), "alias": alias, "record": record_id, "revision": revision_id, "revision_body": body, "source": source, "representation": "exact"});
    if let Some(decision_id) = record_id
        .rsplit(':')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
    {
        attach_promoted_observations(&mut out, observations_for_decision(conn, decision_id));
    }
    Ok(out)
}

fn expand_logical(
    conn: &rusqlite::Connection,
    reference: &str,
    ctx: &RecallContext,
) -> Result<Value, String> {
    let observations = reference
        .strip_prefix("decision::")
        .and_then(|s| s.parse::<i64>().ok())
        .map(|id| observations_for_decision(conn, id))
        .unwrap_or_default();
    match unfold_source(conn, reference, ctx) {
        Some(source) => {
            let mut out = json!({"status": ResponseStatus::Ok.as_str(), "reference": reference, "source": source, "representation": "exact"});
            attach_promoted_observations(&mut out, observations);
            Ok(out)
        }
        None => Ok(expand_fail(
            ResponseStatus::NoMatch.as_str(),
            reference,
            "no readable source for that reference in your scope",
        )),
    }
}

fn attach_promoted_observations(out: &mut Value, observations: Vec<Value>) {
    if !observations.is_empty() {
        out["evidence"] = json!({"observations": observations, "note": "Promoted from attributed observations; exact text remains expandable via obs:<id>."});
    }
}

fn legacy_reference_for(conn: &rusqlite::Connection, record_id: &str) -> Option<String> {
    conn.query_row("SELECT namespace, address FROM addresses WHERE record_id = ?1 AND scheme = 'legacy' LIMIT 1", [record_id], |r| Ok(format!("{}::{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))).ok()
}

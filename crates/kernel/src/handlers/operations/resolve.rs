use super::{Caller, arg_i64, arg_list, arg_str, invalid_field, invalid_request, no_match};
use crate::db::records;
use crate::protocol::ResponseStatus;
use crate::state::RuntimeState;
use serde_json::{Value, json};

pub(super) async fn resolve(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    if let (Some(record), Some(rationale)) =
        (arg_str(args, &["record"]), arg_str(args, &["rationale"]))
    {
        return resolve_heads(cx, state, caller, args, record, rationale).await;
    }
    resolve_legacy(cx, state, args).await
}

async fn resolve_heads(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
    record: &str,
    rationale: &str,
) -> Result<Value, String> {
    let considered = arg_list(args, &["considered", "heads"]);
    let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    let current = records::heads(&conn, record).map_err(|e| e.to_string())?;
    let considered = if considered.is_empty() {
        current.clone()
    } else {
        considered
    };
    if current.is_empty() {
        return Ok(no_match(format!("record `{record}` has no heads")));
    }
    let seq = records::append_ack_commit(&conn, &caller.principal)?;
    Ok(records::resolve_heads(&conn, seq, record, &considered, &caller.principal, rationale, args.get("body").cloned().unwrap_or(json!({}))).map(|revision| json!({"status": ResponseStatus::Ok.as_str(), "record": record, "resolution": revision, "considered": considered, "unresolved_heads": current.iter().filter(|h| !considered.contains(h)).collect::<Vec<_>>()})).unwrap_or_else(|err| invalid_request(err.to_string())))
}

async fn resolve_legacy(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    args: &Value,
) -> Result<Value, String> {
    let keep_id = arg_i64(args, &["keepId", "keep_id", "winnerId", "winner_id"]);
    let action = arg_str(args, &["action"]).unwrap_or("");
    let Some(keep_id) = keep_id else {
        return Ok(invalid_field(
            "resolve needs record+rationale (or legacy keepId+action)",
            "record",
        ));
    };
    let superseded_id = arg_i64(
        args,
        &["supersededId", "superseded_id", "loserId", "loser_id"],
    );
    let mut conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    Ok(crate::handlers::mutate::resolve_decision_with_metadata(
        &mut conn,
        keep_id,
        action,
        superseded_id,
        crate::handlers::mutate::ResolutionMetadata,
    )
    .map(|mut payload| {
        payload["status"] = json!(ResponseStatus::Ok.as_str());
        payload
    })
    .unwrap_or_else(|err| invalid_request(err)))
}

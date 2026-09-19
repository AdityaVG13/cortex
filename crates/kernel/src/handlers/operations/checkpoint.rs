use super::{Caller, arg_bool, arg_str, invalid_field, invalid_request};
use crate::db::records;
use crate::protocol::{Frontier, ResponseStatus};
use crate::state::RuntimeState;
use serde_json::{Value, json};

pub(super) async fn checkpoint(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    use crate::db::threads;
    let Some(thread) = arg_str(args, &["thread", "label"]) else {
        return Ok(invalid_field("thread is required", "thread"));
    };
    let action = arg_str(args, &["action"]).unwrap_or("checkpoint");
    let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    records::ensure_authoritative_schema(&conn).map_err(|e| e.to_string())?;
    if action == "status" {
        return Ok(
            json!({"status": ResponseStatus::Ok.as_str(), "thread": threads::thread_summary(&conn, thread).map_err(|e| e.to_string())?}),
        );
    }
    let ack = records::ack_label(&conn);
    let sp =
        crate::db::SqliteSavepoint::enter(&*conn, "checkpoint_op").map_err(|e| e.to_string())?;
    let result = run_checkpoint_action(&conn, caller, args, thread, action, ack);
    if matches!(&result, Ok(v) if v["status"] == "ok") {
        sp.release().map_err(|e| e.to_string())?;
    }
    result
}

fn run_checkpoint_action(
    conn: &rusqlite::Connection,
    caller: &Caller<'_>,
    args: &Value,
    thread: &str,
    action: &str,
    ack: &str,
) -> Result<Value, String> {
    use crate::db::threads;
    let seq =
        records::append_commit(conn, &caller.principal, None, ack).map_err(|e| e.to_string())?;
    let thread_id = threads::ensure_thread(conn, seq, thread).map_err(|e| e.to_string())?;
    let frontier = crate::store_spi::sqlite::current_frontier(conn);
    match action {
        "checkpoint" => write_checkpoint(conn, caller, args, &thread_id, seq, frontier, ack),
        "obligation" => write_obligation(conn, caller, args, thread, &thread_id, seq, frontier),
        "transition" => write_transition(conn, caller, args, seq, frontier),
        "verify" => write_verify(conn, caller, args, seq, frontier),
        "revalidate" => write_revalidate(conn, caller, args, seq),
        "attempt" => write_attempt(conn, caller, args, thread, &thread_id, seq, frontier),
        other => Ok(invalid_field(
            format!(
                "unknown checkpoint action `{other}`; known: checkpoint, status, obligation, transition, verify, revalidate, attempt"
            ),
            "action",
        )),
    }
}

fn write_checkpoint(
    conn: &rusqlite::Connection,
    caller: &Caller<'_>,
    args: &Value,
    thread_id: &str,
    seq: i64,
    frontier: Frontier,
    ack: &str,
) -> Result<Value, String> {
    let goal = arg_str(args, &["goal"]).unwrap_or("");
    let body = json!({"thread": thread_id, "goal": goal, "state": args.get("state").cloned().unwrap_or(json!({})), "note": arg_str(args, &["note"]), "agent": caller.agent});
    let record_id = format!("checkpoint:{thread_id}");
    let parents = records::heads(conn, &record_id).map_err(|e| e.to_string())?;
    let revision = records::append_revision(
        conn,
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
    crate::db::threads::add_thread_member(conn, thread_id, &record_id, "checkpoint")
        .map_err(|e| e.to_string())?;
    Ok(
        json!({"status": ResponseStatus::Ok.as_str(), "thread": thread_id, "checkpoint": revision, "durable": true, "frontier": frontier, "ack_profile": ack}),
    )
}

fn write_obligation(
    conn: &rusqlite::Connection,
    caller: &Caller<'_>,
    args: &Value,
    thread: &str,
    thread_id: &str,
    seq: i64,
    frontier: Frontier,
) -> Result<Value, String> {
    let Some(title) = arg_str(args, &["title"]) else {
        return Ok(invalid_field("title is required", "title"));
    };
    let predicate = args.get("predicate").cloned().unwrap_or(json!({}));
    let record =
        crate::db::threads::create_obligation(conn, seq, thread, title, predicate, caller.agent)
            .map_err(|e| e.to_string())?;
    Ok(
        json!({"status": ResponseStatus::Ok.as_str(), "thread": thread_id, "obligation": record, "state": "proposed", "frontier": frontier}),
    )
}

fn write_transition(
    conn: &rusqlite::Connection,
    caller: &Caller<'_>,
    args: &Value,
    seq: i64,
    frontier: Frontier,
) -> Result<Value, String> {
    let (Some(record), Some(to)) = (
        arg_str(args, &["obligation", "record"]),
        arg_str(args, &["to", "state"]),
    ) else {
        return Ok(invalid_field(
            "obligation and to are required",
            "obligation",
        ));
    };
    Ok(crate::db::threads::transition_obligation(conn, seq, record, to, caller.agent, arg_str(args, &["note"])).map(|revision| json!({"status": ResponseStatus::Ok.as_str(), "obligation": record, "state": to, "revision": revision, "frontier": frontier})).unwrap_or_else(|err| invalid_request(err)))
}

fn write_verify(
    conn: &rusqlite::Connection,
    caller: &Caller<'_>,
    args: &Value,
    seq: i64,
    frontier: Frontier,
) -> Result<Value, String> {
    let Some(record) = arg_str(args, &["obligation", "record"]) else {
        return Ok(invalid_field("obligation is required", "obligation"));
    };
    let predicate = arg_str(args, &["predicate"]).unwrap_or("");
    let artifact = arg_str(args, &["artifact"]).unwrap_or("");
    let checker = arg_str(args, &["checker"]).unwrap_or(caller.agent);
    let passed = arg_bool(args, &["passed"]).unwrap_or(false);
    if predicate.is_empty() || artifact.is_empty() {
        return Ok(invalid_field(
            "verify needs predicate and artifact",
            "predicate",
        ));
    }
    Ok(crate::db::threads::verify_obligation(conn, seq, record, predicate, artifact, checker, passed, arg_str(args, &["authority"])).map(|revision| json!({"status": ResponseStatus::Ok.as_str(), "obligation": record, "state": "verified_complete", "revision": revision, "frontier": frontier})).unwrap_or_else(|err| invalid_request(err)))
}

fn write_revalidate(
    conn: &rusqlite::Connection,
    caller: &Caller<'_>,
    args: &Value,
    seq: i64,
) -> Result<Value, String> {
    let (Some(record), Some(artifact)) = (
        arg_str(args, &["obligation", "record"]),
        arg_str(args, &["artifact"]),
    ) else {
        return Ok(invalid_field(
            "obligation and artifact are required",
            "obligation",
        ));
    };
    let reopened =
        crate::db::threads::reopen_if_artifact_changed(conn, seq, record, artifact, caller.agent)?;
    Ok(
        json!({"status": ResponseStatus::Ok.as_str(), "obligation": record, "reopened": reopened.is_some(), "revision": reopened, "state": crate::db::threads::obligation_state(conn, record).map_err(|e| e.to_string())?}),
    )
}

fn write_attempt(
    conn: &rusqlite::Connection,
    caller: &Caller<'_>,
    args: &Value,
    thread: &str,
    thread_id: &str,
    seq: i64,
    frontier: Frontier,
) -> Result<Value, String> {
    let body = args.get("attempt").cloned().unwrap_or_else(|| args.clone());
    let record = crate::db::threads::record_attempt(
        conn,
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

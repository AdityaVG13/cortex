use super::*;
use crate::db::{compiled, outbox, records};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::json;

pub(crate) fn capture_registered(
    conn: &mut Connection,
    principal: &str,
    source: &str,
    generation: &str,
    event: ObservationEvent,
) -> Result<ObservationReceipt, String> {
    check_label(source)?;
    check_label(generation)?;
    ensure(conn)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| err.to_string())?;
    let grant = granted(&tx, principal, source, true)?;
    let receipt = capture(&tx, principal, source, generation, &grant, event)?;
    tx.commit().map_err(|err| err.to_string())?;
    Ok(receipt)
}

pub(crate) fn source_capture_limit(
    conn: &Connection,
    principal: &str,
    source: &str,
) -> Result<usize, String> {
    ensure(conn)?;
    Ok(granted(conn, principal, source, true)?.max_bytes)
}
pub(in crate::runtime) fn capture(
    conn: &Connection,
    principal: &str,
    key: &str,
    generation: &str,
    grant: &GrantedSource,
    event: ObservationEvent,
) -> Result<ObservationReceipt, String> {
    check_label(&event.event_key)?;
    if event.text.len() > grant.max_bytes {
        return Err("capture_byte_limit".into());
    }
    if event
        .observed_at
        .as_ref()
        .is_some_and(|time| chrono::DateTime::parse_from_rfc3339(time).is_err())
    {
        return Err("invalid_observation_time".into());
    }
    if crate::handlers::redact_secrets(&event.text) != event.text {
        return Err("capture_secret_rejected".into());
    }
    let body = json!({"observation":event,"role":grant.role});
    let old: Option<(String,String)> = conn.query_row("SELECT r.body_json,e.receipt_json FROM observation_events e JOIN revisions r ON r.revision_id=e.revision_id WHERE e.principal=?1 AND e.source_key=?2 AND e.generation=?3 AND e.event_key=?4", params![principal,key,generation,event.event_key], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|err| err.to_string())?;
    if let Some((previous, receipt)) = old {
        if serde_json::from_str::<serde_json::Value>(&previous).map_err(|err| err.to_string())?
            != body
        {
            return Err("event_identity_conflict".into());
        }
        let mut receipt: ObservationReceipt =
            serde_json::from_str(&receipt).map_err(|err| err.to_string())?;
        let availability: String = conn
            .query_row(
                "SELECT availability FROM sources WHERE source_id=?1",
                params![receipt.source_id],
                |r| r.get(0),
            )
            .map_err(|err| err.to_string())?;
        if availability != "owned_inline" {
            return Err("source_unavailable".into());
        }
        receipt.duplicate = true;
        return Ok(receipt);
    }
    if outbox::debt(conn).refuse_intake() {
        return Err(
            "maintenance_debt_hard_limit: run cortex maintain before capturing more".into(),
        );
    }
    let ack = crate::store_spi::sqlite::ack_profile(conn);
    let sequence = records::append_commit(
        conn,
        principal,
        None,
        crate::runtime::ack_profile_label_pub(&ack),
    )
    .map_err(|err| err.to_string())?;
    let source_id = format!("source:{}", uuid::Uuid::new_v4());
    let record_id = format!("observation:{}", uuid::Uuid::new_v4());
    let revision_id = format!("{record_id}@1");
    conn.execute("INSERT INTO sources(source_id,scope_id,origin_id,media_type,availability,inline_payload,byte_length,capture_sequence) VALUES(?1,?2,?3,'text/plain;charset=utf-8','owned_inline',?4,?5,?6)", params![source_id,grant.scope_id,key,event.text.as_bytes(),event.text.len() as i64,sequence]).map_err(|err| err.to_string())?;
    conn.execute("INSERT INTO records(record_id,scope_id,kind,retention,created_sequence) VALUES(?1,?2,'observation','operational',?3)", params![record_id,grant.scope_id,sequence]).map_err(|err| err.to_string())?;
    conn.execute("INSERT INTO revisions(revision_id,record_id,body_json,epistemic_status,recorded_sequence,representation_version) VALUES(?1,?2,?3,'asserted',?4,'observation/1')", params![revision_id,record_id,body.to_string(),sequence]).map_err(|err| err.to_string())?;
    conn.execute(
        "INSERT INTO record_heads(record_id,revision_id) VALUES(?1,?2)",
        params![record_id, revision_id],
    )
    .map_err(|err| err.to_string())?;
    conn.execute(
        "INSERT INTO revision_sources(revision_id,source_id,role,extent_json) VALUES(?1,?2,?3,?4)",
        params![
            revision_id,
            source_id,
            grant.role,
            json!({"start":0,"end":event.text.len()}).to_string()
        ],
    )
    .map_err(|err| err.to_string())?;
    conn.execute("INSERT INTO change_items(sequence,ordinal,scope_id,record_id,change_kind) VALUES(?1,0,?2,?3,'captured')", params![sequence,grant.scope_id,record_id]).map_err(|err| err.to_string())?;
    compiled::bump_guard(conn, &grant.scope_id, "observation").map_err(|err| err.to_string())?;
    outbox::enqueue_for_commit(
        conn,
        sequence,
        &["checkpoint_wal"],
        json!({"source_id":source_id}),
    )
    .map_err(|err| err.to_string())?;
    let receipt = ObservationReceipt {
        source_id,
        record_id,
        revision_id,
        sequence,
        restore_epoch: records::brain_epochs(conn).1,
        ack_profile: ack,
        retained_bytes: event.text.len(),
        duplicate: false,
    };
    conn.execute("INSERT INTO observation_events(principal,source_key,generation,event_key,source_id,revision_id,receipt_json) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![principal,key,generation,event.event_key,receipt.source_id,receipt.revision_id,serde_json::to_string(&receipt).map_err(|err| err.to_string())?]).map_err(|err| err.to_string())?;
    crate::runtime::cycle::on_capture(conn, principal, key)?;

    Ok(receipt)
}

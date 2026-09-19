use super::{DepositInput, ack_profile_label};
use crate::protocol::{DurabilityVector, Frontier, LogicalId, PayloadAvailability, Receipt};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn record_authoritative(
    conn: &Connection,
    input: &DepositInput<'_>,
    decision_id: i64,
    text: &str,
    action: &str,
) -> rusqlite::Result<()> {
    use crate::db::records::{
        NewRevision, append_commit, append_revision, heads, record_for_legacy,
    };
    crate::db::records::ensure_authoritative_schema(conn)?;
    let ack = ack_profile_label(&crate::store_spi::sqlite::ack_profile(conn));
    let sequence = append_commit(
        conn,
        &input.principal,
        input.idempotency_key.as_deref(),
        ack,
    )?;
    let retention: String = conn
        .query_row(
            "SELECT COALESCE(retention_class, 'operational') FROM decisions WHERE id = ?1",
            rusqlite::params![decision_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "operational".into());
    let retention =
        if ["durable", "operational", "audit", "ephemeral"].contains(&retention.as_str()) {
            retention
        } else {
            "operational".into()
        };
    let record_id = record_for_legacy(conn, "decision", decision_id)?
        .unwrap_or_else(|| format!("decision:{decision_id}"));
    let parents = if matches!(action, "stored" | "inserted") {
        Vec::new()
    } else {
        heads(conn, &record_id)?
    };
    // The record kind is the entry type (constraint, exception, attempt,
    // procedure, decision …): it is the relation recipes select on.
    let kind: String = conn
        .query_row(
            "SELECT COALESCE(type, 'decision') FROM decisions WHERE id = ?1",
            rusqlite::params![decision_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "decision".into());
    let subject = text
        .split(|c: char| c == ':' || c == '—')
        .next()
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let mut body = json!({"text": text, "agent": input.source_agent, "action": action, "context": input.context, "subject": subject});
    if let Some(Value::Object(fields)) = &input.fields {
        for (k, v) in fields {
            body[k] = v.clone();
        }
    }
    append_revision(
        conn,
        sequence,
        NewRevision {
            record_id: &record_id,
            kind: &kind,
            retention: &retention,
            body,
            epistemic_status: "asserted",
            parents: &parents,
            replace_parents: true,
            representation_version: "decision/1",
        },
    )?;
    conn.execute("INSERT OR IGNORE INTO addresses (scheme, namespace, address, record_id) VALUES ('legacy', 'decision', ?1, ?2)", rusqlite::params![decision_id.to_string(), record_id])?;
    // Heavy projection work is durable debt, not writer-lock work: the
    // outbox rows commit with the deposit; a later caller drains them.
    crate::db::outbox::enqueue_for_commit(
        conn,
        sequence,
        &["fts_optimize", "checkpoint_wal"],
        json!({"decision": decision_id}),
    )?;
    if sequence % 50 == 0 {
        crate::db::outbox::enqueue_for_commit(
            conn,
            sequence,
            &["prune_telemetry", "clock_link_audit"],
            json!({}),
        )?;
    }
    Ok(())
}

pub(super) fn build_receipt(
    conn: &Connection,
    request_id: &str,
    target_id: Option<i64>,
    version_id: Option<i64>,
    action: &str,
) -> Receipt {
    let frontier = crate::store_spi::sqlite::current_frontier(conn);
    let mut entries = BTreeMap::new();
    if let Some(id) = target_id {
        entries.insert(
            "decision".to_string(),
            LogicalId::from_legacy("decision", id),
        );
    }
    if let Some(v) = version_id {
        entries.insert("version".to_string(), LogicalId::from_legacy("version", v));
    }
    let receipt_value = version_id
        .map(|v| v.to_string())
        .or_else(|| target_id.map(|t| format!("d{t}")))
        .unwrap_or_else(|| "none".into());
    let mut omissions = Vec::new();
    if !matches!(action, "stored" | "inserted") {
        omissions.push(format!(
            "store action `{action}`: request merged into or deduplicated against an existing row"
        ));
    }
    Receipt {
        receipt_id: LogicalId::new("receipt", receipt_value),
        request_id: request_id.to_string(),
        durability: DurabilityVector {
            accepted: true,
            local_commit: Some(frontier.clone()),
            projected_through: projected_through(&frontier),
            replicated_through: BTreeMap::new(),
            payload_availability: PayloadAvailability::Retained,
            ack_profile: crate::store_spi::sqlite::ack_profile(conn),
        },
        entries,
        aliases: BTreeMap::new(),
        omissions,
        unresolved_needs: Vec::new(),
    }
}

/// Every projection today is maintained synchronously inside the store
/// transaction (FTS triggers, entities, clock anchors), so each index is at
/// the commit frontier. The outbox bead makes these lag independently.
fn projected_through(frontier: &Frontier) -> BTreeMap<String, Frontier> {
    ["exact", "lexical", "entity", "clock"]
        .iter()
        .map(|name| (name.to_string(), frontier.clone()))
        .collect()
}

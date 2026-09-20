mod record;

use crate::clockwork::{AnchorKind, ClockOrigin, QueryAnchor};
use crate::handlers::store::{
    DecisionProvenance, StoreError, store_decision_with_input_embedding_and_provenance_retention,
};
use crate::protocol::{AckProfile, CaptureReceipt, CaptureStatus, Receipt, nonempty_opt};
use cortex_logic::api_types::RetentionClass;
use record::{build_receipt, record_authoritative};
use rusqlite::Connection;
use serde_json::{Value, json};

/// Validated Deposit of one decision. Redaction has already been applied by
/// the caller boundary (`handlers::redact_secrets`) or is applied here for
/// library callers — both paths redact before any projection.
pub struct DepositInput<'a> {
    pub request_id: &'a str,
    /// Principal-scoped idempotency key. Same key + same canonical payload
    /// returns the original outcome; different payload is a conflict.
    pub idempotency_key: Option<String>,
    pub principal: String,
    pub text: &'a str,
    pub context: Option<String>,
    pub entry_type: Option<String>,
    pub source_agent: String,
    pub provenance: DecisionProvenance,
    pub confidence: Option<f64>,
    pub ttl_seconds: Option<i64>,
    pub retention_class: Option<RetentionClass>,
    pub anchors: Vec<QueryAnchor>,
    /// Caller project roots. Projected as explicit path anchors so later
    /// lens/orient/boot with a cwd can admit this row and drop a foreign repo.
    pub paths: Vec<String>,
    /// Observation source ids cited on this Deposit. Part of the
    /// idempotency canonical so a retry cannot amend provenance.
    pub evidence: Vec<String>,
    /// Optional thread/session label. Task-clock evidence, not an eligibility filter.
    pub thread: Option<String>,
    /// Typed fields (case/procedure/counterexample bodies) merged into the
    /// authoritative revision body; free text stays representable.
    pub fields: Option<Value>,
    pub owner_id: Option<i64>,
    /// Benchmark scopes skip focus capture so they do not pollute working state.
    pub benchmark: bool,
}

#[derive(Debug, Clone)]
pub struct DepositOutcome {
    /// Legacy entry payload (`id`, `action`, `versionId`, …) kept for the
    /// existing `/store` wire shape.
    pub entry: Value,
    pub target_id: Option<i64>,
    pub receipt: Receipt,
    pub capture: CaptureReceipt,
}

/// Capture receipt for one text field: what was offered, what was retained,
/// and under which named policy. Redaction is reported, never silent.
pub fn capture_receipt(offered: &str, retained: &str, max_chars: usize) -> CaptureReceipt {
    let offered_bytes = offered.len();
    let retained_bytes = retained.len();
    let redacted = retained.contains("[REDACTED")
        || (retained_bytes < offered_bytes
            && retained.chars().count() < offered.chars().count()
            && retained.chars().count() < max_chars);
    let truncated = offered.chars().count() > max_chars;
    let (status, reason) = if truncated {
        (
            CaptureStatus::Incomplete,
            Some(format!("truncated to {max_chars} chars")),
        )
    } else if redacted {
        (
            CaptureStatus::Redacted,
            Some("secret-shaped spans replaced".into()),
        )
    } else {
        (CaptureStatus::Accepted, None)
    };
    CaptureReceipt {
        status,
        retained_bytes,
        offered_bytes,
        policy: "redact_secrets+char_cap/1".into(),
        reason,
    }
}

/// The one deposit path shared by HTTP `/store`, MCP `cortex_store`/`commit`
/// and library callers: store + focus capture + trace/HEAD version + entity
/// ingest + clock projection, then a Receipt whose durability vector reflects
/// the connection's configured profile.
pub fn deposit_decision(
    conn: &mut Connection,
    input: DepositInput<'_>,
) -> Result<DepositOutcome, StoreError> {
    let text = crate::handlers::redact_secrets(input.text.trim());
    let context = input
        .context
        .as_deref()
        .map(crate::handlers::redact_secrets);
    let canonical = canonical_deposit(
        &text,
        context.as_deref(),
        input.entry_type.as_deref(),
        input.owner_id,
        &input.paths,
        input.thread.as_deref(),
        &input.evidence,
    );
    let canonical_hash = cortex_logic::traces::content_hash(&canonical);
    conn.execute_batch(crate::store_spi::sqlite::IDEMPOTENCY_DDL)
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    if let Some(key) = input.idempotency_key.as_deref() {
        match lookup_ledger(conn, &input.principal, key)? {
            Some((stored_hash, receipt_json, entry_json)) if stored_hash == canonical_hash => {
                let receipt: Receipt = serde_json::from_str(&receipt_json)
                    .map_err(|e| StoreError::Internal(e.to_string()))?;
                let entry: Value = serde_json::from_str(&entry_json)
                    .map_err(|e| StoreError::Internal(e.to_string()))?;
                let target_id = receipt
                    .entries
                    .get("decision")
                    .and_then(|id| id.value.parse().ok());
                let capture = capture_receipt(
                    input.text,
                    &text,
                    crate::handlers::store::MAX_DECISION_CHARS_PUB,
                );
                return Ok(DepositOutcome {
                    entry,
                    target_id,
                    receipt,
                    capture,
                });
            }
            Some((stored_hash, _, _))
                if cortex_logic::traces::is_legacy_content_hash(&stored_hash) =>
            {
                // Pre-cutover seals are process-random SipHash: unverifiable by
                // construction, so the payload can be neither confirmed nor
                // denied. Fail closed with a clear epoch message instead of a
                // misleading "different payload" claim.
                return Err(StoreError::BadRequest(
                    "idempotency_conflict: key sealed under a previous hash epoch; retire and reissue the key",
                ));
            }
            Some(_) => {
                return Err(StoreError::BadRequest(
                    "idempotency_conflict: key reused with a different payload",
                ));
            }
            None => {}
        }
    }
    // Backpressure: at hard maintenance debt the intake refuses instead of
    // pretending the projections are fresh. Callers can drain with a
    // maintenance slice (`cortex maintain`, `BrainStore::maintain_slice`).
    let debt = crate::db::outbox::debt(conn);
    if debt.refuse_intake() {
        return Err(StoreError::BadRequest(
            "maintenance_debt_hard_limit: run a maintenance slice before depositing more",
        ));
    }
    // One atomic batch: store + focus + trace/version + entities + clock
    // projection. A failure or panic rolls the whole deposit back.
    let outcome = crate::db::with_savepoint_mut(
        conn,
        "deposit",
        |conn| deposit_inner(conn, &input, &text, context, &canonical_hash),
        |e| StoreError::Internal(e.to_string()),
    )?;
    // Best-effort identity interning for the cross-product sidecar. A sidecar
    // failure warns and never fails the committed store.
    crate::refzero::intern_store_bytes_for_conn(conn, text.as_bytes());
    Ok(outcome)
}

fn canonical_deposit(
    text: &str,
    context: Option<&str>,
    entry_type: Option<&str>,
    owner_id: Option<i64>,
    paths: &[String],
    thread: Option<&str>,
    evidence: &[String],
) -> String {
    // Canonical comparison preserves Unicode and field identity; it is a
    // structured rendering, not an ad hoc string hash of the raw request.
    // The source agent is attribution on the record, not part of the
    // payload: a retry of the same deposit from another surface replays.
    // Paths and thread are the caller's scope: the same sentence in two
    // repositories is two facts. Cited observations are the deposit's
    // provenance: a retry cannot swap them after the identity is sealed.
    let mut paths: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|p| !p.is_empty())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    let mut evidence: Vec<&str> = evidence
        .iter()
        .map(String::as_str)
        .filter(|p| !p.is_empty())
        .collect();
    evidence.sort_unstable();
    evidence.dedup();
    let thread = nonempty_opt(thread);
    let mut body = json!({"schema":"deposit/1","text":text,"context":context,"type":entry_type,"owner":owner_id,"paths":paths,"thread":thread});
    if !evidence.is_empty() {
        body["evidence"] = json!(evidence);
    }
    serde_json::to_string(&body).unwrap_or_default()
}

fn scope_anchors(paths: &[String], thread: Option<&str>) -> Vec<QueryAnchor> {
    let mut extra = Vec::new();
    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        extra.push(QueryAnchor {
            kind: AnchorKind::Path,
            value: crate::clockwork::normalize_anchor_value(AnchorKind::Path, trimmed),
            specificity: 3,
        });
    }
    if let Some(thread) = nonempty_opt(thread) {
        extra.push(QueryAnchor {
            kind: AnchorKind::Session,
            value: thread.to_ascii_lowercase(),
            specificity: 1,
        });
    }
    extra
}

fn lookup_ledger(
    conn: &Connection,
    principal: &str,
    key: &str,
) -> Result<Option<(String, String, String)>, StoreError> {
    use rusqlite::OptionalExtension;
    conn.query_row("SELECT canonical_hash, receipt_json, COALESCE(entry_json, 'null') FROM operation_ledger WHERE principal = ?1 AND idempotency_key = ?2", rusqlite::params![principal, key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).optional().map_err(|e| StoreError::Internal(e.to_string()))
}

fn deposit_inner(
    conn: &mut Connection,
    input: &DepositInput<'_>,
    text: &str,
    context: Option<String>,
    canonical_hash: &str,
) -> Result<DepositOutcome, StoreError> {
    let (mut entry, new_id) = store_decision_with_input_embedding_and_provenance_retention(
        conn,
        text,
        context,
        input.entry_type.clone(),
        input.source_agent.clone(),
        input.provenance.clone(),
        input.confidence,
        input.ttl_seconds,
        input.retention_class,
        None,
        input.owner_id,
        &input.paths,
    )?;
    if !input.benchmark {
        crate::focus::focus_append(conn, &input.source_agent, text);
    }
    let target_id = new_id.or_else(|| entry.get("id").and_then(|v| v.as_i64()));
    let action = entry
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("stored")
        .to_string();
    let version_id = crate::traces::record_store_write(
        conn,
        &input.source_agent,
        text,
        &action,
        "decision",
        target_id,
        input.owner_id,
    );
    if let Some(version_id) = version_id {
        entry["versionId"] = json!(version_id);
    }
    crate::graph::ingest_for_target(conn, text, "decision", target_id, None, input.owner_id);
    if let Some(id) = target_id {
        let mut anchors = input.anchors.clone();
        anchors.extend(scope_anchors(&input.paths, input.thread.as_deref()));
        let origin = if anchors.is_empty() {
            ClockOrigin::DeterministicExtract
        } else {
            ClockOrigin::Explicit
        };
        crate::clockwork::project_target(conn, text, &anchors, "decision", id, origin, None)
            .map_err(|err| {
                StoreError::Internal(format!("clock projection failed for {id}: {err}"))
            })?;
        // Authoritative tables: the commit row, then a record/revision/head for
        // the decision. A merge/refine produces a successor revision of the
        // existing record; a new decision a baseline revision.
        record_authoritative(conn, input, id, text, &action)
            .map_err(|e| StoreError::Internal(format!("authoritative record: {e}")))?;
        // Loop 5: threaded deposits join the thread as members, so the
        // activity arm can route queries to what was alive with them.
        if let Some(label) = nonempty_opt(input.thread.as_deref()) {
            let frontier = crate::store_spi::sqlite::current_frontier(conn);
            let seq = crate::store_spi::sqlite::frontier_sequence(&frontier);
            let thread_id = crate::db::threads::ensure_thread(conn, seq, label)
                .map_err(|e| StoreError::Internal(format!("thread ensure: {e}")))?;
            if let Some(record_id) =
                crate::db::records::record_for_legacy(conn, "decision", id)
                    .map_err(|e| StoreError::Internal(format!("legacy map: {e}")))?
            {
                crate::db::threads::add_thread_member(conn, &thread_id, &record_id, "deposit")
                    .map_err(|e| StoreError::Internal(format!("thread join: {e}")))?;
            }
        }
    }
    let receipt = build_receipt(conn, input.request_id, target_id, version_id, &action);
    let capture = capture_receipt(
        input.text,
        text,
        crate::handlers::store::MAX_DECISION_CHARS_PUB,
    );
    if let Some(key) = input.idempotency_key.as_deref() {
        conn.execute("INSERT INTO operation_ledger (principal, idempotency_key, request_id, canonical_hash, receipt_json, entry_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![input.principal, key, input.request_id, canonical_hash, serde_json::to_string(&receipt).unwrap_or_default(), entry.to_string()]).map_err(|e| StoreError::Internal(e.to_string()))?;
    }
    Ok(DepositOutcome {
        entry,
        target_id,
        receipt,
        capture,
    })
}

pub fn ack_profile_label(profile: &AckProfile) -> &'static str {
    match profile {
        AckProfile::ProcessCrash => "process_crash",
        AckProfile::PowerLossAssumed { .. } => "power_loss_assumed",
    }
}

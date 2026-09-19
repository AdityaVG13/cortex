use super::super::*;
use super::origin;
use crate::runtime::CortexRuntime;
use crate::runtime::observation::{self, MAX_BATCH_EVENTS, SourceSpec};
use asupersync::Cx;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl CortexRuntime {
    pub(super) async fn capture_host_batch(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        route: HostRoute,
        start: Option<u64>,
        raw: &[u8],
    ) -> Result<HostCaptureReceipt, String> {
        if raw.len() > MAX_CAPTURE_BYTES {
            return Err("capture_batch_byte_limit".into());
        }
        let receipt = self.with_db_tx(
            cx,
            TransactionBehavior::Immediate,
            |conn| {
                observation::ensure(conn)?;
                conn.execute_batch(DDL).map_err(|e| e.to_string())
            },
            |tx, principal| {
        let stored: String = grant_spec_json(&tx, principal, grant_key).optional().map_err(|e| e.to_string())?.ok_or("host_capture_not_authorized")?;
        let grant: HostCaptureGrant = serde_json::from_str(&stored).map_err(|e| e.to_string())?;
        validate(&grant, context, route)?;
        let cursor = cursor_key(grant_key);
        let generation = generation(context);
        observation::granted(&tx, &principal, &cursor, true)?;
        let cutoff = if start.is_some() {
            raw.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1)
        } else {
            raw.len()
        };
        let next = if let Some(start) = start {
            if observation::offset(&tx, &principal, &cursor, &generation)? != start { return Err("cursor_conflict".into()); }
            Some(start.checked_add(cutoff as u64).filter(|v| *v <= i64::MAX as u64).ok_or("invalid_source_cursor")?)
        } else {
            None
        };
        let lines: Vec<&[u8]> = if start.is_some() {
            raw[..cutoff].split_inclusive(|b| *b == b'\n').collect()
        } else {
            vec![raw]
        };
        if lines.len() > MAX_BATCH_EVENTS { return Err("capture_batch_event_limit".into()); }
        let mut result = HostCaptureReceipt {
            adapter_version: grant.adapter_version.clone(),
            accepted: vec![],
            excluded_deliveries: 0,
            ignored_metadata: 0,
            next_offset: next,
            uncommitted_tail_bytes: raw.len() - cutoff,
        };
        let mut raw_offset = start.unwrap_or(0);
        for line in lines {
            let line_start = raw_offset;
            raw_offset += line.len() as u64;
            ingest_host_line(cx, &tx, &principal, grant_key, &generation, &grant, context, route, line_start, line, &mut result)?;
        }
        if let Some(next) = next {
            tx.execute("INSERT INTO observation_cursors VALUES(?1,?2,?3,?4) ON CONFLICT(principal,source_key,generation) DO UPDATE SET byte_offset=excluded.byte_offset",params![principal,cursor,generation,next as i64]).map_err(|e| e.to_string())?;
        }
            Ok(result)
            },
        ).await?;
        // Best-effort identity interning for the cross-product sidecar. A
        // sidecar failure warns and never fails the committed capture.
        crate::refzero::intern_store_bytes_for_db(&self.state().db_path, raw);
        Ok(receipt)
    }
}

fn ingest_host_line(
    cx: &Cx,
    tx: &rusqlite::Transaction<'_>,
    principal: &str,
    grant_key: &str,
    generation: &str,
    grant: &HostCaptureGrant,
    context: &HostCaptureContext,
    route: HostRoute,
    line_start: u64,
    line: &[u8],
    result: &mut HostCaptureReceipt,
) -> Result<(), String> {
    if route == HostRoute::History {
        cx.checkpoint().map_err(|e| e.to_string())?;
        if let Some(kind) = metadata_kind(grant, context, line)? {
            insert_host_metadata(
                tx,
                principal,
                grant_key,
                generation,
                line_start as i64,
                kind,
                line,
            )?;
            result.ignored_metadata += 1;
            return Ok(());
        }
    }
    if route == HostRoute::Live && live_hook_name(line)?.as_deref() == Some("PreCompact") {
        let value = parse_host_json(line)?;
        if field(&value, "session_id")? != context.session_id {
            return Err("host_session_mismatch".into());
        }
        field(&value, "trigger")?;
        // Live PreCompact is not a transcript byte. History metadata
        // uses non-negative line starts; MAX+1 collides with the
        // first history line (offset 0) or a later line at MAX+1.
        // Negative offsets keep the two writers in disjoint keyspaces.
        let next: i64 = tx.query_row("SELECT COALESCE(MIN(byte_offset), 0) - 1 FROM host_capture_metadata WHERE principal=?1 AND grant_key=?2 AND generation=?3 AND byte_offset < 0", params![principal, grant_key, generation], |r| r.get(0)).map_err(|e| e.to_string())?;
        insert_host_metadata(
            tx,
            principal,
            grant_key,
            generation,
            next,
            "precompact",
            line,
        )?;
        result.ignored_metadata += 1;
        return Ok(());
    }
    cx.checkpoint().map_err(|e| e.to_string())?;
    let normalized = normalize_host_event(grant, context, route, line)?;
    let source = if let Some(existing) = existing_host_source(
        tx,
        principal,
        generation,
        &normalized.event.event_key,
        grant_key,
        normalized.kind,
    )? {
        existing
    } else {
        let scope = observation_scope_for_raw(grant, line);
        let source = scoped_source_key(grant, normalized.kind, &scope);
        observation::ensure_source(
            tx,
            principal,
            &SourceSpec {
                key: source.clone(),
                scope,
                role: normalized.kind.role(),
                max_bytes: grant.max_bytes,
            },
        )?;
        source
    };
    let registered = observation::granted(tx, principal, &source, true)?;
    if registered.role != normalized.kind.role_name() {
        return Err("host_source_role_mismatch".into());
    }
    let resolved = origin(context, &normalized.event.event_key)?;
    let origin_name = match resolved {
        HostOrigin::CortexDelivery => "cortex_delivery",
        _ => "external",
    };
    let previous: Option<(String, String)> = tx.query_row("SELECT kind,origin FROM host_capture_origins WHERE principal=?1 AND grant_key=?2 AND generation=?3 AND event_key=?4", params![principal, grant_key, generation, normalized.event.event_key], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(|e| e.to_string())?;
    if previous
        .as_ref()
        .is_some_and(|(kind, origin)| kind != normalized.kind.role_name() || origin != origin_name)
    {
        return Err("host_origin_identity_conflict".into());
    }
    tx.execute(
        "INSERT OR IGNORE INTO host_capture_origins VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            principal,
            grant_key,
            generation,
            normalized.event.event_key,
            normalized.kind.role_name(),
            origin_name
        ],
    )
    .map_err(|e| e.to_string())?;
    if resolved == HostOrigin::CortexDelivery {
        result.excluded_deliveries += 1;
        return Ok(());
    }
    result.accepted.push(observation::capture(
        tx,
        principal,
        &source,
        generation,
        &registered,
        normalized.event,
    )?);
    Ok(())
}

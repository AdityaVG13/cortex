use super::{
    CapturedObservation, MAX_BATCH_EVENTS, MAX_CAPTURE_BYTES, ObservationEvent, ObservationReceipt,
    SourceSpec, TailReceipt, capture, capture_registered, check_label, ensure, ensure_source,
    granted, offset,
};
use crate::runtime::CortexRuntime;
use asupersync::Cx;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl CortexRuntime {
    pub fn observation_principal(&self) -> Result<String, String> {
        match (self.state().team_mode, self.state().default_owner_id) {
            (true, Some(id)) => Ok(format!("user:{id}")),
            (true, None) => Err("local_owner_required".into()),
            (false, _) => Ok("local".into()),
        }
    }

    /// Explicit local operator registration. Observation bodies cannot create or widen it.
    pub async fn register_source(&self, cx: &Cx, spec: SourceSpec) -> Result<(), String> {
        self.with_db_tx(
            cx,
            TransactionBehavior::Immediate,
            |_| Ok(()),
            |tx, principal| {
                ensure_source(tx, principal, &spec)?;
                Ok(())
            },
        )
        .await
    }

    pub async fn set_source_enabled(
        &self,
        cx: &Cx,
        source: &str,
        enabled: bool,
    ) -> Result<(), String> {
        self.with_locked_db(cx, |conn, principal| {
            ensure(conn)?;
            let changed = conn.execute(&format!("UPDATE observation_sources SET enabled=?3, policy_epoch=CASE WHEN ?3=1 THEN ({}) ELSE policy_epoch END WHERE principal=?1 AND source_key=?2", crate::db::records::POLICY_EPOCH_SELECT), params![principal, source, enabled]).map_err(|err| err.to_string())?;
            if changed == 0 { return Err("source_not_authorized".into()); }
            Ok(())
        }).await
    }

    /// Import one explicitly registered file; file content cannot supply a principal or grant.
    pub async fn observe_file(
        &self,
        cx: &Cx,
        path: &std::path::Path,
    ) -> Result<ObservationReceipt, String> {
        let owner = if self.state().team_mode {
            self.state().default_owner_id
        } else {
            None
        };
        self.with_locked_db(cx, |conn, _principal| {
            crate::indexer::index_file(conn, path, owner)
        })
        .await
    }

    pub async fn observe(
        &self,
        cx: &Cx,
        source: &str,
        generation: &str,
        event: ObservationEvent,
    ) -> Result<ObservationReceipt, String> {
        self.with_locked_db(cx, |conn, principal| {
            capture_registered(conn, principal, source, generation, event)
        })
        .await
    }

    pub async fn source_offset(
        &self,
        cx: &Cx,
        source: &str,
        generation: &str,
    ) -> Result<u64, String> {
        self.with_locked_db(cx, |conn, principal| {
            ensure(conn)?;
            granted(conn, principal, source, false)?;
            offset(conn, principal, source, generation)
        })
        .await
    }

    /// Normalized JSONL only. A trailing partial record remains at its source.
    pub async fn tail_observations(
        &self,
        cx: &Cx,
        source: &str,
        generation: &str,
        start: u64,
        chunk: &[u8],
    ) -> Result<TailReceipt, String> {
        check_label(source)?;
        check_label(generation)?;
        if chunk.len() > MAX_CAPTURE_BYTES {
            return Err("capture_batch_byte_limit".into());
        }
        let cutoff = chunk
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |i| i + 1);
        let mut events = Vec::new();
        for line in chunk[..cutoff].split(|byte| *byte == b'\n') {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if events.len() == MAX_BATCH_EVENTS {
                return Err("capture_batch_event_limit".into());
            }
            events.push(
                serde_json::from_slice::<ObservationEvent>(line)
                    .map_err(|err| format!("malformed_complete_record: {err}"))?,
            );
        }
        let next = start
            .checked_add(cutoff as u64)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or("invalid_source_cursor")?;
        self.with_db_tx(cx, TransactionBehavior::Immediate, ensure, |tx, principal| {
            let grant = granted(tx, principal, source, true)?;
            if offset(tx, principal, source, generation)? != start { return Err("cursor_conflict".into()); }
            let mut accepted = Vec::new();
            for event in events {
                cx.checkpoint().map_err(|err| err.to_string())?;
                accepted.push(capture(tx, principal, source, generation, &grant, event)?);
            }
            tx.execute("INSERT INTO observation_cursors VALUES(?1,?2,?3,?4) ON CONFLICT(principal,source_key,generation) DO UPDATE SET byte_offset=excluded.byte_offset", params![principal,source,generation,next as i64]).map_err(|err| err.to_string())?;
            Ok(TailReceipt { accepted, next_offset: next, uncommitted_tail_bytes: chunk.len() - cutoff })
        }).await
    }

    pub async fn read_observation(
        &self,
        cx: &Cx,
        source_id: &str,
    ) -> Result<CapturedObservation, String> {
        self.with_db_tx(cx, TransactionBehavior::Deferred, ensure, |tx, principal| {
            let (key, generation, body, available): (String, String, String, String) = tx.query_row("SELECT e.source_key,e.generation,r.body_json,s.availability FROM observation_events e JOIN revisions r ON r.revision_id=e.revision_id JOIN sources s ON s.source_id=e.source_id WHERE e.principal=?1 AND e.source_id=?2", params![principal,source_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|err| err.to_string())?.ok_or("source_not_authorized")?;
            let grant = granted(tx, principal, &key, false)?;
            if available != "owned_inline" { return Err("source_unavailable".into()); }
            let body: serde_json::Value = serde_json::from_str(&body).map_err(|err| err.to_string())?;
            let event: ObservationEvent = serde_json::from_value(body["observation"].clone()).map_err(|err| err.to_string())?;
            Ok(CapturedObservation { source_id: source_id.into(), source_key: key, scope_id: grant.scope_id, generation, role: grant.role, event_key: event.event_key, text: event.text, observed_at: event.observed_at })
        }).await
    }
}

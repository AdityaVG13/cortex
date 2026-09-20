use super::{View, legacy_record};
use crate::presence::{
    CHANGE_RULE_VERSION, ChangeCursor, CurrentEpochs, CursorError, PresenceDecision, decide,
};
use crate::protocol::{LogicalId, ResponseStatus};
use rusqlite::{Connection, params};
use serde_json::json;

impl View {
    /// Coverage watermark per partition at this connection's frontier.
    pub fn watermark(&mut self, conn: &Connection) {
        let frontier = crate::store_spi::sqlite::current_frontier(conn);
        let seq = crate::store_spi::sqlite::frontier_sequence(&frontier);
        let max_decision: i64 = conn
            .query_row("SELECT COALESCE(MAX(id),0) FROM decisions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let max_memory: i64 = conn
            .query_row("SELECT COALESCE(MAX(id),0) FROM memories", [], |r| r.get(0))
            .unwrap_or(0);
        let archived: i64 = crate::db::count_or_zero(
            conn,
            "SELECT COUNT(*) FROM decisions WHERE status IN ('archived','superseded')",
        );
        let cold_segments = crate::db::cold::cold_count(conn);
        let searched_cold = self.include_cold;
        let exhausted = self.coverage.partitions["exhausted"]
            .as_bool()
            .unwrap_or(true);
        let routes = self.coverage.partitions["routes"].clone();
        self.coverage.partitions = json!({"routes":routes,"decisions":{"source_frontier":seq,"index_frontier":seq,"searched_range":{"ids_through":max_decision},"continuation":null,"limits_hit":!exhausted,"archive_policy":"active_only","exhausted":exhausted},"memories":{"source_frontier":seq,"index_frontier":seq,"searched_range":{"ids_through":max_memory},"continuation":null,"limits_hit":!exhausted,"archive_policy":"active_only","exhausted":exhausted},"cold":{"searched":searched_cold,"rows_not_searched":if searched_cold { 0 } else { archived },"cold_segments":cold_segments,"reason":if searched_cold { "cold partition included" } else { "archived/superseded rows are outside the active partition; ask with profile=history or time" }}});
    }

    /// Bind aliases to a receipt row scoped to the principal and brain epoch.
    /// The asking need is stored (bounded) so outcome feedback can resolve
    /// which query a receipt answered without trusting the client to repeat it.
    pub fn persist_receipt(
        &mut self,
        conn: &Connection,
        principal: &str,
        need: &str,
    ) -> rusqlite::Result<()> {
        crate::db::records::ensure_authoritative_schema(conn)?;
        let (_, restore_epoch, policy_epoch) = crate::db::records::brain_epochs(conn);
        self.apply_change_cursor(conn, principal, &restore_epoch);
        let frontier = crate::store_spi::sqlite::current_frontier(conn);
        let seq = crate::store_spi::sqlite::frontier_sequence(&frontier);
        let random: String =
            conn.query_row("SELECT lower(hex(randomblob(4)))", [], |r| r.get(0))?;
        let receipt_id = format!("view-{seq}-{random}");
        self.frontier = Some(json!(frontier));
        let sp = crate::db::SqliteSavepoint::enter(conn, "view_receipt")?;
        let need: String = need.chars().take(512).collect();
        conn.execute("INSERT INTO view_receipts (receipt_id, principal_id, brain_epoch, through_sequence, receipt_json) VALUES (?1, ?2, ?3, ?4, ?5)", params![receipt_id, principal, restore_epoch, seq, json!({"profile": self.profile, "cards": self.cards.len(), "need": need}).to_string()])?;
        for card in &mut self.cards {
            card.expandable = false;
            let Some(record_id) = legacy_record(conn, &card.reference)? else {
                continue;
            };
            let heads = crate::db::records::heads(conn, &record_id)?;
            let Some(revision) = heads.first() else {
                continue;
            };
            conn.execute("INSERT OR REPLACE INTO view_aliases (receipt_id, alias, record_id, revision_id, representation_version) VALUES (?1, ?2, ?3, ?4, 'brief/1')", params![receipt_id, card.alias, record_id, revision])?;
            card.expandable = true;
        }
        // Presence is a transport saving. Bind aliases first so a suppressed
        // Card in `present` can still be expanded by alias+receipt.
        self.apply_presence(conn, &restore_epoch, &policy_epoch)?;
        sp.release()?;
        self.receipt_id = receipt_id;
        Ok(())
    }

    /// Per-Card presence decision. A suppressed Card keeps its alias and
    /// revision in `present` so the agent can still refer to it; its payload
    /// is omitted from the transport only.
    fn apply_presence(
        &mut self,
        conn: &Connection,
        restore_epoch: &str,
        policy_epoch: &str,
    ) -> rusqlite::Result<()> {
        let Some(inputs) = self.presence.clone() else {
            return Ok(());
        };
        let current = CurrentEpochs {
            brain_epoch: restore_epoch.to_string(),
            policy_epoch: policy_epoch.to_string(),
        };
        let context_epoch = inputs.context_epoch.clone().unwrap_or_default();
        let mut i = 0;
        while i < self.cards.len() {
            let revision = legacy_record(conn, &self.cards[i].reference)?
                .and_then(|record| crate::db::records::heads(conn, &record).ok())
                .and_then(|h| h.first().cloned());
            let decision = match revision.as_ref() {
                Some(rev) => decide(
                    inputs.presence.as_ref(),
                    inputs.attested_brain.as_deref(),
                    inputs.attested_policy.as_deref(),
                    &current,
                    &context_epoch,
                    &LogicalId::new("revision", rev),
                    "brief/1",
                ),
                None => PresenceDecision::DeliverRepresentationDiffers,
            };
            if decision.suppresses() {
                let card = self.cards.remove(i);
                self.present.push(json!({"alias": card.alias, "label": card.label, "revision": revision, "representation": "brief/1", "decision": decision}));
            } else {
                i += 1;
            }
        }
        self.used_bytes = self.cards.iter().map(|c| c.bytes).sum();
        Ok(())
    }

    /// Change cursor: changes since the cursor under the same restore epoch,
    /// scope/filter identity and rule version; otherwise `resnapshot_required`
    /// and the View stays self-contained.
    fn apply_change_cursor(&mut self, conn: &Connection, principal: &str, restore_epoch: &str) {
        let scope_filter = format!("{principal}:{}", self.profile);
        let frontier = crate::store_spi::sqlite::current_frontier(conn);
        let seq = crate::store_spi::sqlite::frontier_sequence(&frontier);
        let out = ChangeCursor {
            restore_epoch: restore_epoch.to_string(),
            scope_filter: scope_filter.clone(),
            sequence: seq,
            rule_version: CHANGE_RULE_VERSION.into(),
        };
        self.change_cursor_out = Some(out.encode());
        let Some(raw) = self.change_cursor_in.clone() else {
            return;
        };
        match ChangeCursor::decode(&raw).and_then(|c| c.validate(restore_epoch, &scope_filter)) {
            Ok(since) => {
                // A failed versions read is not "nothing changed": that would
                // hide durable work behind an Ok empty delta.
                match conn.prepare("SELECT v.id, v.target_type, v.target_id, v.op FROM versions v WHERE v.id > ?1 ORDER BY v.id LIMIT 200") {
                    Ok(mut stmt) => match stmt.query_map(params![since], |r| Ok(json!({"sequence": r.get::<_, i64>(0)?, "target": format!("{}::{}", r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, Option<i64>>(2)?.unwrap_or(0)), "action": r.get::<_, String>(3)?}))) {
                        Ok(rows) => match rows.collect::<Result<Vec<_>, _>>() {
                            Ok(changes) => {
                                self.changes = changes;
                                self.cursor_status = Some(ResponseStatus::Ok);
                            }
                            Err(_) => self.cursor_status = Some(ResponseStatus::Unavailable),
                        },
                        Err(_) => self.cursor_status = Some(ResponseStatus::Unavailable),
                    },
                    Err(_) => self.cursor_status = Some(ResponseStatus::Unavailable),
                }
            }
            Err(CursorError::Malformed) => {
                self.cursor_status = Some(ResponseStatus::InvalidRequest)
            }
            Err(CursorError::ResnapshotRequired { .. }) => {
                self.cursor_status = Some(ResponseStatus::ResnapshotRequired)
            }
        }
    }
}

use super::*;
use crate::protocol::{AckProfile, DurabilityVector, LogicalId, PayloadAvailability, Receipt};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;

impl SqliteTx<'_> {
    pub(super) fn head_of(
        conn: &Connection,
        record: &LogicalId,
    ) -> Result<Option<LogicalId>, StoreSpiError> {
        let Ok(id) = record.value.parse::<i64>() else {
            return Ok(None);
        };
        let Some(table) = super::scan::legacy_table(&record.namespace) else {
            return Ok(None);
        };
        conn.query_row(
            &format!(
                "SELECT version_id FROM {table} WHERE id = ?1",
                table = table.table
            ),
            params![id],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(|e| StoreSpiError::Unavailable(e.to_string()))
        .map(|v| v.flatten().map(|v| LogicalId::from_legacy("version", v)))
    }

    fn stamp_insert(
        &mut self,
        table: &str,
        namespace: &str,
        local_name: String,
        text: &str,
        agent: &str,
        owner_id: Option<i64>,
    ) -> Result<(), StoreSpiError> {
        let id = self.conn.last_insert_rowid();
        let version = crate::traces::record_store_write(
            self.conn,
            agent,
            text,
            "stored",
            namespace,
            Some(id),
            owner_id,
        );
        if let Some(v) = version {
            let stamped = self
                .conn
                .execute(
                    &format!("UPDATE {table} SET version_id = ?1 WHERE id = ?2"),
                    params![v, id],
                )
                .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
            if stamped == 0 {
                return Err(StoreSpiError::Unavailable(format!(
                    "{namespace} {id} missing after insert"
                )));
            }
        }
        self.entries
            .insert(local_name, LogicalId::from_legacy(namespace, id));
        Ok(())
    }
}

impl WriteTransaction for SqliteTx<'_> {
    fn apply(&mut self, op: Op) -> Result<(), StoreSpiError> {
        self.canonical.push(format!("{op:?}"));
        match op {
            Op::InsertDecision {
                local_name,
                text,
                context,
                agent,
                owner_id,
            } => {
                self.conn.execute("INSERT INTO decisions (decision, context, type, source_agent, status, owner_id) VALUES (?1, ?2, 'decision', ?3, 'active', ?4)", params![text, context, agent, owner_id]).map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                self.stamp_insert("decisions", "decision", local_name, &text, &agent, owner_id)?;
            }
            Op::InsertMemory {
                local_name,
                text,
                kind,
                agent,
                owner_id,
            } => {
                self.conn.execute("INSERT INTO memories (text, type, source, source_agent, status, owner_id) VALUES (?1, ?2, 'spi', ?3, 'active', ?4)", params![text, kind, agent, owner_id]).map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                self.stamp_insert("memories", "memory", local_name, &text, &agent, owner_id)?;
            }
            Op::SetStatus { target, status } => {
                let Some(table) = super::scan::legacy_table(&target.namespace) else {
                    return Err(StoreSpiError::LimitExceeded(format!(
                        "unknown record namespace {}",
                        target.namespace
                    )));
                };
                let id: i64 = target
                    .value
                    .parse()
                    .map_err(|_| StoreSpiError::LimitExceeded("non-numeric legacy id".into()))?;
                let updated = self.conn.execute(&format!("UPDATE {table} SET status = ?1, updated_at = datetime('now') WHERE id = ?2", table = table.table), params![status, id]).map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
                if updated == 0 {
                    return Err(StoreSpiError::LimitExceeded(format!(
                        "unknown record {}",
                        target.canonical()
                    )));
                }
                let _ = crate::traces::record_store_write(
                    self.conn,
                    &self.intent.principal,
                    &status,
                    "status",
                    table.ns,
                    Some(id),
                    None,
                );
            }
        }
        Ok(())
    }

    fn commit(mut self, durability: Durability) -> Result<Receipt, StoreSpiError> {
        let frontier = current_frontier(self.conn);
        let receipt = Receipt {
            receipt_id: LogicalId::new("receipt", frontier_sequence(&frontier).to_string()),
            request_id: self.intent.request_id.clone(),
            durability: DurabilityVector {
                accepted: true,
                local_commit: Some(frontier.clone()),
                projected_through: [("exact".to_string(), frontier.clone())]
                    .into_iter()
                    .collect(),
                replicated_through: BTreeMap::new(),
                payload_availability: PayloadAvailability::Retained,
                ack_profile: match durability {
                    Durability::PowerLossAssumed => AckProfile::PowerLossAssumed {
                        platform_profile: "sqlite-wal-synchronous-full".into(),
                    },
                    Durability::ProcessCrash => AckProfile::ProcessCrash,
                },
            },
            entries: std::mem::take(&mut self.entries),
            aliases: BTreeMap::new(),
            omissions: Vec::new(),
            unresolved_needs: Vec::new(),
        };
        if let Some(key) = &self.intent.idempotency_key {
            let hash = cortex_logic::traces::content_hash(&self.canonical.join("\n"));
            let receipt_json = serde_json::to_string(&receipt)
                .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
            self.conn.execute("INSERT INTO operation_ledger (principal, idempotency_key, request_id, canonical_hash, receipt_json) VALUES (?1, ?2, ?3, ?4, ?5)", params![self.intent.principal, key, self.intent.request_id, hash, receipt_json]).map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        }
        if matches!(durability, Durability::PowerLossAssumed) {
            let _ = self.conn.execute_batch("PRAGMA synchronous = FULL");
        }
        self.conn
            .execute_batch("COMMIT")
            .map_err(|e| StoreSpiError::Unavailable(e.to_string()))?;
        self.finished = true;
        Ok(receipt)
    }

    fn abort(mut self) {
        let _ = self.conn.execute_batch("ROLLBACK");
        // Only mark finished when the connection is actually autocommit.
        // A failed ROLLBACK with a live statement would otherwise skip Drop's
        // retry and leave the next lock holder inside the write transaction.
        self.finished = self.conn.is_autocommit();
    }
}

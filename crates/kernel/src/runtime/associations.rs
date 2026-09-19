//! Optional, bounded local co-occurrence routes. These are navigation hints, not
//! aliases approved as facts, causal credit, authority, or independent witnesses.
use super::CortexRuntime;
use asupersync::Cx;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

const MAX_SOURCES: usize = 512;
const MAX_TOKENS: usize = 32;
const MAX_CUES: usize = 16;
const MAX_RESULTS: usize = 64;
const DDL: &str = "
CREATE TABLE IF NOT EXISTS observation_association_state (
 principal TEXT NOT NULL, scope_label TEXT NOT NULL, enabled INTEGER NOT NULL,
 PRIMARY KEY(principal,scope_label));
CREATE TABLE IF NOT EXISTS observation_association_incidence (
 principal TEXT NOT NULL, scope_label TEXT NOT NULL, source_id TEXT NOT NULL,
 digest TEXT NOT NULL, tokens_json TEXT NOT NULL,
 PRIMARY KEY(principal,scope_label,source_id));";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AssociationExplanation {
    pub source_id: String,
    pub route: String,
    pub cue: String,
    pub alias: String,
    /// Distinct exact texts from distinct registered source lineages; not CQR witnesses.
    pub support_sources: Vec<String>,
    pub score: f64,
}

mod load;
pub(crate) use load::maintain_in_transaction;
use load::*;
mod explain;
pub(crate) use explain::candidates;
use explain::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssociationAssessment {
    Useful,
    Harmful,
    Neutral,
}

impl CortexRuntime {
    /// Explicit attributed usefulness, never inferred from exposure or subsequent tool success.
    pub async fn record_association_feedback(
        &self,
        cx: &Cx,
        scope: &str,
        event_key: &str,
        source_id: &str,
        assessment: AssociationAssessment,
    ) -> Result<bool, String> {
        if scope.is_empty() || scope.len() > 1024 || event_key.is_empty() || event_key.len() > 256 {
            return Err("invalid_feedback_identity".into());
        }
        self.with_locked_db(cx, |conn, principal| {
            ensure(conn)?;
            ensure_feedback(conn)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
            if !evidence(&tx, principal, scope)?.iter().any(|e| e.id == source_id) { return Err("feedback_source_not_eligible".into()); }
            let value = match assessment { AssociationAssessment::Useful => 1, AssociationAssessment::Harmful => -1, AssociationAssessment::Neutral => 0 };
            let previous:Option<(String,i64)>=tx.query_row("SELECT source_id,value FROM observation_association_feedback WHERE principal=?1 AND scope_label=?2 AND event_key=?3",params![principal,scope,event_key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
            if let Some((old_target, old_value)) = previous {
                if old_target != source_id || old_value != value { return Err("feedback_identity_conflict".into()); }
                return Ok(false);
            }
            tx.execute("INSERT INTO observation_association_feedback(principal,scope_label,event_key,source_id,value) VALUES(?1,?2,?3,?4,?5)",params![principal,scope,event_key,source_id,value]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(true)
        }).await
    }

    pub async fn retract_association_feedback(
        &self,
        cx: &Cx,
        scope: &str,
        event_key: &str,
    ) -> Result<(), String> {
        self.with_locked_db(cx, |conn, principal| {
            ensure_feedback(conn)?;
            if conn.execute("UPDATE observation_association_feedback SET active=0 WHERE principal=?1 AND scope_label=?2 AND event_key=?3",params![principal,scope,event_key]).map_err(|e|e.to_string())?==0 {return Err("feedback_not_found".into());}
            Ok(())
        }).await
    }
    /// Operator opt-in/re-enable and atomically replace only this scoped projection.
    pub async fn rebuild_associations(&self, cx: &Cx, scope: &str) -> Result<usize, String> {
        self.with_db_tx(
            cx,
            TransactionBehavior::Immediate,
            ensure,
            |tx, principal| {
                crate::runtime::upsert_scope_enabled(
                    tx,
                    "observation_association_state",
                    principal,
                    scope,
                    1,
                )?;
                refresh(tx, principal, scope)
            },
        )
        .await
    }
    /// Delete derived incidence only, and prevent automatic refresh until rebuild.
    pub async fn reset_associations(&self, cx: &Cx, scope: &str) -> Result<(), String> {
        self.with_db_tx(cx, TransactionBehavior::Immediate, ensure, |tx, principal| {
            crate::runtime::upsert_scope_enabled(tx, "observation_association_state", principal, scope, 0)?;
            tx.execute("DELETE FROM observation_association_incidence WHERE principal=?1 AND scope_label=?2", params![principal, scope]).map_err(|e| e.to_string())?;
            Ok(())
        }).await
    }
    pub async fn explain_associations(
        &self,
        cx: &Cx,
        scope: &str,
        cues: &[String],
        limit: usize,
    ) -> Result<Vec<AssociationExplanation>, String> {
        self.with_db_tx(
            cx,
            TransactionBehavior::Deferred,
            |_| Ok(()),
            |tx, principal| explain(tx, principal, scope, cues, limit),
        )
        .await
    }
}

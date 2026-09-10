//! Optional, bounded local co-occurrence routes. These are navigation hints, not
//! aliases approved as facts, causal credit, authority, or independent witnesses.
use super::CortexRuntime;
use asupersync::Cx;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

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

fn ensure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DDL).map_err(|e| e.to_string())
}
fn exists(conn: &Connection, table: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}
fn enabled(conn: &Connection, principal: &str, scope: &str) -> Result<bool, String> {
    Ok(conn.query_row("SELECT enabled FROM observation_association_state WHERE principal=?1 AND scope_label=?2", params![principal, scope], |r| r.get::<_, bool>(0)).optional().map_err(|e| e.to_string())?.unwrap_or(true))
}
fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| s.len() >= 3 && s.len() <= 64)
        .take(2048)
        .map(str::to_lowercase)
        .filter(|s| {
            !matches!(
                s.as_str(),
                "the" | "and" | "that" | "this" | "with" | "from" | "for" | "are" | "was" | "not"
            )
        })
        .take(MAX_TOKENS)
        .collect()
}
struct Evidence {
    id: String,
    lineage: String,
    digest: String,
    tokens: BTreeSet<String>,
}

// Exact rows and CURRENT policy are always consulted, including for supporters.
// Unresolved agent/tool derivation is conservatively not independent evidence.
fn evidence(conn: &Connection, principal: &str, scope: &str) -> Result<Vec<Evidence>, String> {
    if !exists(conn, "observation_events")? {
        return Ok(Vec::new());
    }
    let retract = if exists(conn, "observation_retractions")? {
        " AND NOT EXISTS(SELECT 1 FROM observation_retractions x WHERE x.source_id=e.source_id)"
    } else {
        ""
    };
    let sql = format!(
        "SELECT e.source_id,e.source_key,s.inline_payload
        FROM observation_events e
        JOIN observation_sources o ON o.principal=e.principal AND o.source_key=e.source_key
        JOIN sources s ON s.source_id=e.source_id
        JOIN scopes sc ON sc.scope_id=s.scope_id
        JOIN revisions r ON r.revision_id=e.revision_id
        JOIN record_heads h ON h.revision_id=r.revision_id AND h.record_id=r.record_id
        WHERE e.principal=?1 AND o.scope_label=?2 AND sc.owner_id=?1
        AND o.scope_id=s.scope_id AND o.enabled=1
        AND o.policy_epoch=(SELECT policy_epoch FROM brain_meta WHERE singleton=1)
        AND o.role IN ('document','user_statement')
        AND json_extract(r.body_json,'$.role')=o.role
        AND r.epistemic_status!='retracted' AND s.availability='owned_inline'
        AND length(s.inline_payload)<=65536
        AND COALESCE((SELECT state FROM capture_policy WHERE scope IN (?2,'*')
          ORDER BY CASE WHEN scope=?2 THEN 0 ELSE 1 END LIMIT 1),'active')!='stopped'
        {retract} ORDER BY s.capture_sequence DESC,e.source_id LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![principal, scope, MAX_SOURCES as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for row in rows {
        let (id, lineage, bytes) = row.map_err(|e| e.to_string())?;
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        result.push(Evidence {
            id,
            lineage,
            digest: Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            tokens: tokens(text),
        });
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssociationAssessment {
    Useful,
    Harmful,
    Neutral,
}

fn ensure_feedback(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS observation_association_feedback(principal TEXT NOT NULL,scope_label TEXT NOT NULL,event_key TEXT NOT NULL,source_id TEXT NOT NULL,value INTEGER NOT NULL CHECK(value BETWEEN -1 AND 1),active INTEGER NOT NULL DEFAULT 1,PRIMARY KEY(principal,scope_label,event_key))").map_err(|e|e.to_string())
}
pub(super) fn maintain_in_transaction(
    conn: &Connection,
    principal: &str,
    scope: &str,
) -> Result<usize, String> {
    ensure(conn)?;
    if !enabled(conn, principal, scope)? {
        return Ok(0);
    }
    refresh(conn, principal, scope)
}
/// Bounded refresh for prepare. Explicit reset is sticky until explicit rebuild.

fn refresh(conn: &Connection, principal: &str, scope: &str) -> Result<usize, String> {
    let rows = evidence(conn, principal, scope)?;
    conn.execute(
        "DELETE FROM observation_association_incidence WHERE principal=?1 AND scope_label=?2",
        params![principal, scope],
    )
    .map_err(|e| e.to_string())?;
    for row in &rows {
        conn.execute(
            "INSERT INTO observation_association_incidence VALUES(?1,?2,?3,?4,?5)",
            params![
                principal,
                scope,
                row.id,
                row.digest,
                serde_json::to_string(&row.tokens).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(rows.len())
}

fn explain(
    conn: &Connection,
    principal: &str,
    scope: &str,
    cues: &[String],
    limit: usize,
) -> Result<Vec<AssociationExplanation>, String> {
    ensure(conn)?;
    if limit == 0 || !enabled(conn, principal, scope)? {
        return Ok(Vec::new());
    }
    let cues: BTreeSet<String> = cues
        .iter()
        .take(MAX_CUES)
        .flat_map(|c| tokens(&c.chars().take(1024).collect::<String>()))
        .take(MAX_CUES)
        .collect();
    if cues.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    for mut row in evidence(conn, principal, scope)? {
        let saved: Option<(String,String)> = conn.query_row("SELECT digest,tokens_json FROM observation_association_incidence WHERE principal=?1 AND scope_label=?2 AND source_id=?3", params![principal,scope,row.id], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e| e.to_string())?;
        if let Some((digest, json)) = saved {
            if digest == row.digest {
                let projected: BTreeSet<String> =
                    serde_json::from_str(&json).map_err(|e| e.to_string())?;
                // A corrupt/stale projection cannot supply tokens absent from exact bytes.
                row.tokens = row.tokens.intersection(&projected).cloned().collect();
                rows.push(row);
            }
        }
    }
    // Shared exact text connects source-key lineages transitively: copying an
    // older revision cannot manufacture a second witness beside its new revision.
    let mut components: BTreeMap<String, String> = rows
        .iter()
        .map(|r| (r.lineage.clone(), r.lineage.clone()))
        .collect();
    let mut digest_owner: BTreeMap<String, String> = BTreeMap::new();
    for row in &rows {
        if let Some(other) = digest_owner.get(&row.digest) {
            let left = components[&row.lineage].clone();
            let right = components[other].clone();
            let root = left.min(right.clone());
            let old = components[&row.lineage].clone();
            for component in components.values_mut() {
                if *component == old || *component == right {
                    *component = root.clone();
                }
            }
        } else {
            digest_owner.insert(row.digest.clone(), row.lineage.clone());
        }
    }
    let mut routes: BTreeMap<(String, String), Vec<&Evidence>> = BTreeMap::new();
    for row in &rows {
        for cue in row.tokens.intersection(&cues) {
            for alias in row.tokens.difference(&cues) {
                routes
                    .entry((cue.clone(), alias.clone()))
                    .or_default()
                    .push(row);
            }
        }
    }
    let mut best: BTreeMap<String, AssociationExplanation> = BTreeMap::new();
    for ((cue, alias), support) in routes {
        let mut texts = BTreeSet::new();
        let mut lineages = BTreeSet::new();
        let mut support_sources = Vec::new();
        for row in support {
            let lineage = &components[&row.lineage];
            if !texts.contains(&row.digest) && !lineages.contains(lineage) {
                texts.insert(row.digest.clone());
                lineages.insert(lineage.clone());
                support_sources.push(row.id.clone());
            }
        }
        if support_sources.len() < 2 {
            continue;
        }
        support_sources.sort();
        let score = (support_sources.len().min(8) as f64) / 8.0;
        for row in &rows {
            // This channel is deliberately separate from literal matches.
            if row.tokens.is_disjoint(&cues) && row.tokens.contains(&alias) {
                let item = AssociationExplanation {
                    source_id: row.id.clone(),
                    route: "learned_local_association".into(),
                    cue: cue.clone(),
                    alias: alias.clone(),
                    support_sources: support_sources.clone(),
                    score,
                };
                if best.get(&row.id).is_none_or(|old| old.score < score) {
                    best.insert(row.id.clone(), item);
                }
            }
        }
    }
    let mut result: Vec<_> = best.into_values().collect();
    ensure_feedback(conn)?;
    for item in &mut result {
        let weight:i64=conn.query_row("SELECT COALESCE(sum(value),0) FROM observation_association_feedback WHERE principal=?1 AND scope_label=?2 AND source_id=?3 AND active=1",params![principal,scope,item.source_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        item.score = (item.score + weight.clamp(-2, 2) as f64 / 8.0).clamp(0.0, 1.0);
    }
    result.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.source_id.cmp(&b.source_id))
    });
    result.truncate(limit.min(MAX_RESULTS));
    Ok(result)
}

/// Optional learned channel only. Caller must still perform exact qualification closure.
pub(crate) fn candidates(
    conn: &Connection,
    principal: &str,
    scope_label: &str,
    cues: &[String],
    limit: usize,
) -> Result<Vec<(String, f64)>, String> {
    Ok(explain(conn, principal, scope_label, cues, limit)?
        .into_iter()
        .map(|r| (r.source_id, r.score))
        .collect())
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
        let principal = self.observation_principal()?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        ensure_feedback(&conn)?;
        let tx = rusqlite::Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if !evidence(&tx, &principal, scope)?
            .iter()
            .any(|e| e.id == source_id)
        {
            return Err("feedback_source_not_eligible".into());
        }
        let value = match assessment {
            AssociationAssessment::Useful => 1,
            AssociationAssessment::Harmful => -1,
            AssociationAssessment::Neutral => 0,
        };
        let previous:Option<(String,i64)>=tx.query_row("SELECT source_id,value FROM observation_association_feedback WHERE principal=?1 AND scope_label=?2 AND event_key=?3",params![principal,scope,event_key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
        if let Some((old_target, old_value)) = previous {
            if old_target != source_id || old_value != value {
                return Err("feedback_identity_conflict".into());
            }
            return Ok(false);
        }
        tx.execute("INSERT INTO observation_association_feedback(principal,scope_label,event_key,source_id,value) VALUES(?1,?2,?3,?4,?5)",params![principal,scope,event_key,source_id,value]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub async fn retract_association_feedback(
        &self,
        cx: &Cx,
        scope: &str,
        event_key: &str,
    ) -> Result<(), String> {
        let principal = self.observation_principal()?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure_feedback(&conn)?;
        if conn.execute("UPDATE observation_association_feedback SET active=0 WHERE principal=?1 AND scope_label=?2 AND event_key=?3",params![principal,scope,event_key]).map_err(|e|e.to_string())?==0 {return Err("feedback_not_found".into());}
        Ok(())
    }
    /// Operator opt-in/re-enable and atomically replace only this scoped projection.
    pub async fn rebuild_associations(&self, cx: &Cx, scope: &str) -> Result<usize, String> {
        let principal = self.observation_principal()?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let tx = rusqlite::Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO observation_association_state VALUES(?1,?2,1) ON CONFLICT(principal,scope_label) DO UPDATE SET enabled=1", params![principal,scope]).map_err(|e| e.to_string())?;
        let count = refresh(&tx, &principal, scope)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(count)
    }
    /// Delete derived incidence only, and prevent automatic refresh until rebuild.
    pub async fn reset_associations(&self, cx: &Cx, scope: &str) -> Result<(), String> {
        let principal = self.observation_principal()?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let tx = rusqlite::Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO observation_association_state VALUES(?1,?2,0) ON CONFLICT(principal,scope_label) DO UPDATE SET enabled=0", params![principal,scope]).map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM observation_association_incidence WHERE principal=?1 AND scope_label=?2",
            params![principal, scope],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub async fn explain_associations(
        &self,
        cx: &Cx,
        scope: &str,
        cues: &[String],
        limit: usize,
    ) -> Result<Vec<AssociationExplanation>, String> {
        let principal = self.observation_principal()?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        let tx = rusqlite::Transaction::new_unchecked(&conn, TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let result = explain(&tx, &principal, scope, cues, limit)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
}

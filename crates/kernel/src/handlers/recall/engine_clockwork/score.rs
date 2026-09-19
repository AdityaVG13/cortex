use super::{
    RecallContext, RecallItem, ScoredCandidate, as_of_bind, candidate_matches_source_scope,
    is_visible, load_target, mark_arm, path_context_compatible, round4, strip_fts_operators,
};
use crate::clockwork::{
    ClockTarget, ClockWhy, FilterEvidence, LinkHit, QueryFrame, RankKey, Rankable, TemporalMode,
    TieBreak, WhyAnchor, admit_with_lineage, current_generation,
};
/// Consecutive-token phrase match (same law as retention cues).
pub(super) use crate::protocol::hay_has_phrase as hay_has_quoted_phrase;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(super) fn scored_to_item(
    candidate: ScoredCandidate,
    frame: &QueryFrame,
    valid_at: &str,
    ctx: &RecallContext,
) -> RecallItem {
    let evidence = candidate.evidence();
    let witnesses = candidate.witnesses.clone();
    let arms_for_questions = candidate.arms.clone();
    let status_for_questions = candidate.status.clone();
    let admitted_by = admit_with_lineage(
        Rankable {
            eligible: true,
            hard_anchor: candidate.hard_anchor,
            evidence,
            strong_lexical: candidate.strong_lexical,
        },
        &candidate.witnesses,
    )
    .unwrap_or("clock_quorum")
    .to_string();
    let relevance = rank_to_relevance(&candidate.rank_key());
    let why = ClockWhy::new(
        admitted_by,
        candidate.hard_anchor,
        evidence,
        stable_anchors(candidate.anchors),
        stable_links(candidate.links),
        FilterEvidence {
            acl: if ctx.team_mode {
                "owner".to_string()
            } else {
                "solo".to_string()
            },
            head: frame.head_id,
            valid_at: valid_at.to_string(),
            status_filters: current_status_filters(frame, ctx),
        },
        TieBreak {
            clock_count: evidence.nonzero_count(),
            strength: evidence.strength_sum(),
            hops: candidate.hops,
            specificity: candidate.specificity,
            fts_rank: candidate.fts_rank,
            use_score: candidate.use_score,
            recency: candidate.ts,
            target_type: candidate.target_type.clone(),
            target_id: candidate.target_id,
        },
    )
    .with_lineage(witnesses, &arms_for_questions, &status_for_questions);
    let admitted_arms = Value::Array(
        candidate
            .arms
            .iter()
            .map(|arm| Value::String((*arm).to_string()))
            .collect(),
    );
    let mut item = RecallItem::new_with_why(
        candidate.source,
        relevance,
        candidate.excerpt,
        "clock-quorum".to_string(),
    );
    let mut why_value =
        serde_json::to_value(&why).unwrap_or_else(|_| json!({"engine":"clock-quorum"}));
    if let Some(votes) = why_value
        .get_mut("clockVotes")
        .and_then(|v| v.as_object_mut())
    {
        votes.insert("admittedArms".to_string(), admitted_arms);
    }
    item.clock_why = Some(why_value);
    item.status = Some(candidate.status).filter(|s| !s.is_empty());
    item.valid_from = candidate.valid_from;
    item.valid_until = candidate.valid_until;
    item
}

pub(super) fn current_status_filters(frame: &QueryFrame, ctx: &RecallContext) -> Vec<String> {
    if ctx.include_cold
        || as_of_bind(ctx).is_some()
        || frame.as_of.is_some()
        || matches!(
            frame.temporal_mode,
            TemporalMode::Historical | TemporalMode::ExplicitAsOf
        )
    {
        Vec::new()
    } else {
        vec!["archived".to_string(), "superseded".to_string()]
    }
}

/// Required role (constraint-like kind) and open contradictions come from
/// the row's own governance columns, never from relevance evidence.
pub(super) fn annotate_roles(
    conn: &Connection,
    by_key: &mut HashMap<(String, i64), ScoredCandidate>,
) -> Result<(), String> {
    for ((target_type, target_id), candidate) in by_key.iter_mut() {
        if target_type != "decision" {
            continue;
        }
        match conn.query_row(
            "SELECT COALESCE(type, 'decision') FROM decisions WHERE id = ?1",
            params![target_id],
            |r| r.get::<_, String>(0),
        ) {
            Ok(kind) => {
                candidate.required_role = matches!(
                    kind.as_str(),
                    "constraint" | "policy" | "rule" | "convention" | "contract" | "preference"
                )
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) => return Err(e.to_string()),
        }
        candidate.contradiction = conn.query_row("SELECT COUNT(*) FROM decision_conflicts WHERE classification = 'CONTRADICTS' AND status = 'open' AND (source_decision_id = ?1 OR target_decision_id = ?1)", params![target_id], |r| r.get::<_, i64>(0)).map(|n| n > 0).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn rank_to_relevance(key: &RankKey) -> f64 {
    let hard = if key.hard_anchor { 4.0 } else { 0.0 };
    let score = hard
        + f64::from(key.clock_count)
        + f64::from(key.strength) * 0.25
        + f64::from(key.specificity) * 0.15;
    round4((score / 10.0).clamp(0.05, 1.0))
}

/// FTS MATCH already quotes `*`/`^` away; lexical contains() must use the
/// same needle or a quoted glob never scores as a quoted hit.
pub(super) fn lexical_needle(raw: &str) -> Option<String> {
    strip_fts_operators(raw).map(|t| t.to_ascii_lowercase())
}

pub(super) fn row_eligible(
    conn: &Connection,
    candidate: &ScoredCandidate,
    ctx: &RecallContext,
    frame: &QueryFrame,
    source_prefix: Option<&str>,
) -> Result<bool, String> {
    Ok(candidate_matches_source_scope(
        &candidate.target_type,
        candidate.target_id,
        &candidate.source,
        source_prefix,
    ) && is_visible(candidate.owner_id, candidate.visibility.as_deref(), ctx)
        && (!(matches!(frame.temporal_mode, TemporalMode::Current) || as_of_bind(ctx).is_some())
            || load_target(conn, &candidate.target_type, candidate.target_id, ctx)?.is_some())
        && path_context_compatible(conn, candidate, frame)?)
}

pub(super) fn upsert(out: &mut HashMap<(String, i64), ScoredCandidate>, incoming: ScoredCandidate) {
    let key = (incoming.target_type.clone(), incoming.target_id);
    out.entry(key)
        .and_modify(|existing| {
            existing.write = existing.write.max(incoming.write);
            existing.truth = existing.truth.max(incoming.truth);
            existing.task = existing.task.max(incoming.task);
            existing.history = existing.history.max(incoming.history);
            existing.hard_anchor |= incoming.hard_anchor;
            existing.strong_lexical |= incoming.strong_lexical;
            existing.specificity = existing.specificity.max(incoming.specificity);
            existing.fts_rank = existing.fts_rank.max(incoming.fts_rank);
            existing.hops = if existing.hops == 0 {
                incoming.hops
            } else {
                existing.hops.min(incoming.hops)
            };
            for witness in &incoming.witnesses {
                if !existing.witnesses.contains(witness) {
                    existing.witnesses.push(witness.clone());
                }
            }
            if incoming.excerpt.len() > existing.excerpt.len() {
                existing.excerpt = incoming.excerpt.clone();
            }
            for anchor in &incoming.anchors {
                if !existing
                    .anchors
                    .iter()
                    .any(|a| a.kind == anchor.kind && a.value == anchor.value)
                {
                    existing.anchors.push(anchor.clone());
                }
            }
            for arm in &incoming.arms {
                mark_arm(&mut existing.arms, arm);
            }
            existing.links.extend(incoming.links.iter().cloned());
            if existing.status.is_empty() {
                existing.status = incoming.status.clone();
            }
            if existing.valid_from.is_none() {
                existing.valid_from = incoming.valid_from.clone();
            }
            if existing.valid_until.is_none() {
                existing.valid_until = incoming.valid_until.clone();
            }
        })
        .or_insert(incoming);
}

fn first_id(conn: &Connection, sql: &str, source: &str) -> Option<i64> {
    conn.query_row(sql, params![source], |row| row.get(0))
        .optional()
        .ok()
        .flatten()
}

pub(super) fn resolve_source<'a>(conn: &Connection, source: &'a str) -> Option<(&'a str, i64)> {
    for (prefix, kind) in [("memory::", "memory"), ("decision::", "decision")] {
        if let Some(id) = source.strip_prefix(prefix).and_then(|s| s.parse().ok()) {
            return Some((kind, id));
        }
    }
    // cortex-7db determinism contract: context/source are not unique (the
    // store path inserts freely and dedupes only on decision-text similarity;
    // the indexer deliberately leaves superseded rows sharing a source), and
    // the chosen id decides which row load_target loads. The cut must be
    // data-defined, never SQLite-plan-defined: minimum id, matching the
    // hop-frontier/shared-anchor tiebreak.
    first_id(
        conn,
        "SELECT id FROM decisions WHERE context = ?1 ORDER BY id ASC LIMIT 1",
        source,
    )
    .map(|id| ("decision", id))
    .or_else(|| {
        first_id(
            conn,
            "SELECT id FROM memories WHERE source = ?1 ORDER BY id ASC LIMIT 1",
            source,
        )
        .map(|id| ("memory", id))
    })
}

pub(super) fn canonical_entity_name(conn: &Connection, entity_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT canonical_name FROM entities WHERE id = ?1",
        params![entity_id],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

pub(super) fn feedback_use_score(conn: &Connection, source: &str) -> Result<i64, String> {
    let pos: f64 = conn.query_row("SELECT COALESCE(SUM(CASE WHEN signal > 0 THEN signal ELSE 0 END), 0), COALESCE(SUM(CASE WHEN signal < 0 THEN -signal ELSE 0 END), 0) FROM recall_feedback WHERE result_source = ?1", params![source], |row| Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?))).optional().map_err(|e| e.to_string())?.map(|(p, n)| p - (2.0 * n)).unwrap_or(0.0);
    Ok(pos.round() as i64)
}

pub(super) fn hop_relation(conn: &Connection, target: &ClockTarget) -> Option<String> {
    conn.query_row("SELECT relation FROM clock_links WHERE (src_type = ?1 AND src_id = ?2) OR (dst_type = ?1 AND dst_id = ?2) ORDER BY CASE relation WHEN 'used_with' THEN 0 ELSE 1 END, relation LIMIT 1", params![target.target_type, target.target_id], |row| row.get(0)).ok()
}

pub(super) fn stable_anchors(mut anchors: Vec<WhyAnchor>) -> Vec<WhyAnchor> {
    anchors.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.value.cmp(&b.value))
            .then_with(|| b.specificity.cmp(&a.specificity))
    });
    anchors.dedup_by(|a, b| a.kind == b.kind && a.value == b.value);
    anchors
}

pub(super) fn stable_links(mut links: Vec<LinkHit>) -> Vec<LinkHit> {
    links.sort_by(|a, b| {
        a.relation
            .cmp(&b.relation)
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.to.cmp(&b.to))
    });
    links.dedup();
    links
}

pub fn clock_health_payload(conn: &Connection) -> Value {
    let generation = current_generation(conn).unwrap_or(0);
    let anchors = crate::db::count_or_zero(conn, "SELECT COUNT(*) FROM clock_anchors");
    let graph_ready = crate::db::count_or_zero(
        conn,
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'entities'",
    ) > 0;
    json!({"engine":"clock-quorum","modelFree":true,"anchorsReady":anchors > 0 || generation > 0,"graphReady":graph_ready,"derivedGeneration":generation})
}

use super::{
    ARM_ANCHOR, ARM_HISTORY, ARM_HOP, ARM_LEXICAL, ARM_TASK, ARM_TRUTH, RecallContext,
    ScoredCandidate, as_of_bind, build_fts_query, build_search_term_groups, caller_acl_param,
    candidate_matches_source_scope, canonical_entity_name, feedback_use_score, fts_rows,
    hay_has_quoted_phrase, hop_relation, is_visible, lexical_needle, load_target, loaded_candidate,
    mark_arm, qualified_acl, qualified_as_of_gates, quote_fts_match_term, resolve_source,
    source_prefix_applies_to_kind, upsert,
};
use crate::clockwork::{
    ClockTarget, ENTITY_GRAPH_CAP, FTS_CANDIDATE_CAP, GRAPH_HOP_CAP, HISTORY_CANDIDATE_CAP,
    LinkHit, QueryFrame, STRONG_ANCHOR_CAP, TASK_CANDIDATE_CAP, TemporalMode, WhyAnchor, Witness,
    WitnessDomain, hay_has_lexical, lookup_targets_for_anchors, traverse_hops,
};
use crate::graph;
use rusqlite::{Connection, params};
use std::collections::HashMap;

fn entity_mention_targets(conn: &Connection, entity_id: i64) -> Result<Vec<ClockTarget>, String> {
    let mut stmt = conn.prepare_cached("SELECT target_type, target_id FROM entity_mentions WHERE entity_id = ?1 ORDER BY target_type, target_id LIMIT ?2").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![entity_id, ENTITY_GRAPH_CAP as i64], |row| {
            Ok(ClockTarget {
                target_type: row.get(0)?,
                target_id: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

pub(super) fn collect_write_arm(
    conn: &Connection,
    frame: &QueryFrame,
    query_text: &str,
    source_prefix: Option<&str>,
    ctx: &RecallContext,
    out: &mut HashMap<(String, i64), ScoredCandidate>,
) -> Result<(), String> {
    let fts_query = clock_fts_query(frame)
        .unwrap_or_else(|| build_fts_query(&build_search_term_groups(query_text)));
    if fts_query.is_empty() {
        return Ok(());
    }
    let rare_terms: Vec<&str> = frame
        .terms
        .iter()
        .filter(|t| {
            !t.contains(' ')
                && !t.contains('/')
                && (t.len() >= 6
                    || t.chars().any(|c| c.is_ascii_digit())
                    || t.contains('_')
                    || !graph::lexical_cluster_mates(t).is_empty())
        })
        .map(String::as_str)
        .collect();
    let quoted = !frame.quoted_phrases.is_empty();
    for kind in ["decision", "memory"] {
        if !source_prefix_applies_to_kind(kind, source_prefix) {
            continue;
        }
        let rows = fts_rows(
            conn,
            kind,
            &fts_query,
            FTS_CANDIDATE_CAP,
            source_prefix,
            ctx,
        )?;
        for row in rows {
            let hay = row.excerpt.to_ascii_lowercase();
            let exact_rare = rare_terms
                .iter()
                .filter(|term| hay_has_lexical(&hay, term))
                .count();
            // Direct write evidence: quoted phrase, exact rare term, morphological
            // variant, or closed lexicon mate. Ordinary BM25 without one of those
            // stays write=1 and cannot admit alone.
            let unique = exact_rare >= 1;
            // FTS already stripped `*`/`^` inside quotes; a raw contains()
            // still treats `foo*` as a glob needle and misses the hit.
            // Multi-word needles also cannot use contains: `end to end`
            // misses hyphenated `end-to-end` (write stays 1, cannot admit
            // alone) and `log file` fires inside `catalog file`.
            let quoted_hit = quoted
                && frame.quoted_phrases.iter().any(|p| {
                    lexical_needle(p).is_some_and(|needle| hay_has_quoted_phrase(&hay, &needle))
                });
            let write = if quoted_hit || unique { 2 } else { 1 };
            let strong_lexical = write == 2;
            let mut candidate = loaded_candidate(
                conn,
                &row,
                0,
                write,
                0,
                0,
                0,
                false,
                strong_lexical,
                if write == 2 { 2 } else { 1 },
            )?;
            mark_arm(&mut candidate.arms, ARM_LEXICAL);
            candidate.witness(
                WitnessDomain::Lexical,
                fts_query.chars().take(40).collect::<String>(),
                if write == 2 { 2 } else { 1 },
            );
            upsert(out, candidate);
        }
    }
    Ok(())
}

fn clock_fts_query(frame: &QueryFrame) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    for term in &frame.terms {
        let t = term.trim();
        if t.len() < 2
            || t.contains(' ')
            || t.contains('/')
            || t.contains('\\')
            || matches!(t, "and" | "or" | "not")
        {
            continue;
        }
        if let Some(quoted) = quote_fts_match_term(t) {
            terms.push(quoted);
        }
    }
    terms.sort();
    terms.dedup();
    let phrases: Vec<String> = frame
        .quoted_phrases
        .iter()
        .filter(|phrase| phrase.len() >= 2)
        .filter_map(|phrase| quote_fts_match_term(phrase))
        .collect();
    match (phrases.is_empty(), terms.is_empty()) {
        (true, true) => None,
        (true, false) => Some(terms.join(" OR ")),
        (false, true) => Some(phrases.join(" AND ")),
        (false, false) => Some(format!(
            "({}) AND ({})",
            phrases.join(" AND "),
            terms.join(" OR ")
        )),
    }
}

pub(super) fn collect_anchor_arm(
    conn: &Connection,
    frame: &QueryFrame,
    ctx: &RecallContext,
    out: &mut HashMap<(String, i64), ScoredCandidate>,
) -> Result<(), String> {
    let strong: Vec<_> = frame
        .anchors
        .iter()
        .filter(|a| a.specificity >= 2)
        .cloned()
        .collect();
    if strong.is_empty() {
        return Ok(());
    }
    let targets = crate::clockwork::lookup_targets_with_matches(conn, &strong, STRONG_ANCHOR_CAP)
        .map_err(|e| e.to_string())?;
    let scope = crate::clockwork::AnchorScope::from_query(&ctx.paths, &frame.anchors);
    for (target, matched) in targets {
        let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)? else {
            continue;
        };
        mark_arm(&mut row.arms, ARM_ANCHOR);
        // Qualified: only the anchors that matched *this* row decide hardness,
        // and only inside their namespace (repo root, issuer). A row whose own
        // paths live under a foreign root, or whose issuer host disagrees, is
        // a strong lead, not identity.
        let namespace = crate::clockwork::RowNamespace {
            paths: crate::clockwork::target_anchor_values(
                conn,
                &target,
                crate::clockwork::AnchorKind::Path,
            )
            .map_err(|e| e.to_string())?,
            hosts: crate::clockwork::target_anchor_values(
                conn,
                &target,
                crate::clockwork::AnchorKind::UrlHost,
            )
            .map_err(|e| e.to_string())?,
        };
        let (matched, demotions) = crate::clockwork::qualify_matches(&matched, &namespace, &scope);
        for reason in demotions {
            row.witness(WitnessDomain::Anchor, format!("demoted:{reason}"), 1);
        }
        row.hard_anchor = matched.iter().any(|a| a.specificity >= 3);
        let best = matched.iter().map(|a| a.specificity).max().unwrap_or(2);
        let key = matched
            .iter()
            .map(|a| a.value.clone())
            .next()
            .unwrap_or_default();
        row.witness(WitnessDomain::Anchor, key, best);
        row.write = row.write.max(if row.hard_anchor { 2 } else { 1 });
        row.specificity = row
            .specificity
            .max(matched.iter().map(|a| a.specificity).max().unwrap_or(0));
        row.anchors = matched
            .into_iter()
            .map(|a| WhyAnchor {
                kind: a.kind,
                value: a.value,
                specificity: a.specificity,
            })
            .collect();
        upsert(out, row);
    }
    Ok(())
}

pub(super) fn collect_truth_arm(
    conn: &Connection,
    frame: &QueryFrame,
    ctx: &RecallContext,
    out: &mut HashMap<(String, i64), ScoredCandidate>,
) -> Result<(), String> {
    if frame.entity_ids.is_empty() {
        return Ok(());
    }
    for entity_id in frame.entity_ids.iter().take(ENTITY_GRAPH_CAP) {
        for target in entity_mention_targets(conn, *entity_id)? {
            let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)?
            else {
                continue;
            };
            mark_arm(&mut row.arms, ARM_TRUTH);
            let expanded = frame.expanded_entity_ids.contains(entity_id);
            // An entity named in the query itself is hard; one reached only
            // through expansion is a low-specificity access aid.
            row.truth = row.truth.max(if expanded { 1 } else { 2 });
            row.hard_anchor |= !expanded;
            row.specificity = row.specificity.max(if expanded { 1 } else { 2 });
            let name =
                canonical_entity_name(conn, *entity_id).unwrap_or_else(|| entity_id.to_string());
            row.witness(
                WitnessDomain::Entity,
                name.clone(),
                if expanded { 1 } else { 3 },
            );
            row.anchors.push(WhyAnchor {
                kind: crate::clockwork::AnchorKind::Entity,
                value: name,
                specificity: 2,
            });
            upsert(out, row);
        }
    }
    let graph_hits = graph::entity_arm_candidates(conn, &frame.raw, ENTITY_GRAPH_CAP);
    for (source, excerpt, score) in graph_hits {
        if let Some((target_type, target_id)) = resolve_source(conn, &source) {
            let Some(mut row) = load_target(conn, target_type, target_id, ctx)? else {
                continue;
            };
            mark_arm(&mut row.arms, ARM_TRUTH);
            row.truth = row.truth.max(if score >= 1.0 { 2 } else { 1 });
            if score >= 1.0 {
                row.hard_anchor = true;
            }
            row.witness(
                WitnessDomain::Entity,
                source.clone(),
                if score >= 1.0 { 3 } else { 1 },
            );
            if row.excerpt.is_empty() {
                row.excerpt = excerpt;
            }
            upsert(out, row);
        }
    }
    Ok(())
}

pub(super) fn collect_task_arm(
    conn: &Connection,
    frame: &QueryFrame,
    ctx: &RecallContext,
    out: &mut HashMap<(String, i64), ScoredCandidate>,
) -> Result<(), String> {
    let mut path_anchors = Vec::new();
    for path in &frame.paths {
        path_anchors.push(crate::clockwork::QueryAnchor {
            kind: crate::clockwork::AnchorKind::Path,
            value: normalize_task_path(path),
            specificity: 3,
        });
    }
    for symbol in &frame.symbols {
        path_anchors.push(crate::clockwork::QueryAnchor {
            kind: crate::clockwork::AnchorKind::Symbol,
            value: crate::clockwork::normalize_anchor_value(
                crate::clockwork::AnchorKind::Symbol,
                symbol,
            ),
            specificity: 3,
        });
    }
    for anchor in &frame.anchors {
        if matches!(
            anchor.kind,
            crate::clockwork::AnchorKind::Path | crate::clockwork::AnchorKind::Symbol
        ) && anchor.specificity >= 2
        {
            path_anchors.push(anchor.clone());
        }
    }
    if path_anchors.is_empty() {
        return Ok(());
    }
    let targets =
        crate::clockwork::lookup_targets_with_matches(conn, &path_anchors, TASK_CANDIDATE_CAP)
            .map_err(|e| e.to_string())?;
    let scope = crate::clockwork::AnchorScope::from_query(&ctx.paths, &frame.anchors);
    for (target, matched) in targets {
        let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)? else {
            continue;
        };
        mark_arm(&mut row.arms, ARM_TASK);
        // Same qualification as the anchor arm: the task's own paths are
        // identity only inside the task's repository root.
        let namespace = crate::clockwork::RowNamespace {
            paths: crate::clockwork::target_anchor_values(
                conn,
                &target,
                crate::clockwork::AnchorKind::Path,
            )
            .map_err(|e| e.to_string())?,
            hosts: Vec::new(),
        };
        let (matched, demotions) = crate::clockwork::qualify_matches(&matched, &namespace, &scope);
        for reason in demotions {
            row.witness(WitnessDomain::Task, format!("demoted:{reason}"), 1);
        }
        let hard = matched.iter().any(|a| a.specificity >= 3);
        row.task = row.task.max(if hard { 2 } else { 1 });
        row.hard_anchor |= hard;
        row.specificity = row.specificity.max(if hard { 3 } else { 2 });
        row.witness(
            WitnessDomain::Task,
            matched
                .iter()
                .map(|a| a.value.clone())
                .next()
                .unwrap_or_default(),
            if hard { 3 } else { 2 },
        );
        row.anchors.extend(matched.iter().map(|a| WhyAnchor {
            kind: a.kind,
            value: a.value.clone(),
            specificity: a.specificity,
        }));
        upsert(out, row);
    }
    if let Some(session) = frame.session_id.as_deref() {
        let session_anchor = [crate::clockwork::QueryAnchor {
            kind: crate::clockwork::AnchorKind::Session,
            value: session.to_ascii_lowercase(),
            specificity: 1,
        }];
        for target in lookup_targets_for_anchors(conn, &session_anchor, TASK_CANDIDATE_CAP)
            .map_err(|e| e.to_string())?
        {
            let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)?
            else {
                continue;
            };
            mark_arm(&mut row.arms, ARM_TASK);
            row.task = row.task.max(1);
            row.witness(WitnessDomain::Task, format!("session:{session}"), 1);
            upsert(out, row);
        }
    }
    Ok(())
}

pub(super) fn normalize_task_path(raw: &str) -> String {
    crate::clockwork::strip_path_globs(crate::clockwork::normalize_anchor_value(
        crate::clockwork::AnchorKind::Path,
        raw,
    ))
}

#[path = "arms/graph.rs"]
mod history;
pub(super) use history::{collect_history_arm, collect_hop_arm};

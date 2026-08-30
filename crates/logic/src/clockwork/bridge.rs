//! Model-free query expansion for Clock-Quorum Recall.
//!
//! Combines three evidence sources that stay inspectable and local:
//! - morphology (Porter-like stems) for cache↔caching;
//! - closed developer synonym clusters already used for entity qualifiers;
//! - KAR-style co-occurrence: sibling clock anchors on the same stored target.
//!
//! Expansion never becomes a hard anchor by itself. Common words still cannot
//! admit a result. Unrelated neighbors (snack policy vs payments webhook)
//! share no cluster, stem, or co-occurring strong anchor.

use super::anchors::AnchorKind;
use super::morph::{morph_stem, morph_variants};
use super::query::{QueryAnchor, QueryFrame};
use rusqlite::{params, Connection};

const MAX_EXPANDED_TERMS: usize = 16;
const MAX_SIBLING_ANCHORS: usize = 6;

pub fn expand_query_frame(conn: &Connection, frame: &mut QueryFrame) {
    let mut seeds: Vec<String> = Vec::new();
    for term in &frame.terms {
        push_unique(&mut seeds, term.clone());
    }
    for anchor in &frame.anchors {
        if matches!(
            anchor.kind,
            AnchorKind::Term | AnchorKind::Entity | AnchorKind::Acronym | AnchorKind::QuotedPhrase
        ) {
            push_unique(&mut seeds, anchor.value.clone());
        }
    }
    for token in frame.raw.split_whitespace() {
        let cleaned: String = token
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        if cleaned.len() >= 3 && !is_query_stop(&cleaned) {
            push_unique(&mut seeds, cleaned);
        }
    }

    let mut extra_terms: Vec<String> = Vec::new();
    let mut extra_anchors: Vec<QueryAnchor> = Vec::new();
    for seed in &seeds {
        for variant in morph_variants(seed) {
            push_unique(&mut extra_terms, variant.clone());
            extra_anchors.push(QueryAnchor {
                kind: AnchorKind::Term,
                value: variant,
                specificity: 1,
            });
        }
        for mate in crate::graph::lexical_cluster_mates(seed) {
            push_unique(&mut extra_terms, (*mate).to_string());
            extra_anchors.push(QueryAnchor {
                kind: AnchorKind::Term,
                value: (*mate).to_string(),
                specificity: 1,
            });
        }
        for sibling in sibling_anchors(conn, seed)
            .into_iter()
            .take(MAX_SIBLING_ANCHORS)
        {
            push_unique(&mut extra_terms, sibling.value.clone());
            extra_anchors.push(sibling);
        }
    }

    extra_terms.truncate(MAX_EXPANDED_TERMS);
    for term in extra_terms {
        if !frame.terms.iter().any(|existing| existing == &term) {
            frame.terms.push(term);
        }
    }
    extra_anchors.sort_by(|a, b| {
        b.specificity
            .cmp(&a.specificity)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.value.cmp(&b.value))
    });
    extra_anchors.dedup_by(|a, b| a.kind == b.kind && a.value == b.value);
    for anchor in extra_anchors {
        if !frame
            .anchors
            .iter()
            .any(|existing| existing.kind == anchor.kind && existing.value == anchor.value)
        {
            frame.anchors.push(anchor);
        }
    }
    frame.anchors.truncate(super::MAX_ANCHORS_PER_QUERY.max(1));

    let joined = frame.terms.join(" ");
    if !joined.is_empty() {
        for id in crate::graph::resolve_query(conn, &joined) {
            if !frame.entity_ids.contains(&id) {
                frame.entity_ids.push(id);
            }
        }
    }
}

fn sibling_anchors(conn: &Connection, seed: &str) -> Vec<QueryAnchor> {
    let lowered = seed.to_ascii_lowercase();
    let stem = morph_stem(&lowered);
    let mut stmt = match conn.prepare_cached(
        "SELECT a2.kind, a2.value, a2.specificity
         FROM clock_anchors a1
         JOIN clock_anchor_evidence e1 ON e1.anchor_id = a1.id
         JOIN clock_anchor_evidence e2
           ON e2.target_type = e1.target_type AND e2.target_id = e1.target_id
         JOIN clock_anchors a2 ON a2.id = e2.anchor_id
         WHERE a1.value = ?1
           AND a2.value != a1.value
           AND a2.specificity >= 2
           AND a2.kind IN ('term', 'entity', 'acronym', 'symbol', 'quoted_phrase')
         ORDER BY a2.specificity DESC, a2.kind ASC, a2.value ASC
         LIMIT 8",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    if let Ok(rows) = stmt.query_map(params![lowered], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    }) {
        for (kind, value, spec) in rows.flatten() {
            if morph_stem(&value) == stem {
                continue;
            }
            if let Some(kind) = AnchorKind::parse(&kind) {
                out.push(QueryAnchor {
                    kind,
                    value,
                    specificity: spec.clamp(1, 2) as u8,
                });
            }
        }
    }
    out
}

fn push_unique(out: &mut Vec<String>, value: String) {
    if value.len() < 3 {
        return;
    }
    if !out.iter().any(|existing| existing == &value) {
        out.push(value);
    }
}

fn is_query_stop(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "a"
            | "an"
            | "is"
            | "are"
            | "was"
            | "were"
            | "be"
            | "do"
            | "does"
            | "did"
            | "how"
            | "what"
            | "which"
            | "who"
            | "when"
            | "where"
            | "why"
            | "we"
            | "our"
            | "use"
            | "for"
            | "with"
            | "from"
            | "this"
            | "that"
            | "should"
            | "work"
            | "using"
            | "used"
    )
}

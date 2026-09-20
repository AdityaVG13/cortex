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
use rusqlite::{Connection, params};

const MAX_EXPANDED_TERMS: usize = 16;
const MAX_SIBLING_ANCHORS: usize = 6;

pub fn expand_query_frame(
    conn: &Connection,
    frame: &mut QueryFrame,
    principal: Option<&str>,
) {
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
        if seeds.len() >= super::MAX_QUERY_TOKENS {
            break;
        }
        let cleaned: String = token
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        if cleaned.len() >= 3 && !is_query_stop(&cleaned) {
            push_unique(&mut seeds, cleaned);
        }
    }
    seeds.truncate(super::MAX_QUERY_TOKENS);

    let mut extra_terms: Vec<String> = Vec::new();
    let mut extra_anchors: Vec<QueryAnchor> = Vec::new();
    // Loop 2: terms from similar *successful* past queries. Same channel
    // as the other sources (spec-1 access aids, never hard anchors alone).
    // Principal-scoped; without a principal there is no expansion. First,
    // so the cap below trims morphosyntactic filler before learned terms.
    if let Some(principal) = principal {
        for remembered in remembered_query_terms(conn, principal, frame) {
            push_term(&mut extra_terms, &mut extra_anchors, remembered);
        }
        // Loop 4: doc-side terms of matured bridges (positive mass ≥ 2,
        // unvetoed). Learned after remembered: both precede filler.
        for bridged in bridge_doc_terms(conn, principal, frame) {
            push_term(&mut extra_terms, &mut extra_anchors, bridged);
        }
    }
    for seed in &seeds {
        for variant in morph_variants(seed) {
            push_term(&mut extra_terms, &mut extra_anchors, variant);
        }
        for mate in crate::graph::lexical_cluster_mates(seed) {
            push_term(&mut extra_terms, &mut extra_anchors, (*mate).to_string());
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
    extend_absent(&mut frame.terms, extra_terms, |a, b| a == b);
    extra_anchors.sort_by(|a, b| {
        b.specificity
            .cmp(&a.specificity)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.value.cmp(&b.value))
    });
    extra_anchors.dedup_by(|a, b| a.kind == b.kind && a.value == b.value);
    extend_absent(&mut frame.anchors, extra_anchors, |a, b| {
        a.kind == b.kind && a.value == b.value
    });
    frame.anchors.truncate(super::MAX_ANCHORS_PER_QUERY.max(1));

    let joined = frame.terms.join(" ");
    if !joined.is_empty() {
        for id in crate::graph::resolve_query(conn, &joined) {
            if !frame.entity_ids.contains(&id) {
                frame.entity_ids.push(id);
                frame.expanded_entity_ids.push(id);
            }
        }
    }
}

fn remembered_query_terms(
    conn: &Connection,
    principal: &str,
    frame: &QueryFrame,
) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut stmt = match conn.prepare_cached("SELECT terms_json, anchors_json FROM query_memory WHERE principal = ?1 AND successes > 0 ORDER BY successes DESC, signature ASC LIMIT 64") { Ok(stmt) => stmt, Err(_) => return Vec::new() };
    let rows: Vec<(String, String)> = match stmt
        .query_map(params![principal], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
        Ok(mapped) => mapped.flatten().collect(),
        Err(_) => return Vec::new(),
    };
    let have_terms: BTreeSet<&str> = frame.terms.iter().map(String::as_str).collect();
    // Stem-normalized overlap: a shared word and its stem are one shared
    // word, not two (otherwise any single shared word clears the bar).
    let have_stems: BTreeSet<String> = frame
        .terms
        .iter()
        .filter(|t| !t.contains(' '))
        .map(|t| morph_stem(&t.to_ascii_lowercase()))
        .collect();
    let have_anchors: BTreeSet<String> = frame
        .anchors
        .iter()
        .filter(|a| a.specificity >= 2)
        .map(|a| format!("{}:{}", a.kind.as_str(), a.value))
        .collect();
    let have_anchors: BTreeSet<&str> = have_anchors.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    let mut frames_used = 0;
    for (terms_json, anchors_json) in &rows {
        if frames_used >= 3 || out.len() >= 8 {
            break;
        }
        let row_terms: Vec<String> = serde_json::from_str(terms_json).unwrap_or_default();
        let row_anchors: Vec<String> = serde_json::from_str(anchors_json).unwrap_or_default();
        let row_stems: BTreeSet<String> = row_terms
            .iter()
            .filter(|t| !t.contains(' '))
            .map(|t| morph_stem(&t.to_ascii_lowercase()))
            .collect();
        let shared_terms = row_stems.intersection(&have_stems).count();
        let shared_anchors = row_anchors
            .iter()
            .filter(|a| have_anchors.contains(a.as_str()))
            .count();
        if shared_terms < 2 && shared_anchors < 1 {
            continue;
        }
        frames_used += 1;
        for term in row_terms {
            if out.len() >= 8 {
                break;
            }
            if term.len() >= 3 && !have_terms.contains(term.as_str()) && !out.contains(&term) {
                out.push(term);
            }
        }
    }
    out
}

fn bridge_doc_terms(conn: &Connection, principal: &str, frame: &QueryFrame) -> Vec<String> {
    use std::collections::BTreeSet;
    let seeds: Vec<String> = frame
        .terms
        .iter()
        .filter(|t| !t.contains(' ') && t.len() >= 3)
        .map(|t| t.to_ascii_lowercase())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(16)
        .collect();
    if seeds.is_empty() {
        return Vec::new();
    }
    let placeholders = seeds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("SELECT doc_term FROM term_bridges WHERE principal = ?1 AND query_term IN ({placeholders}) AND positive >= 2 AND negative = 0 ORDER BY positive DESC, doc_term ASC LIMIT 8");
    let mut stmt = match conn.prepare_cached(&sql) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let have: BTreeSet<String> = frame.terms.iter().cloned().collect();
    let mut params: Vec<&dyn rusqlite::types::ToSql> = vec![&principal as &dyn rusqlite::types::ToSql];
    params.extend(seeds.iter().map(|s| s as &dyn rusqlite::types::ToSql));
    match stmt.query_map(params.as_slice(), |row| row.get::<_, String>(0)) {
        Ok(mapped) => mapped
            .flatten()
            .filter(|term| !have.contains(term))
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn sibling_anchors(conn: &Connection, seed: &str) -> Vec<QueryAnchor> {
    let lowered = seed.to_ascii_lowercase();
    let stem = morph_stem(&lowered);
    let mut stmt = match conn.prepare_cached("SELECT a2.kind, a2.value, a2.specificity FROM clock_anchors a1 JOIN clock_anchor_evidence e1 ON e1.anchor_id = a1.id JOIN clock_anchor_evidence e2 ON e2.target_type = e1.target_type AND e2.target_id = e1.target_id JOIN clock_anchors a2 ON a2.id = e2.anchor_id WHERE a1.value = ?1 AND a2.value != a1.value AND a2.specificity >= 2 AND a2.kind IN ('term', 'entity', 'acronym', 'symbol', 'quoted_phrase') ORDER BY a2.specificity DESC, a2.kind ASC, a2.value ASC LIMIT 8") { Ok(stmt) => stmt, Err(_) => return Vec::new() };
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

fn extend_absent<T>(out: &mut Vec<T>, items: Vec<T>, eq: impl Fn(&T, &T) -> bool) {
    for item in items {
        if !out.iter().any(|existing| eq(existing, &item)) {
            out.push(item);
        }
    }
}

fn push_term(extra_terms: &mut Vec<String>, extra_anchors: &mut Vec<QueryAnchor>, value: String) {
    push_unique(extra_terms, value.clone());
    extra_anchors.push(QueryAnchor {
        kind: AnchorKind::Term,
        value,
        specificity: 1,
    });
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

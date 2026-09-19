use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, BTreeSet};

fn projected_evidence(
    conn: &Connection,
    principal: &str,
    scope: &str,
) -> Result<Vec<Evidence>, String> {
    let mut rows = Vec::new();
    for mut row in evidence(conn, principal, scope)? {
        let saved: Option<(String, String)> = conn.query_row("SELECT digest,tokens_json FROM observation_association_incidence WHERE principal=?1 AND scope_label=?2 AND source_id=?3", params![principal, scope, row.id], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(|e| e.to_string())?;
        let Some((digest, json)) = saved else {
            continue;
        };
        if digest != row.digest {
            continue;
        }
        let projected: BTreeSet<String> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        // A projection cannot supply tokens absent from the exact bytes.
        row.tokens.retain(|token| projected.contains(token));
        rows.push(row);
    }
    Ok(rows)
}

fn lineage_components(rows: &[Evidence]) -> BTreeMap<&str, &str> {
    // Shared text connects source lineages transitively. A copied old revision
    // must not become a second witness beside its new revision.
    let mut components: BTreeMap<&str, &str> = rows
        .iter()
        .map(|row| (row.lineage.as_str(), row.lineage.as_str()))
        .collect();
    let mut digest_owner: BTreeMap<&str, &str> = BTreeMap::new();
    for row in rows {
        let Some(other) = digest_owner.get(row.digest.as_str()) else {
            digest_owner.insert(row.digest.as_str(), row.lineage.as_str());
            continue;
        };
        let left = components[row.lineage.as_str()];
        let right = components[other];
        let root = left.min(right);
        for component in components
            .values_mut()
            .filter(|c| **c == left || **c == right)
        {
            *component = root;
        }
    }
    components
}

fn independent_support(support: Vec<&Evidence>, components: &BTreeMap<&str, &str>) -> Vec<String> {
    let mut texts = BTreeSet::new();
    let mut lineages = BTreeSet::new();
    let mut sources = Vec::new();
    for row in support {
        let lineage = components[row.lineage.as_str()];
        if texts.contains(row.digest.as_str()) || lineages.contains(lineage) {
            continue;
        }
        texts.insert(row.digest.as_str());
        lineages.insert(lineage);
        sources.push(row.id.clone());
    }
    sources.sort();
    sources
}

fn association_routes<'a>(
    rows: &'a [Evidence],
    cues: &'a BTreeSet<String>,
) -> BTreeMap<(&'a str, &'a str), Vec<&'a Evidence>> {
    let mut routes: BTreeMap<(&str, &str), Vec<&Evidence>> = BTreeMap::new();
    for row in rows {
        for cue in row.tokens.intersection(cues) {
            for alias in row.tokens.difference(cues) {
                routes
                    .entry((cue.as_str(), alias.as_str()))
                    .or_default()
                    .push(row);
            }
        }
    }
    routes
}

fn rank_routes(rows: &[Evidence], cues: &BTreeSet<String>) -> Vec<AssociationExplanation> {
    let components = lineage_components(rows);
    let mut best: BTreeMap<&str, AssociationExplanation> = BTreeMap::new();
    for ((cue, alias), support) in association_routes(rows, cues) {
        let support_sources = independent_support(support, &components);
        if support_sources.len() < 2 {
            continue;
        }
        let score = (support_sources.len().min(8) as f64) / 8.0;
        // Learned results are separate from literal matches. Ties retain the
        // first route in the deterministic BTreeMap order.
        for row in rows
            .iter()
            .filter(|row| row.tokens.is_disjoint(cues) && row.tokens.contains(alias))
        {
            if best
                .get(row.id.as_str())
                .is_some_and(|old| old.score >= score)
            {
                continue;
            }
            best.insert(
                &row.id,
                AssociationExplanation {
                    source_id: row.id.clone(),
                    route: "learned_local_association".into(),
                    cue: cue.into(),
                    alias: alias.into(),
                    support_sources: support_sources.clone(),
                    score,
                },
            );
        }
    }
    best.into_values().collect()
}

pub(super) fn explain(
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
    let rows = projected_evidence(conn, principal, scope)?;
    let mut result = rank_routes(&rows, &cues);
    ensure_feedback(conn)?;
    for item in &mut result {
        let weight: i64 = conn.query_row("SELECT COALESCE(sum(value),0) FROM observation_association_feedback WHERE principal=?1 AND scope_label=?2 AND source_id=?3 AND active=1", params![principal, scope, item.source_id], |r| r.get(0)).map_err(|e| e.to_string())?;
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

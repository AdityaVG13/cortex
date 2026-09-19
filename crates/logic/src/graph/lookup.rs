use super::mentions::{Mention, extract_mentions, kind_class, normalize_token, same_qualifier};
use crate::protocol::{temporal_bounds_sql_at, unorphaned_version_sql};
use rusqlite::{Connection, OptionalExtension, params};

/// Resolves a mention to an existing compatible entity or creates one.
/// Records each resolved surface form as an alias.
pub fn resolve_mention(
    conn: &Connection,
    mention: &Mention,
    trace_id: Option<i64>,
    owner_id: Option<i64>,
) -> Option<i64> {
    match lookup_alias_of_kind(conn, &mention.surface.to_lowercase(), &mention.kind) {
        Ok(Some(id)) => return Some(id),
        Ok(None) => {}
        Err(_) => return None,
    }
    let entity_id = match first_matching_entity(conn, Some(&mention.kind), &mention.qualifier) {
        Some(id) => id,
        None => {
            conn.execute("INSERT INTO entities (canonical_name, qualifier, kind, owner_id) VALUES (?1, ?2, ?3, ?4)", params![mention.surface, mention.qualifier, mention.kind, owner_id]).ok()?;
            conn.last_insert_rowid()
        }
    };
    let _ = conn.execute("INSERT OR IGNORE INTO entity_aliases (entity_id, alias, qualifier, status, source_trace_id) VALUES (?1, ?2, ?3, 'confirmed', ?4)", params![entity_id, mention.surface.to_lowercase(), mention.qualifier, trace_id]);
    Some(entity_id)
}

/// Resolves text mentions and links their entities to a stored row.
pub fn ingest_for_target(
    conn: &Connection,
    text: &str,
    target_type: &str,
    target_id: Option<i64>,
    trace_id: Option<i64>,
    owner_id: Option<i64>,
) -> Vec<i64> {
    let ids: Vec<i64> = extract_mentions(text)
        .iter()
        .filter_map(|m| resolve_mention(conn, m, trace_id, owner_id))
        .collect();
    if let Some(target) = target_id {
        for entity_id in &ids {
            let _ = conn.execute("INSERT OR IGNORE INTO entity_mentions (entity_id, target_type, target_id) VALUES (?1, ?2, ?3)", params![entity_id, target_type, target]);
        }
    }
    ids
}

/// Resolves a free-text query to entity IDs.
pub fn resolve_query(conn: &Connection, query: &str) -> Vec<i64> {
    let query = crate::clockwork::bound_query_text(query);
    let mut ids: Vec<i64> = extract_mentions(query)
        .iter()
        .filter_map(|m| resolve_mention_to_existing(conn, m))
        .collect();
    // Count only lookup candidates toward MAX_QUERY_TOKENS so filler
    // ("the", kind suffixes) cannot evict the named qualifier.
    let mut considered = 0usize;
    for token in query.split_whitespace() {
        let token = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
        let lowered = token.to_ascii_lowercase();
        let norm = normalize_token(token);
        if norm.len() < 2 || kind_class(&norm).is_some() || crate::clockwork::is_stop_word(&norm) {
            continue;
        }
        if considered >= crate::clockwork::MAX_QUERY_TOKENS {
            break;
        }
        considered += 1;
        let id = lookup_alias(conn, &lowered)
            .or_else(|| {
                if norm != lowered {
                    lookup_alias(conn, &norm)
                } else {
                    None
                }
            })
            .or_else(|| lookup_entity_by_qualifier(conn, &norm));
        ids.extend(id);
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn lookup_alias(conn: &Connection, alias: &str) -> Option<i64> {
    conn.query_row(
        "SELECT entity_id FROM entity_aliases WHERE alias = ?1 ORDER BY entity_id ASC LIMIT 1",
        params![alias],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

fn lookup_alias_of_kind(
    conn: &Connection,
    alias: &str,
    kind: &str,
) -> rusqlite::Result<Option<i64>> {
    conn.query_row("SELECT a.entity_id FROM entity_aliases a JOIN entities e ON e.id = a.entity_id WHERE a.alias = ?1 AND e.kind = ?2 ORDER BY a.entity_id ASC LIMIT 1", params![alias, kind], |row| row.get(0)).optional()
}

fn lookup_entity_by_qualifier(conn: &Connection, qualifier: &str) -> Option<i64> {
    first_matching_entity(conn, None, qualifier)
}

fn first_matching_entity(conn: &Connection, kind: Option<&str>, qualifier: &str) -> Option<i64> {
    let sql = if kind.is_some() {
        "SELECT id, qualifier FROM entities WHERE kind = ?1 ORDER BY id ASC"
    } else {
        "SELECT id, qualifier FROM entities ORDER BY id ASC"
    };
    let mut stmt = conn.prepare_cached(sql).ok()?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(kind), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;
    rows.flatten()
        .find(|(_, existing)| same_qualifier(qualifier, existing))
        .map(|(id, _)| id)
}

fn resolve_mention_to_existing(conn: &Connection, mention: &Mention) -> Option<i64> {
    match lookup_alias_of_kind(conn, &mention.surface.to_lowercase(), &mention.kind) {
        Ok(Some(id)) => return Some(id),
        Ok(None) => {}
        Err(_) => return None,
    }
    first_matching_entity(conn, Some(&mention.kind), &mention.qualifier)
        .or_else(|| lookup_entity_by_qualifier(conn, &mention.qualifier))
}

/// Returns rows linked directly or one hop from entities resolved from a query.
/// Uses recall-compatible source keys for rank fusion.
pub fn entity_arm_candidates(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Vec<(String, String, f64)> {
    let seed_ids = resolve_query(conn, query);
    if seed_ids.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entity_id in related_entities(conn, &seed_ids) {
        let score = if seed_ids.contains(&entity_id) {
            1.0
        } else {
            0.6
        };
        append_entity_candidates(conn, entity_id, score, limit, &mut out, &mut seen);
        if out.len() >= limit {
            break;
        }
    }
    out.truncate(limit);
    out
}

fn related_entities(conn: &Connection, seeds: &[i64]) -> Vec<i64> {
    let mut ids = seeds.to_vec();
    let Ok(mut stmt) = conn.prepare_cached("SELECT DISTINCT other.entity_id FROM entity_mentions seed JOIN entity_mentions other ON other.target_type = seed.target_type AND other.target_id = seed.target_id WHERE seed.entity_id = ?1 AND other.entity_id != ?1 ORDER BY other.entity_id ASC LIMIT 8") else { return ids; };
    for seed in seeds {
        let Ok(rows) = stmt.query_map(params![seed], |row| row.get::<_, i64>(0)) else {
            continue;
        };
        for id in rows.flatten() {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

fn append_entity_candidates(
    conn: &Connection,
    entity_id: i64,
    score: f64,
    limit: usize,
    out: &mut Vec<(String, String, f64)>,
    seen: &mut std::collections::HashSet<String>,
) {
    for (table, kind, alias, source_sql, text_col) in [
        (
            "decisions",
            "decision",
            "d",
            "COALESCE(d.context, 'decision::' || d.id)",
            "decision",
        ),
        (
            "memories",
            "memory",
            "m",
            "COALESCE(m.source, 'memory::' || m.id)",
            "text",
        ),
    ] {
        let gates = format!(
            "{} AND {}",
            temporal_bounds_sql_at(alias, "'now'"),
            unorphaned_version_sql(alias)
        );
        let sql = format!(
            "SELECT {source_sql}, {alias}.{text_col} FROM entity_mentions em JOIN {table} {alias} ON em.target_type = '{kind}' AND {alias}.id = em.target_id WHERE em.entity_id = ?1 AND {alias}.status NOT IN ('superseded','archived') AND {gates} ORDER BY {alias}.id DESC LIMIT ?2"
        );
        let Ok(mut stmt) = conn.prepare_cached(&sql) else {
            continue;
        };
        let Ok(rows) = stmt.query_map(params![entity_id, limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        for (source, text) in rows.flatten() {
            if seen.insert(source.clone()) {
                out.push((source, text, score));
            }
        }
    }
}

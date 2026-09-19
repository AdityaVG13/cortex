use super::*;
use rusqlite::{Connection, params};
use std::collections::HashMap;

pub(crate) fn collect_history_arm(
    conn: &Connection,
    frame: &QueryFrame,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    out: &mut HashMap<(String, i64), ScoredCandidate>,
) -> Result<(), String> {
    match frame.temporal_mode {
        TemporalMode::Current | TemporalMode::Any => return Ok(()),
        TemporalMode::Historical | TemporalMode::ExplicitAsOf => {}
    }
    let as_of = as_of_bind(ctx);
    let caller = caller_acl_param(ctx);
    let gates = if as_of.is_some() {
        qualified_as_of_gates("d", "?1")
    } else {
        // Historical with no timestamp: archived/superseded stay
        // eligible. Current-time valid_until would hide a closed window.
        "(d.version_id IS NULL OR d.version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) AND (?1 IS NULL OR 1)".to_string()
    };
    let acl = qualified_acl("d", "?3");
    let sql = format!(
        "SELECT 'decision', d.id, d.decision, COALESCE(d.context, 'decision::' || d.id), d.owner_id, d.visibility, d.created_at, d.status, d.valid_from, d.valid_until FROM decisions d WHERE {gates} {acl} ORDER BY d.id DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            params![as_of, HISTORY_CANDIDATE_CAP as i64, caller],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    for (
        target_type,
        target_id,
        excerpt,
        source,
        owner_id,
        visibility,
        ts,
        status,
        valid_from,
        valid_until,
    ) in rows.flatten()
    {
        if !is_visible(owner_id, visibility.as_deref(), ctx) {
            continue;
        }
        if !candidate_matches_source_scope(&target_type, target_id, &source, source_prefix) {
            continue;
        }
        let mut row = ScoredCandidate {
            target_type,
            target_id,
            source,
            excerpt,
            owner_id,
            visibility,
            ts: crate::handlers::parse_timestamp_ms(ts.as_deref().unwrap_or("")),
            hops: 0,
            write: 0,
            truth: 0,
            task: 0,
            history: if as_of.is_some() { 2 } else { 1 },
            hard_anchor: false,
            strong_lexical: false,
            specificity: 1,
            fts_rank: 0,
            use_score: 0,
            anchors: Vec::new(),
            links: Vec::new(),
            arms: vec![ARM_HISTORY],
            status,
            valid_from,
            valid_until,
            witnesses: Vec::new(),
            required_role: false,
            contradiction: false,
        };
        row.witness(
            WitnessDomain::History,
            as_of.clone().unwrap_or_else(|| "recent".into()),
            if as_of.is_some() { 2 } else { 1 },
        );
        row.use_score = feedback_use_score(conn, &row.source)?;
        upsert(out, row);
    }
    Ok(())
}

pub(crate) fn collect_hop_arm(
    conn: &Connection,
    frame: &QueryFrame,
    ctx: &RecallContext,
    out: &mut HashMap<(String, i64), ScoredCandidate>,
) -> Result<(), String> {
    let mut seeds: Vec<ClockTarget> = out
        .keys()
        .cloned()
        .map(|(target_type, target_id)| ClockTarget {
            target_type,
            target_id,
        })
        .collect();
    if seeds.is_empty() {
        for entity_id in frame.entity_ids.iter().take(ENTITY_GRAPH_CAP) {
            seeds.extend(entity_mention_targets(conn, *entity_id)?);
        }
    }
    seeds.sort();
    seeds.dedup();
    if seeds.is_empty() {
        return Ok(());
    }
    let hops = traverse_hops(conn, &seeds, 2, GRAPH_HOP_CAP).map_err(|e| e.to_string())?;
    // Every hop-discovered row inherits the origin of the seed set it was
    // reached from: one traversal is one lineage family and can count at
    // most once toward relevance, never as two independent clocks.
    let seed_origin = format!(
        "hop:{}",
        seeds
            .iter()
            .map(|s| format!("{}::{}", s.target_type, s.target_id))
            .collect::<Vec<_>>()
            .join("+")
    );
    for (target, hop) in hops {
        if hop == 0 {
            continue;
        }
        let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)? else {
            continue;
        };
        mark_arm(&mut row.arms, ARM_HOP);
        row.hops = hop;
        let relation = hop_relation(conn, &target).unwrap_or_else(|| "observed_with".to_string());
        row.witnesses.push(Witness::derived(
            WitnessDomain::Hop,
            seed_origin.clone(),
            relation.clone(),
            hop,
        ));
        if relation == "used_with" {
            // Explicit feedback ("this was useful for that query") is its own
            // evidentiary origin — the feedback ledger — distinct from the
            // seed row, so it can pair with the route. It is usefulness under
            // a task context, never a truth vote; rejection removes it.
            row.task = row.task.max(1);
            row.witnesses.push(Witness::direct(
                WitnessDomain::Task,
                format!("used_with:{}::{}", target.target_type, target.target_id),
                "feedback",
                1,
            ));
        }
        row.links.push(LinkHit {
            relation,
            from: format!("{}::{}", target.target_type, target.target_id),
            to: frame.raw.chars().take(40).collect(),
        });
        upsert(out, row);
    }
    Ok(())
}

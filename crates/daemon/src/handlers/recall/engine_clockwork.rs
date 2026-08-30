use crate::clockwork::{
    admit, current_generation, expand_query_frame, hay_has_lexical, lookup_targets_for_anchors, parse_query_frame, query_signature, traverse_hops,
    ClockEvidence, ClockTarget, ClockWhy, FilterEvidence, LinkHit, QueryFrame, RankKey, Rankable, TemporalMode, TieBreak, WhyAnchor,
    ENTITY_GRAPH_CAP, FTS_CANDIDATE_CAP, GRAPH_HOP_CAP, HISTORY_CANDIDATE_CAP, STRONG_ANCHOR_CAP, TASK_CANDIDATE_CAP,
};
use crate::graph;
use crate::traces;

const ACTIVE_GATES: &str = "status NOT IN ('superseded','archived') \
 AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) \
 AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) \
 AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) \
 AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))";

#[derive(Clone)]
struct ScoredCandidate {
    target_type: String,
    target_id: i64,
    source: String,
    excerpt: String,
    owner_id: Option<i64>,
    visibility: Option<String>,
    ts: i64,
    hops: u8,
    write: u8,
    truth: u8,
    task: u8,
    history: u8,
    hard_anchor: bool,
    strong_lexical: bool,
    specificity: u8,
    fts_rank: i64,
    use_score: i64,
    anchors: Vec<WhyAnchor>,
    links: Vec<LinkHit>,
    status: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
}

impl ScoredCandidate {
    fn evidence(&self) -> ClockEvidence {
        ClockEvidence { write: self.write, truth: self.truth, task: self.task, history: self.history }
    }

    fn rank_key(&self) -> RankKey {
        RankKey::from_parts(
            self.hard_anchor,
            self.evidence(),
            self.specificity,
            self.hops,
            self.fts_rank,
            self.use_score,
            self.ts,
            self.target_type.clone(),
            self.target_id,
        )
    }
}

pub(crate) fn run_clock_quorum_recall(
    conn: &Connection, query_text: &str, token_budget: usize, k: usize, ctx: &RecallContext, source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    let mut frame = parse_query_frame(
        query_text,
        ctx.caller_id,
        ctx.session_id.clone(),
        ctx.goal_id,
        ctx.paths.clone(),
        ctx.symbols.clone(),
        ctx.as_of.clone(),
        traces::current_head(conn),
    );
    if let Some(prefix) = source_prefix.map(str::trim).filter(|s| !s.is_empty()) {
        if !frame.paths.iter().any(|p| p == prefix) {
            frame.paths.push(prefix.to_string());
        }
    }
    frame.entity_ids = graph::resolve_query(conn, query_text);
    expand_query_frame(conn, &mut frame);
    let _signature = query_signature(&frame);

    let mut by_key: HashMap<(String, i64), ScoredCandidate> = HashMap::new();
    collect_write_arm(conn, &frame, query_text, source_prefix, ctx, &mut by_key)?;
    collect_anchor_arm(conn, &frame, ctx, &mut by_key)?;
    collect_truth_arm(conn, &frame, ctx, &mut by_key)?;
    collect_task_arm(conn, &frame, ctx, &mut by_key)?;
    collect_history_arm(conn, &frame, ctx, &mut by_key)?;
    collect_hop_arm(conn, &frame, ctx, &mut by_key)?;

    let mut admitted: Vec<ScoredCandidate> = Vec::new();
    for candidate in by_key.into_values() {
        if !row_eligible(conn, &candidate, ctx, &frame)? {
            continue;
        }
        let evidence = candidate.evidence();
        let rankable = Rankable {
            eligible: true,
            hard_anchor: candidate.hard_anchor,
            evidence,
            strong_lexical: candidate.strong_lexical,
        };
        if admit(rankable).is_some() {
            admitted.push(candidate);
        }
    }
    admitted.sort_by(|a, b| a.rank_key().cmp(&b.rank_key()));
    admitted.truncate(k.max(1) * 3);

    let valid_at = frame.as_of.clone().unwrap_or_else(|| "current".to_string());
    let mut items: Vec<RecallItem> = admitted.into_iter().map(|candidate| scored_to_item(candidate, &frame, &valid_at, ctx)).collect();
    if token_budget > 0 {
        items = pack_budget(items, token_budget, query_text);
    }
    items.truncate(k.max(1));
    Ok(items)
}

fn scored_to_item(candidate: ScoredCandidate, frame: &QueryFrame, valid_at: &str, ctx: &RecallContext) -> RecallItem {
    let evidence = candidate.evidence();
    let admitted_by = admit(Rankable {
        eligible: true,
        hard_anchor: candidate.hard_anchor,
        evidence,
        strong_lexical: candidate.strong_lexical,
    })
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
            acl: if ctx.team_mode { "owner".to_string() } else { "solo".to_string() },
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
    );
    let mut item = RecallItem::new_with_why(candidate.source, relevance, candidate.excerpt, "clock-quorum".to_string());
    item.clock_why = Some(serde_json::to_value(&why).unwrap_or_else(|_| json!({"engine":"clock-quorum"})));
    item.status = Some(candidate.status).filter(|s| !s.is_empty());
    item.valid_from = candidate.valid_from;
    item.valid_until = candidate.valid_until;
    item
}

fn current_status_filters(frame: &QueryFrame, ctx: &RecallContext) -> Vec<String> {
    if ctx.as_of.is_some()
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

fn rank_to_relevance(key: &RankKey) -> f64 {
    let hard = if key.hard_anchor { 4.0 } else { 0.0 };
    let score = hard + f64::from(key.clock_count) + f64::from(key.strength) * 0.25 + f64::from(key.specificity) * 0.15;
    round4((score / 10.0).clamp(0.05, 1.0))
}

fn collect_write_arm(
    conn: &Connection, frame: &QueryFrame, query_text: &str, source_prefix: Option<&str>, ctx: &RecallContext,
    out: &mut HashMap<(String, i64), ScoredCandidate>,
) -> Result<(), String> {
    let fts_query = match clock_fts_query(frame) {
        Some(query) => query,
        None => {
            let groups = build_search_term_groups(query_text);
            if groups.is_empty() {
                return Ok(());
            }
            build_fts_query(&groups)
        }
    };
    let rare_terms: Vec<&str> = frame
        .terms
        .iter()
        .filter(|t| {
            !t.contains(' ')
                && !t.contains('/')
                && (t.len() >= 6 || t.chars().any(|c| c.is_ascii_digit()) || t.contains('_') || !graph::lexical_cluster_mates(t).is_empty())
        })
        .map(String::as_str)
        .collect();
    let quoted = !frame.quoted_phrases.is_empty();
    for kind in ["decision", "memory"] {
        let rows = fts_rows(conn, kind, &fts_query, FTS_CANDIDATE_CAP, source_prefix, ctx)?;
        for row in rows {
            let hay = row.excerpt.to_ascii_lowercase();
            let exact_rare = rare_terms.iter().filter(|term| hay_has_lexical(&hay, term)).count();
            // Direct write evidence: quoted phrase, exact rare term, morphological
            // variant, or closed lexicon mate. Ordinary BM25 without one of those
            // stays write=1 and cannot admit alone.
            let unique = exact_rare >= 1;
            let quoted_hit = quoted && frame.quoted_phrases.iter().any(|p| hay.contains(p));
            let write = if quoted_hit || unique { 2 } else { 1 };
            let strong_lexical = write == 2;
            upsert(out, loaded_candidate(conn, &row, 0, write, 0, 0, 0, false, strong_lexical, if write == 2 { 2 } else { 1 })?);
        }
    }
    Ok(())
}

fn clock_fts_query(frame: &QueryFrame) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    for term in &frame.terms {
        let t = term.trim();
        if t.len() < 2 || t.contains(' ') || t.contains('/') || t.contains('\\') {
            continue;
        }
        if matches!(t, "and" | "or" | "not") {
            continue;
        }
        if t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            terms.push(t.to_string());
        } else {
            terms.push(format!("\"{}\"", t.replace('"', "\"\"")));
        }
    }
    terms.sort();
    terms.dedup();
    if terms.is_empty() {
        return None;
    }
    let or_terms = terms.join(" OR ");
    let phrases: Vec<String> = frame
        .quoted_phrases
        .iter()
        .filter(|phrase| phrase.len() >= 2 && !phrase.contains('/'))
        .map(|phrase| format!("\"{}\"", phrase.replace('"', "\"\"")))
        .collect();
    if phrases.is_empty() {
        Some(or_terms)
    } else {
        Some(format!("({}) AND ({})", phrases.join(" AND "), or_terms))
    }
}

fn collect_anchor_arm(conn: &Connection, frame: &QueryFrame, ctx: &RecallContext, out: &mut HashMap<(String, i64), ScoredCandidate>) -> Result<(), String> {
    let strong: Vec<_> = frame.anchors.iter().filter(|a| a.specificity >= 2).cloned().collect();
    if strong.is_empty() {
        return Ok(());
    }
    let targets = lookup_targets_for_anchors(conn, &strong, STRONG_ANCHOR_CAP).map_err(|e| e.to_string())?;
    for target in targets {
        let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)? else {
            continue;
        };
        let matched: Vec<_> = frame.anchors.iter().filter(|a| a.specificity >= 2).cloned().collect();
        row.hard_anchor = matched.iter().any(|a| a.specificity >= 3);
        row.write = row.write.max(if row.hard_anchor { 2 } else { 1 });
        row.specificity = row.specificity.max(matched.iter().map(|a| a.specificity).max().unwrap_or(0));
        row.anchors = matched.into_iter().map(|a| WhyAnchor { kind: a.kind, value: a.value, specificity: a.specificity }).collect();
        upsert(out, row);
    }
    Ok(())
}

fn collect_truth_arm(conn: &Connection, frame: &QueryFrame, ctx: &RecallContext, out: &mut HashMap<(String, i64), ScoredCandidate>) -> Result<(), String> {
    if frame.entity_ids.is_empty() {
        return Ok(());
    }
    for entity_id in frame.entity_ids.iter().take(ENTITY_GRAPH_CAP) {
        let mut stmt = conn
            .prepare_cached("SELECT target_type, target_id FROM entity_mentions WHERE entity_id = ?1 ORDER BY target_type, target_id LIMIT ?2")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![entity_id, ENTITY_GRAPH_CAP as i64], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|e| e.to_string())?;
        for (target_type, target_id) in rows.flatten() {
            let Some(mut row) = load_target(conn, &target_type, target_id, ctx)? else {
                continue;
            };
            row.truth = 2;
            row.hard_anchor = true;
            row.specificity = row.specificity.max(2);
            if let Some(name) = canonical_entity_name(conn, *entity_id) {
                row.anchors.push(WhyAnchor { kind: crate::clockwork::AnchorKind::Entity, value: name, specificity: 2 });
            }
            upsert(out, row);
        }
    }
    let graph_hits = graph::entity_arm_candidates(conn, &frame.raw, ENTITY_GRAPH_CAP);
    for (source, excerpt, score) in graph_hits {
        if let Some((target_type, target_id)) = resolve_source(conn, &source) {
            let Some(mut row) = load_target(conn, target_type, target_id, ctx)? else {
                continue;
            };
            row.truth = row.truth.max(if score >= 1.0 { 2 } else { 1 });
            if score >= 1.0 {
                row.hard_anchor = true;
            }
            if row.excerpt.is_empty() {
                row.excerpt = excerpt;
            }
            upsert(out, row);
        }
    }
    Ok(())
}

fn collect_task_arm(conn: &Connection, frame: &QueryFrame, ctx: &RecallContext, out: &mut HashMap<(String, i64), ScoredCandidate>) -> Result<(), String> {
    let mut path_anchors = Vec::new();
    for path in &frame.paths {
        path_anchors.push(crate::clockwork::QueryAnchor { kind: crate::clockwork::AnchorKind::Path, value: normalize_task_path(path), specificity: 3 });
    }
    for symbol in &frame.symbols {
        path_anchors.push(crate::clockwork::QueryAnchor {
            kind: crate::clockwork::AnchorKind::Symbol,
            value: crate::clockwork::normalize_anchor_value(crate::clockwork::AnchorKind::Symbol, symbol),
            specificity: 3,
        });
    }
    for anchor in &frame.anchors {
        if matches!(anchor.kind, crate::clockwork::AnchorKind::Path | crate::clockwork::AnchorKind::Symbol) && anchor.specificity >= 2 {
            path_anchors.push(anchor.clone());
        }
    }
    if path_anchors.is_empty() {
        return Ok(());
    }
    let targets = lookup_targets_for_anchors(conn, &path_anchors, TASK_CANDIDATE_CAP).map_err(|e| e.to_string())?;
    for target in targets {
        let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)? else {
            continue;
        };
        row.task = 2;
        row.hard_anchor = true;
        row.specificity = 3;
        row.anchors
            .extend(path_anchors.iter().map(|a| WhyAnchor { kind: a.kind, value: a.value.clone(), specificity: a.specificity }));
        upsert(out, row);
    }
    if let Some(session) = frame.session_id.as_deref() {
        let session_anchor =
            [crate::clockwork::QueryAnchor { kind: crate::clockwork::AnchorKind::Session, value: session.to_ascii_lowercase(), specificity: 1 }];
        for target in lookup_targets_for_anchors(conn, &session_anchor, TASK_CANDIDATE_CAP).map_err(|e| e.to_string())? {
            let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)? else {
                continue;
            };
            row.task = row.task.max(1);
            upsert(out, row);
        }
    }
    Ok(())
}

fn normalize_task_path(raw: &str) -> String {
    let mut value = crate::clockwork::normalize_anchor_value(crate::clockwork::AnchorKind::Path, raw);
    loop {
        if let Some(stripped) = value.strip_suffix("/**") {
            value = stripped.to_string();
            continue;
        }
        if let Some(stripped) = value.strip_suffix("/*") {
            value = stripped.to_string();
            continue;
        }
        if let Some(stripped) = value.strip_suffix('*') {
            value = stripped.to_string();
            continue;
        }
        break;
    }
    value.trim_end_matches('/').to_string()
}

fn collect_history_arm(conn: &Connection, frame: &QueryFrame, ctx: &RecallContext, out: &mut HashMap<(String, i64), ScoredCandidate>) -> Result<(), String> {
    match frame.temporal_mode {
        TemporalMode::Current | TemporalMode::Any => return Ok(()),
        TemporalMode::Historical | TemporalMode::ExplicitAsOf => {}
    }
    let as_of = frame.as_of.as_deref();
    let mut stmt = conn
        .prepare_cached(
            "SELECT 'decision', id, decision, COALESCE(context, 'decision::' || id), owner_id, visibility,
                    created_at, status, valid_from, valid_until
             FROM decisions
             WHERE (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')
                    OR ?1 IS NOT NULL)
             ORDER BY id DESC
             LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![as_of, HISTORY_CANDIDATE_CAP as i64], |row| {
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
        })
        .map_err(|e| e.to_string())?;
    for (target_type, target_id, excerpt, source, owner_id, visibility, ts, status, valid_from, valid_until) in rows.flatten() {
        if !is_visible(owner_id, visibility.as_deref(), ctx) {
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
            status,
            valid_from,
            valid_until,
        };
        row.use_score = feedback_use_score(conn, &row.source);
        upsert(out, row);
    }
    Ok(())
}

fn collect_hop_arm(conn: &Connection, frame: &QueryFrame, ctx: &RecallContext, out: &mut HashMap<(String, i64), ScoredCandidate>) -> Result<(), String> {
    let mut seeds: Vec<ClockTarget> = out.keys().cloned().map(|(target_type, target_id)| ClockTarget { target_type, target_id }).collect();
    if seeds.is_empty() {
        for entity_id in frame.entity_ids.iter().take(ENTITY_GRAPH_CAP) {
            let mut stmt = conn
                .prepare_cached("SELECT target_type, target_id FROM entity_mentions WHERE entity_id = ?1 ORDER BY target_type, target_id LIMIT ?2")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![entity_id, ENTITY_GRAPH_CAP as i64], |row| {
                    Ok(ClockTarget { target_type: row.get(0)?, target_id: row.get(1)? })
                })
                .map_err(|e| e.to_string())?;
            seeds.extend(rows.flatten());
        }
    }
    seeds.sort();
    seeds.dedup();
    if seeds.is_empty() {
        return Ok(());
    }
    let hops = traverse_hops(conn, &seeds, 2, GRAPH_HOP_CAP).map_err(|e| e.to_string())?;
    for (target, hop) in hops {
        if hop == 0 {
            continue;
        }
        let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)? else {
            continue;
        };
        row.hops = hop;
        row.write = row.write.max(1);
        row.truth = row.truth.max(1);
        let relation = hop_relation(conn, &target).unwrap_or_else(|| "observed_with".to_string());
        if relation == "used_with" {
            row.task = row.task.max(1);
            row.write = row.write.max(1);
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

fn caller_acl_param(ctx: &RecallContext) -> Option<i64> {
    if ctx.team_mode {
        ctx.caller_id
    } else {
        None
    }
}

fn qualified_current_gates(alias: &str) -> String {
    format!(
        "{a}.status NOT IN ('superseded','archived') \
         AND ({a}.expires_at IS NULL OR julianday({a}.expires_at) > julianday('now')) \
         AND ({a}.valid_from IS NULL OR julianday({a}.valid_from) <= julianday('now')) \
         AND ({a}.valid_until IS NULL OR julianday({a}.valid_until) > julianday('now')) \
         AND ({a}.version_id IS NULL OR {a}.version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))",
        a = alias
    )
}

fn qualified_as_of_gates(alias: &str, bind: &str) -> String {
    format!(
        "({a}.expires_at IS NULL OR julianday({a}.expires_at) > julianday({b})) \
         AND ({a}.valid_from IS NULL OR julianday({a}.valid_from) <= julianday({b})) \
         AND ({a}.valid_until IS NULL OR julianday({a}.valid_until) > julianday({b})) \
         AND ({a}.version_id IS NULL OR {a}.version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))",
        a = alias,
        b = bind
    )
}

fn qualified_acl(alias: &str, bind: &str) -> String {
    format!(" AND ({b} IS NULL OR {a}.owner_id IS NULL OR {a}.owner_id = {b} OR {a}.visibility IN ('shared','team','public'))", a = alias, b = bind)
}

fn fts_rows(conn: &Connection, kind: &str, fts_query: &str, limit: usize, source_prefix: Option<&str>, ctx: &RecallContext) -> Result<Vec<LoadedRow>, String> {
    let source_like = source_prefix.map(|p| format!("{p}%"));
    let caller = caller_acl_param(ctx);
    let is_decision = kind == "decision";
    let alias = if is_decision { "d" } else { "m" };
    let gates = if ctx.as_of.is_some() { qualified_as_of_gates(alias, "?5") } else { format!("{} AND (?5 IS NULL OR 1)", qualified_current_gates(alias)) };
    let acl = qualified_acl(alias, "?4");
    let sql = if is_decision {
        format!(
            "SELECT d.id, d.decision, COALESCE(d.context, 'decision::' || d.id), d.owner_id, d.visibility,
                    d.created_at, d.status, d.valid_from, d.valid_until
             FROM decisions_fts fts JOIN decisions d ON d.id = fts.rowid
             WHERE decisions_fts MATCH ?1 AND {gates}
               AND (?3 IS NULL OR COALESCE(d.context, 'decision::' || d.id) LIKE ?3)
               {acl}
             ORDER BY bm25(decisions_fts, 6.6, 1.0) LIMIT ?2"
        )
    } else {
        format!(
            "SELECT m.id, m.text, COALESCE(m.source, 'memory::' || m.id), m.owner_id, m.visibility,
                    m.created_at, m.status, m.valid_from, m.valid_until
             FROM memories_fts fts JOIN memories m ON m.id = fts.rowid
             WHERE memories_fts MATCH ?1 AND {gates}
               AND (?3 IS NULL OR COALESCE(m.source, 'memory::' || m.id) LIKE ?3)
               {acl}
             ORDER BY bm25(memories_fts, 4.6, 1.7, 2.2) LIMIT ?2"
        )
    };
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let as_of = ctx.as_of.clone();
    let rows = stmt
        .query_map(params![fts_query, limit as i64, source_like, caller, as_of], |row| {
            Ok(LoadedRow {
                target_type: if is_decision { "decision".to_string() } else { "memory".to_string() },
                target_id: row.get(0)?,
                excerpt: row.get(1)?,
                source: row.get(2)?,
                owner_id: row.get(3)?,
                visibility: row.get(4)?,
                ts_raw: row.get(5)?,
                status: row.get(6)?,
                valid_from: row.get(7)?,
                valid_until: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        if !is_visible(row.owner_id, row.visibility.as_deref(), ctx) {
            continue;
        }
        out.push(row);
    }
    Ok(out)
}

struct LoadedRow {
    target_type: String,
    target_id: i64,
    excerpt: String,
    source: String,
    owner_id: Option<i64>,
    visibility: Option<String>,
    ts_raw: Option<String>,
    status: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
}

fn loaded_candidate(
    conn: &Connection, row: &LoadedRow, hops: u8, write: u8, truth: u8, task: u8, history: u8, hard_anchor: bool, strong_lexical: bool, specificity: u8,
) -> Result<ScoredCandidate, String> {
    Ok(ScoredCandidate {
        target_type: row.target_type.clone(),
        target_id: row.target_id,
        source: row.source.clone(),
        excerpt: row.excerpt.clone(),
        owner_id: row.owner_id,
        visibility: row.visibility.clone(),
        ts: crate::handlers::parse_timestamp_ms(row.ts_raw.as_deref().unwrap_or("")),
        hops,
        write,
        truth,
        task,
        history,
        hard_anchor,
        strong_lexical,
        specificity,
        fts_rank: (write as i64) * 100,
        use_score: feedback_use_score(conn, &row.source),
        anchors: Vec::new(),
        links: Vec::new(),
        status: row.status.clone(),
        valid_from: row.valid_from.clone(),
        valid_until: row.valid_until.clone(),
    })
}

fn load_target(conn: &Connection, target_type: &str, target_id: i64, ctx: &RecallContext) -> Result<Option<ScoredCandidate>, String> {
    let caller = caller_acl_param(ctx);
    let as_of = ctx.as_of.clone();
    let gates = if ctx.as_of.is_some() {
        qualified_as_of_gates("x", "?3").replace("x.", "")
    } else {
        format!("{} AND (?3 IS NULL OR 1)", qualified_current_gates("x").replace("x.", ""))
    };
    let acl = qualified_acl("x", "?2").replace("x.", "");
    let sql = if target_type == "memory" {
        format!(
            "SELECT text, COALESCE(source, 'memory::' || id), owner_id, visibility, created_at, status, valid_from, valid_until
             FROM memories WHERE id = ?1 AND {gates} {acl}"
        )
    } else {
        format!(
            "SELECT decision, COALESCE(context, 'decision::' || id), owner_id, visibility, created_at, status, valid_from, valid_until
             FROM decisions WHERE id = ?1 AND {gates} {acl}"
        )
    };
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let row = stmt
        .query_row(params![target_id, caller, as_of], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((excerpt, source, owner_id, visibility, ts, status, valid_from, valid_until)) = row else {
        return Ok(None);
    };
    if !is_visible(owner_id, visibility.as_deref(), ctx) {
        return Ok(None);
    }
    Ok(Some(ScoredCandidate {
        target_type: target_type.to_string(),
        target_id,
        source: source.clone(),
        excerpt,
        owner_id,
        visibility,
        ts: crate::handlers::parse_timestamp_ms(ts.as_deref().unwrap_or("")),
        hops: 0,
        write: 0,
        truth: 0,
        task: 0,
        history: 0,
        hard_anchor: false,
        strong_lexical: false,
        specificity: 0,
        fts_rank: 0,
        use_score: feedback_use_score(conn, &source),
        anchors: Vec::new(),
        links: Vec::new(),
        status,
        valid_from,
        valid_until,
    }))
}

fn row_eligible(conn: &Connection, candidate: &ScoredCandidate, ctx: &RecallContext, frame: &QueryFrame) -> Result<bool, String> {
    if !is_visible(candidate.owner_id, candidate.visibility.as_deref(), ctx) {
        return Ok(false);
    }
    if ctx.team_mode && candidate.owner_id.is_some() && candidate.owner_id != ctx.caller_id {
        if !matches!(candidate.visibility.as_deref(), Some("shared") | Some("team")) {
            return Ok(false);
        }
    }
    if matches!(frame.temporal_mode, TemporalMode::Current) || ctx.as_of.is_some() {
        let exists = load_target(conn, &candidate.target_type, candidate.target_id, ctx)?.is_some();
        if !exists {
            return Ok(false);
        }
    }
    if !path_context_compatible(conn, candidate, frame)? {
        return Ok(false);
    }
    Ok(true)
}

fn path_context_compatible(conn: &Connection, candidate: &ScoredCandidate, frame: &QueryFrame) -> Result<bool, String> {
    if frame.paths.is_empty() {
        return Ok(true);
    }
    let query_paths: Vec<String> = frame.paths.iter().map(|p| normalize_task_path(p)).filter(|p| !p.is_empty()).collect();
    if query_paths.is_empty() {
        return Ok(true);
    }
    let candidate_paths = candidate_path_values(conn, &candidate.target_type, candidate.target_id)?;
    if candidate_paths.is_empty() {
        return Ok(true);
    }
    Ok(candidate_paths.iter().any(|cp| query_paths.iter().any(|qp| path_compatible(cp, qp))))
}

fn candidate_path_values(conn: &Connection, target_type: &str, target_id: i64) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT a.value FROM clock_anchors a
             JOIN clock_anchor_evidence e ON e.anchor_id = a.id
             WHERE e.target_type = ?1 AND e.target_id = ?2 AND a.kind = 'path'",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![target_type, target_id], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

fn path_compatible(candidate: &str, query: &str) -> bool {
    if candidate == query {
        return true;
    }
    candidate.starts_with(&(query.to_string() + "/")) || query.starts_with(&(candidate.to_string() + "/"))
}

fn upsert(out: &mut HashMap<(String, i64), ScoredCandidate>, incoming: ScoredCandidate) {
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
            existing.hops = if existing.hops == 0 { incoming.hops } else { existing.hops.min(incoming.hops) };
            if existing.excerpt.len() < incoming.excerpt.len() && existing.excerpt.is_empty() {
                existing.excerpt = incoming.excerpt.clone();
            }
            for anchor in &incoming.anchors {
                if !existing.anchors.iter().any(|a| a.kind == anchor.kind && a.value == anchor.value) {
                    existing.anchors.push(anchor.clone());
                }
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

fn resolve_source<'a>(conn: &Connection, source: &'a str) -> Option<(&'a str, i64)> {
    if let Some(id) = source.strip_prefix("memory::").and_then(|s| s.parse().ok()) {
        return Some(("memory", id));
    }
    if let Some(id) = source.strip_prefix("decision::").and_then(|s| s.parse().ok()) {
        return Some(("decision", id));
    }
    conn.query_row("SELECT id FROM decisions WHERE context = ?1 LIMIT 1", params![source], |row| row.get(0))
        .optional()
        .ok()
        .flatten()
        .map(|id| ("decision", id))
        .or_else(|| {
            conn.query_row("SELECT id FROM memories WHERE source = ?1 LIMIT 1", params![source], |row| row.get(0))
                .optional()
                .ok()
                .flatten()
                .map(|id| ("memory", id))
        })
}

fn canonical_entity_name(conn: &Connection, entity_id: i64) -> Option<String> {
    conn.query_row("SELECT canonical_name FROM entities WHERE id = ?1", params![entity_id], |row| row.get(0))
        .optional()
        .ok()
        .flatten()
}

fn feedback_use_score(conn: &Connection, source: &str) -> i64 {
    let pos: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(CASE WHEN signal > 0 THEN signal ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN signal < 0 THEN -signal ELSE 0 END), 0)
             FROM recall_feedback WHERE result_source = ?1",
            params![source],
            |row| Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?)),
        )
        .optional()
        .ok()
        .flatten()
        .map(|(p, n)| p - (2.0 * n))
        .unwrap_or(0.0);
    pos.round() as i64
}

fn pack_budget(items: Vec<RecallItem>, token_budget: usize, query_text: &str) -> Vec<RecallItem> {
    let mut spent = 0usize;
    let mut kept = Vec::new();
    for (idx, mut item) in items.into_iter().enumerate() {
        let remaining = token_budget.saturating_sub(spent);
        if remaining <= MIN_BUDGET_HEADROOM_TOKENS {
            break;
        }
        let tokens = recall_item_token_cost(&item);
        if tokens <= remaining {
            item.tokens = Some(tokens);
            spent += tokens;
            kept.push(item);
            continue;
        }
        let cap = budget_rank_char_cap(token_budget, idx, query_text).min((remaining as f64 * 3.6) as usize).max(MIN_EXCERPT_CHARS);
        if let Some((excerpt, used)) = fit_excerpt_to_remaining_budget(&item.source, &item.excerpt, query_text, cap, remaining) {
            item.excerpt = excerpt;
            item.tokens = Some(used);
            spent += used;
            kept.push(item);
        }
    }
    kept
}

fn hop_relation(conn: &Connection, target: &ClockTarget) -> Option<String> {
    conn.query_row(
        "SELECT relation FROM clock_links
         WHERE (src_type = ?1 AND src_id = ?2) OR (dst_type = ?1 AND dst_id = ?2)
         ORDER BY CASE relation WHEN 'used_with' THEN 0 ELSE 1 END, relation
         LIMIT 1",
        params![target.target_type, target.target_id],
        |row| row.get(0),
    )
    .ok()
}

fn stable_anchors(mut anchors: Vec<WhyAnchor>) -> Vec<WhyAnchor> {
    anchors.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.value.cmp(&b.value)).then_with(|| b.specificity.cmp(&a.specificity)));
    anchors.dedup_by(|a, b| a.kind == b.kind && a.value == b.value);
    anchors
}

fn stable_links(mut links: Vec<LinkHit>) -> Vec<LinkHit> {
    links.sort_by(|a, b| a.relation.cmp(&b.relation).then_with(|| a.from.cmp(&b.from)).then_with(|| a.to.cmp(&b.to)));
    links.dedup();
    links
}

pub(crate) fn clock_health_payload(conn: &Connection) -> Value {
    let generation = current_generation(conn).unwrap_or(0);
    let anchors: i64 = conn.query_row("SELECT COUNT(*) FROM clock_anchors", [], |row| row.get(0)).unwrap_or(0);
    let graph_ready = conn
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'entities'", [], |row| row.get::<_, i64>(0))
        .unwrap_or(0)
        > 0;
    json!({
        "engine": "clock-quorum",
        "modelFree": true,
        "anchorsReady": anchors > 0 || generation > 0,
        "graphReady": graph_ready,
        "derivedGeneration": generation
    })
}

#[allow(dead_code)]
pub(crate) fn estimate_query_tokens(query_text: &str) -> usize {
    estimate_tokens(query_text)
}

#[allow(dead_code)]
pub(crate) fn merge_search_candidates(items: Vec<SearchCandidate>) -> BTreeMap<String, SearchCandidate> {
    let mut map = BTreeMap::new();
    for item in items {
        map.entry(item.source.clone()).or_insert(item);
    }
    map
}

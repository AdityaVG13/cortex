use crate::clockwork::{
    admit_with_lineage, current_generation, expand_query_frame, hay_has_lexical, lookup_targets_for_anchors, parse_query_frame, query_signature, traverse_hops,
    ClockEvidence, ClockTarget, ClockWhy, FilterEvidence, LinkHit, QueryFrame, RankKey, Rankable, TemporalMode, TieBreak, WhyAnchor, Witness, WitnessDomain,
    ENTITY_GRAPH_CAP, FTS_CANDIDATE_CAP, GRAPH_HOP_CAP, HISTORY_CANDIDATE_CAP, STRONG_ANCHOR_CAP, TASK_CANDIDATE_CAP,
};
use crate::graph;
use crate::traces;

/// Per-arm provenance markers. Each collector arm stamps the candidates it
/// contributes to; `scored_to_item` surfaces the merged list as
/// `clockVotes.admittedArms` in the why payload. Additive metadata only:
/// markers never influence admission, ranking, or tie-breaking.
const ARM_LEXICAL: &str = "lexical";
const ARM_ANCHOR: &str = "anchor";
const ARM_TRUTH: &str = "truth";
const ARM_TASK: &str = "task";
const ARM_HISTORY: &str = "history";
const ARM_HOP: &str = "hop";

fn mark_arm(arms: &mut Vec<&'static str>, arm: &'static str) {
    if !arms.contains(&arm) {
        arms.push(arm);
    }
}

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
    arms: Vec<&'static str>,
    status: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
    /// Provenance-bearing witnesses; direct ones originate at this row,
    /// derived ones carry the origin of the seed they were reached from.
    witnesses: Vec<Witness>,
    /// Constraint-like kind: ranked ahead of everything but eligibility.
    required_role: bool,
    /// Has an open CONTRADICTS conflict: surfaced before plain support.
    contradiction: bool,
}

/// Per-route-family quotas. A global top-k across memory kinds can hide a
/// rare old constraint behind lexical noise; each family is bounded on its
/// own and reports exhaustion in the trace.
pub const ROUTE_QUOTAS: [(&str, usize); 6] = [("lexical", 24), ("anchor", 24), ("truth", 16), ("task", 16), ("history", 16), ("hop", 16)];

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RouteTrace {
    pub collected: std::collections::BTreeMap<String, usize>,
    pub kept: std::collections::BTreeMap<String, usize>,
    pub exhausted: Vec<String>,
    pub admitted: usize,
    pub leads: usize,
    pub rank_tuple: &'static str,
}

thread_local! {
    static LAST_ROUTE_TRACE: std::cell::RefCell<Option<RouteTrace>> = const { std::cell::RefCell::new(None) };
}
pub fn take_last_route_trace() -> Option<RouteTrace> {
    LAST_ROUTE_TRACE.with(|cell| cell.borrow_mut().take())
}

/// Apply per-family quotas: within each family keep the best-ranked rows;
/// a row survives if it survives in any family it was collected by.
fn apply_route_quotas(by_key: &mut HashMap<(String, i64), ScoredCandidate>, trace: &mut RouteTrace) {
    let mut keep: std::collections::HashSet<(String, i64)> = std::collections::HashSet::new();
    for (family, cap) in ROUTE_QUOTAS {
        let mut members: Vec<(&(String, i64), &ScoredCandidate)> = by_key.iter().filter(|(_, c)| c.arms.contains(&family)).collect();
        trace.collected.insert(family.to_string(), members.len());
        members.sort_by(|a, b| a.1.rank_key().cmp(&b.1.rank_key()));
        if members.len() > cap {
            trace.exhausted.push(family.to_string());
        }
        for (key, _) in members.into_iter().take(cap) {
            keep.insert(key.clone());
        }
        trace.kept.insert(family.to_string(), cap.min(trace.collected[family]));
    }
    by_key.retain(|key, _| keep.contains(key));
}

impl ScoredCandidate {
    fn origin(&self) -> String {
        format!("{}::{}", self.target_type, self.target_id)
    }
    fn witness(&mut self, domain: WitnessDomain, key: impl Into<String>, specificity: u8) {
        // Direct evidence channels on one row (its text, its anchor
        // projections, its history) are distinct origins; a derived hop is not.
        let origin = format!("{:?}:{}", domain, self.origin()).to_ascii_lowercase();
        self.witnesses.push(Witness::direct(domain, origin, key, specificity));
    }
    fn evidence(&self) -> ClockEvidence {
        ClockEvidence { write: self.write, truth: self.truth, task: self.task, history: self.history }
    }

    fn rank_key(&self) -> RankKey {
        let mut key = RankKey::from_parts(
            self.hard_anchor,
            self.evidence(),
            self.specificity,
            self.hops,
            self.fts_rank,
            self.use_score,
            self.ts,
            self.target_type.clone(),
            self.target_id,
        );
        key.required_role = self.required_role;
        key.lineage = crate::clockwork::independent_support(&self.witnesses).min(255) as u8;
        key.contradiction = self.contradiction;
        key
    }
}

pub fn run_clock_quorum_recall(
    conn: &Connection, query_text: &str, token_budget: usize, k: usize, ctx: &RecallContext, source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    let query_text = crate::clockwork::bound_query_text(query_text);
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
    // `source_prefix` is a provenance filter (memories.source / decision
    // identity), not a filesystem path. Pushing it onto `frame.paths` made
    // the task arm LIKE-match children of the prefix — including after FTS
    // skipped an empty/stop-word query — and treated trailing `*` as a glob.
    let source_prefix = source_prefix.map(str::trim).filter(|s| !s.is_empty());
    frame.entity_ids = graph::resolve_query(conn, query_text);
    expand_query_frame(conn, &mut frame);
    let _signature = query_signature(&frame);

    let mut by_key: HashMap<(String, i64), ScoredCandidate> = HashMap::new();
    collect_write_arm(conn, &frame, query_text, source_prefix, ctx, &mut by_key)?;
    collect_anchor_arm(conn, &frame, ctx, &mut by_key)?;
    collect_truth_arm(conn, &frame, ctx, &mut by_key)?;
    collect_task_arm(conn, &frame, ctx, &mut by_key)?;
    collect_history_arm(conn, &frame, ctx, source_prefix, &mut by_key)?;
    collect_hop_arm(conn, &frame, ctx, &mut by_key)?;

    let mut trace = RouteTrace { rank_tuple: crate::clockwork::RANK_TUPLE_VERSION, ..RouteTrace::default() };
    apply_route_quotas(&mut by_key, &mut trace);
    annotate_roles(conn, &mut by_key);
    let total = by_key.len();
    let mut admitted: Vec<ScoredCandidate> = Vec::new();
    for candidate in by_key.into_values() {
        if !row_eligible(conn, &candidate, ctx, &frame, source_prefix)? {
            continue;
        }
        let evidence = candidate.evidence();
        let rankable = Rankable {
            eligible: true,
            hard_anchor: candidate.hard_anchor,
            evidence,
            strong_lexical: candidate.strong_lexical,
        };
        if admit_with_lineage(rankable, &candidate.witnesses).is_some() {
            admitted.push(candidate);
        }
    }
    admitted.sort_by(|a, b| a.rank_key().cmp(&b.rank_key()));
    admitted.truncate(k.max(1).saturating_mul(3));
    trace.admitted = admitted.len();
    trace.leads = total.saturating_sub(admitted.len());
    LAST_ROUTE_TRACE.with(|cell| *cell.borrow_mut() = Some(trace));

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
    let witnesses = candidate.witnesses.clone();
    let arms_for_questions = candidate.arms.clone();
    let status_for_questions = candidate.status.clone();
    let admitted_by = admit_with_lineage(
        Rankable { eligible: true, hard_anchor: candidate.hard_anchor, evidence, strong_lexical: candidate.strong_lexical },
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
    )
    .with_lineage(witnesses, &arms_for_questions, &status_for_questions);
    let admitted_arms =
        Value::Array(candidate.arms.iter().map(|arm| Value::String((*arm).to_string())).collect());
    let mut item = RecallItem::new_with_why(candidate.source, relevance, candidate.excerpt, "clock-quorum".to_string());
    let mut why_value = serde_json::to_value(&why).unwrap_or_else(|_| json!({"engine":"clock-quorum"}));
    if let Some(votes) = why_value.get_mut("clockVotes").and_then(|v| v.as_object_mut()) {
        votes.insert("admittedArms".to_string(), admitted_arms);
    }
    item.clock_why = Some(why_value);
    item.status = Some(candidate.status).filter(|s| !s.is_empty());
    item.valid_from = candidate.valid_from;
    item.valid_until = candidate.valid_until;
    item
}

fn current_status_filters(frame: &QueryFrame, ctx: &RecallContext) -> Vec<String> {
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
fn annotate_roles(conn: &Connection, by_key: &mut HashMap<(String, i64), ScoredCandidate>) {
    for ((target_type, target_id), candidate) in by_key.iter_mut() {
        if target_type != "decision" {
            continue;
        }
        if let Ok(kind) = conn.query_row("SELECT COALESCE(type, 'decision') FROM decisions WHERE id = ?1", params![target_id], |r| r.get::<_, String>(0)) {
            candidate.required_role = matches!(kind.as_str(), "constraint" | "policy" | "rule" | "convention" | "contract" | "preference");
        }
        candidate.contradiction = conn
            .query_row(
                "SELECT COUNT(*) FROM decision_conflicts WHERE classification = 'CONTRADICTS' AND status = 'open' AND (source_decision_id = ?1 OR target_decision_id = ?1)",
                params![target_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
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
    if fts_query.is_empty() {
        return Ok(());
    }
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
        if !source_prefix_applies_to_kind(kind, source_prefix) {
            continue;
        }
        let rows = fts_rows(conn, kind, &fts_query, FTS_CANDIDATE_CAP, source_prefix, ctx)?;
        for row in rows {
            let hay = row.excerpt.to_ascii_lowercase();
            let exact_rare = rare_terms.iter().filter(|term| hay_has_lexical(&hay, term)).count();
            // Direct write evidence: quoted phrase, exact rare term, morphological
            // variant, or closed lexicon mate. Ordinary BM25 without one of those
            // stays write=1 and cannot admit alone.
            let unique = exact_rare >= 1;
            // FTS already stripped `*`/`^` inside quotes; a raw contains()
            // still treats `foo*` as a glob needle and misses the hit.
            let quoted_hit = quoted
                && frame.quoted_phrases.iter().any(|p| {
                    lexical_needle(p).is_some_and(|needle| hay.contains(&needle))
                });
            let write = if quoted_hit || unique { 2 } else { 1 };
            let strong_lexical = write == 2;
            let mut candidate = loaded_candidate(conn, &row, 0, write, 0, 0, 0, false, strong_lexical, if write == 2 { 2 } else { 1 })?;
            mark_arm(&mut candidate.arms, ARM_LEXICAL);
            candidate.witness(WitnessDomain::Lexical, fts_query.chars().take(40).collect::<String>(), if write == 2 { 2 } else { 1 });
            upsert(out, candidate);
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
        (false, false) => Some(format!("({}) AND ({})", phrases.join(" AND "), terms.join(" OR "))),
    }
}

fn collect_anchor_arm(conn: &Connection, frame: &QueryFrame, ctx: &RecallContext, out: &mut HashMap<(String, i64), ScoredCandidate>) -> Result<(), String> {
    let strong: Vec<_> = frame.anchors.iter().filter(|a| a.specificity >= 2).cloned().collect();
    if strong.is_empty() {
        return Ok(());
    }
    let targets = crate::clockwork::lookup_targets_with_matches(conn, &strong, STRONG_ANCHOR_CAP).map_err(|e| e.to_string())?;
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
            paths: crate::clockwork::target_anchor_values(conn, &target, crate::clockwork::AnchorKind::Path).map_err(|e| e.to_string())?,
            hosts: crate::clockwork::target_anchor_values(conn, &target, crate::clockwork::AnchorKind::UrlHost).map_err(|e| e.to_string())?,
        };
        let (matched, demotions) = crate::clockwork::qualify_matches(&matched, &namespace, &scope);
        for reason in demotions {
            row.witness(WitnessDomain::Anchor, format!("demoted:{reason}"), 1);
        }
        row.hard_anchor = matched.iter().any(|a| a.specificity >= 3);
        let best = matched.iter().map(|a| a.specificity).max().unwrap_or(2);
        let key = matched.iter().map(|a| a.value.clone()).next().unwrap_or_default();
        row.witness(WitnessDomain::Anchor, key, best);
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
            mark_arm(&mut row.arms, ARM_TRUTH);
            let expanded = frame.expanded_entity_ids.contains(entity_id);
            // An entity named in the query itself is hard; one reached only
            // through expansion is a low-specificity access aid.
            row.truth = row.truth.max(if expanded { 1 } else { 2 });
            row.hard_anchor |= !expanded;
            row.specificity = row.specificity.max(if expanded { 1 } else { 2 });
            let name = canonical_entity_name(conn, *entity_id).unwrap_or_else(|| entity_id.to_string());
            row.witness(WitnessDomain::Entity, name.clone(), if expanded { 1 } else { 3 });
            row.anchors.push(WhyAnchor { kind: crate::clockwork::AnchorKind::Entity, value: name, specificity: 2 });
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
            row.witness(WitnessDomain::Entity, source.clone(), if score >= 1.0 { 3 } else { 1 });
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
    let targets = crate::clockwork::lookup_targets_with_matches(conn, &path_anchors, TASK_CANDIDATE_CAP).map_err(|e| e.to_string())?;
    let scope = crate::clockwork::AnchorScope::from_query(&ctx.paths, &frame.anchors);
    for (target, matched) in targets {
        let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)? else {
            continue;
        };
        mark_arm(&mut row.arms, ARM_TASK);
        // Same qualification as the anchor arm: the task's own paths are
        // identity only inside the task's repository root.
        let namespace = crate::clockwork::RowNamespace {
            paths: crate::clockwork::target_anchor_values(conn, &target, crate::clockwork::AnchorKind::Path).map_err(|e| e.to_string())?,
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
        row.witness(WitnessDomain::Task, matched.iter().map(|a| a.value.clone()).next().unwrap_or_default(), if hard { 3 } else { 2 });
        row.anchors.extend(matched.iter().map(|a| WhyAnchor { kind: a.kind, value: a.value.clone(), specificity: a.specificity }));
        upsert(out, row);
    }
    if let Some(session) = frame.session_id.as_deref() {
        let session_anchor =
            [crate::clockwork::QueryAnchor { kind: crate::clockwork::AnchorKind::Session, value: session.to_ascii_lowercase(), specificity: 1 }];
        for target in lookup_targets_for_anchors(conn, &session_anchor, TASK_CANDIDATE_CAP).map_err(|e| e.to_string())? {
            let Some(mut row) = load_target(conn, &target.target_type, target.target_id, ctx)? else {
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

fn collect_history_arm(
    conn: &Connection, frame: &QueryFrame, ctx: &RecallContext, source_prefix: Option<&str>,
    out: &mut HashMap<(String, i64), ScoredCandidate>,
) -> Result<(), String> {
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
        row.witness(WitnessDomain::History, as_of.clone().unwrap_or_else(|| "recent".into()), if as_of.is_some() { 2 } else { 1 });
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
    // Every hop-discovered row inherits the origin of the seed set it was
    // reached from: one traversal is one lineage family and can count at
    // most once toward relevance, never as two independent clocks.
    let seed_origin = format!("hop:{}", seeds.iter().map(|s| format!("{}::{}", s.target_type, s.target_id)).collect::<Vec<_>>().join("+"));
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
        row.witnesses.push(Witness::derived(WitnessDomain::Hop, seed_origin.clone(), relation.clone(), hop));
        if relation == "used_with" {
            // Explicit feedback ("this was useful for that query") is its own
            // evidentiary origin — the feedback ledger — distinct from the
            // seed row, so it can pair with the route. It is usefulness under
            // a task context, never a truth vote; rejection removes it.
            row.task = row.task.max(1);
            row.witnesses.push(Witness::direct(WitnessDomain::Task, format!("used_with:{}::{}", target.target_type, target.target_id), "feedback", 1));
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

/// Empty `as_of` is omitted JSON, not "valid at the empty instant".
/// `julianday('')` is NULL and would fail every temporal gate.
fn as_of_bind(ctx: &RecallContext) -> Option<&str> {
    ctx.as_of.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn qualified_validity_gates(alias: &str) -> String {
    format!(
        "({a}.expires_at IS NULL OR TRIM({a}.expires_at) = '' OR julianday({a}.expires_at) > julianday('now')) \
         AND ({a}.valid_from IS NULL OR TRIM({a}.valid_from) = '' OR julianday({a}.valid_from) <= julianday('now')) \
         AND ({a}.valid_until IS NULL OR TRIM({a}.valid_until) = '' OR julianday({a}.valid_until) > julianday('now')) \
         AND ({a}.version_id IS NULL OR {a}.version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))",
        a = alias
    )
}

fn qualified_current_gates(alias: &str) -> String {
    format!(
        "{a}.status NOT IN ('superseded','archived') AND {rest}",
        a = alias,
        rest = qualified_validity_gates(alias)
    )
}

fn qualified_as_of_gates(alias: &str, bind: &str) -> String {
    format!(
        "({a}.expires_at IS NULL OR TRIM({a}.expires_at) = '' OR julianday({a}.expires_at) > julianday({b})) \
         AND ({a}.valid_from IS NULL OR TRIM({a}.valid_from) = '' OR julianday({a}.valid_from) <= julianday({b})) \
         AND ({a}.valid_until IS NULL OR TRIM({a}.valid_until) = '' OR julianday({a}.valid_until) > julianday({b})) \
         AND ({a}.version_id IS NULL OR {a}.version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))",
        a = alias,
        b = bind
    )
}

fn qualified_acl(alias: &str, bind: &str) -> String {
    format!(" AND ({b} IS NULL OR {a}.owner_id IS NULL OR {a}.owner_id = {b} OR {a}.visibility IN ('shared','team','public'))", a = alias, b = bind)
}

/// FTS MATCH already quotes `*`/`^` away; lexical contains() must use the
/// same needle or a quoted glob never scores as a quoted hit.
fn lexical_needle(raw: &str) -> Option<String> {
    let t = raw
        .chars()
        .map(|ch| if ch == '*' || ch == '^' { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if t.is_empty() || t.replace('"', "").is_empty() {
        None
    } else {
        Some(t)
    }
}

fn source_prefix_is_path(prefix: &str) -> bool {
    prefix.contains('/') && !prefix.contains("::")
}

fn source_prefix_applies_to_kind(kind: &str, prefix: Option<&str>) -> bool {
    let Some(prefix) = prefix.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let lower = prefix.to_ascii_lowercase();
    if lower.starts_with("memory::") {
        return matches!(kind, "memory" | "memories");
    }
    if lower.starts_with("decision::") {
        return matches!(kind, "decision" | "decisions");
    }
    true
}

fn starts_with_ascii_ignore_case(source: &str, prefix: &str) -> bool {
    let pb = prefix.as_bytes();
    let sb = source.as_bytes();
    sb.len() >= pb.len() && sb[..pb.len()].eq_ignore_ascii_case(pb)
}

fn scoped_source_matches(source: &str, prefix: Option<&str>) -> bool {
    let Some(prefix) = prefix.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    if !starts_with_ascii_ignore_case(source, prefix) {
        return false;
    }
    if source_prefix_is_path(prefix) {
        let rest = &source.as_bytes()[prefix.len()..];
        return rest.is_empty() || rest.first() == Some(&b'/');
    }
    true
}

fn candidate_matches_source_scope(kind: &str, id: i64, source: &str, prefix: Option<&str>) -> bool {
    if !source_prefix_applies_to_kind(kind, prefix) {
        return false;
    }
    if scoped_source_matches(source, prefix) {
        return true;
    }
    let ident = if kind == "decision" || kind == "decisions" {
        format!("decision::{id}")
    } else {
        format!("memory::{id}")
    };
    scoped_source_matches(&ident, prefix)
}

/// LIKE `prefix%` plus a slash-boundary guard so `src/app` does not fill
/// FTS LIMIT with `src/application`. Identity prefixes skip the guard (`?6`).
/// Synthetic `{kind}::{id}` still matches when the display source is context.
fn source_scope_sql(col: &str, ident: &str) -> String {
    format!(
        "(?3 IS NULL OR (({col} LIKE ?3 ESCAPE '\\' AND (?6 IS NULL OR instr('/' || lower({col}) || '/', '/' || lower(?6) || '/') > 0)) OR {ident} LIKE ?3 ESCAPE '\\'))"
    )
}

fn fts_rows(conn: &Connection, kind: &str, fts_query: &str, limit: usize, source_prefix: Option<&str>, ctx: &RecallContext) -> Result<Vec<LoadedRow>, String> {
    if !source_prefix_applies_to_kind(kind, source_prefix) {
        return Ok(Vec::new());
    }
    let source_like = source_prefix.map(like_prefix);
    let path_guard = source_prefix.filter(|p| source_prefix_is_path(p));
    let caller = caller_acl_param(ctx);
    let is_decision = kind == "decision";
    let alias = if is_decision { "d" } else { "m" };
    let gates = if as_of_bind(ctx).is_some() {
        qualified_as_of_gates(alias, "?5")
    } else if ctx.include_cold {
        format!("{} AND (?5 IS NULL OR 1)", qualified_validity_gates(alias))
    } else {
        format!("{} AND (?5 IS NULL OR 1)", qualified_current_gates(alias))
    };
    let acl = qualified_acl(alias, "?4");
    let scope = if is_decision {
        source_scope_sql("COALESCE(d.context, 'decision::' || d.id)", "'decision::' || d.id")
    } else {
        source_scope_sql("COALESCE(m.source, 'memory::' || m.id)", "'memory::' || m.id")
    };
    let sql = if is_decision {
        format!(
            "SELECT d.id, d.decision, COALESCE(d.context, 'decision::' || d.id), d.owner_id, d.visibility,
                    d.created_at, d.status, d.valid_from, d.valid_until
             FROM decisions_fts fts JOIN decisions d ON d.id = fts.rowid
             WHERE decisions_fts MATCH ?1 AND {gates}
               AND {scope}
               {acl}
             ORDER BY bm25(decisions_fts, 6.6, 1.0) LIMIT ?2"
        )
    } else {
        format!(
            "SELECT m.id, m.text, COALESCE(m.source, 'memory::' || m.id), m.owner_id, m.visibility,
                    m.created_at, m.status, m.valid_from, m.valid_until
             FROM memories_fts fts JOIN memories m ON m.id = fts.rowid
             WHERE memories_fts MATCH ?1 AND {gates}
               AND {scope}
               {acl}
             ORDER BY bm25(memories_fts, 4.6, 1.7, 2.2) LIMIT ?2"
        )
    };
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let as_of = as_of_bind(ctx);
    let rows = stmt
        .query_map(params![fts_query, limit as i64, source_like, caller, as_of, path_guard], |row| {
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
        if !candidate_matches_source_scope(&row.target_type, row.target_id, &row.source, source_prefix) {
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
        arms: Vec::new(),
        status: row.status.clone(),
        valid_from: row.valid_from.clone(),
        valid_until: row.valid_until.clone(),
        witnesses: Vec::new(),
        required_role: false,
        contradiction: false,
    })
}

fn load_target(conn: &Connection, target_type: &str, target_id: i64, ctx: &RecallContext) -> Result<Option<ScoredCandidate>, String> {
    let caller = caller_acl_param(ctx);
    let as_of = as_of_bind(ctx);
    let gates = if as_of.is_some() {
        qualified_as_of_gates("x", "?3").replace("x.", "")
    } else if ctx.include_cold {
        format!(
            "{} AND (?3 IS NULL OR 1)",
            qualified_validity_gates("x").replace("x.", "")
        )
    } else {
        format!(
            "{} AND (?3 IS NULL OR 1)",
            qualified_current_gates("x").replace("x.", "")
        )
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
        arms: Vec::new(),
        status,
        valid_from,
        valid_until,
        witnesses: Vec::new(),
        required_role: false,
        contradiction: false,
    }))
}

fn row_eligible(
    conn: &Connection, candidate: &ScoredCandidate, ctx: &RecallContext, frame: &QueryFrame, source_prefix: Option<&str>,
) -> Result<bool, String> {
    if !candidate_matches_source_scope(&candidate.target_type, candidate.target_id, &candidate.source, source_prefix) {
        return Ok(false);
    }
    if !is_visible(candidate.owner_id, candidate.visibility.as_deref(), ctx) {
        return Ok(false);
    }
    if ctx.team_mode
        && candidate.owner_id.is_some()
        && candidate.owner_id != ctx.caller_id
        && !matches!(candidate.visibility.as_deref(), Some("shared") | Some("team"))
    {
        return Ok(false);
    }
    if matches!(frame.temporal_mode, TemporalMode::Current) || as_of_bind(ctx).is_some() {
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

pub(crate) fn normalize_query_paths(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|path| normalize_task_path(path))
        .filter(|path| !path.is_empty())
        .collect()
}

/// Read law: an unscoped row stays visible under a named project.
pub(crate) fn read_path_sets(query_paths: &[String], candidate_paths: &[String]) -> bool {
    if query_paths.is_empty() || candidate_paths.is_empty() {
        return true;
    }
    candidate_paths
        .iter()
        .any(|candidate| query_paths.iter().any(|query| path_compatible(candidate, query)))
}

/// Write identity: unscoped and path-scoped facts never Jaccard-merge.
pub(crate) fn jaccard_path_sets(incoming_paths: &[String], candidate_paths: &[String]) -> bool {
    match (incoming_paths.is_empty(), candidate_paths.is_empty()) {
        (true, true) => true,
        (false, false) => candidate_paths
            .iter()
            .any(|candidate| incoming_paths.iter().any(|incoming| path_compatible(candidate, incoming))),
        _ => false,
    }
}

pub(crate) fn explicit_paths_by_target(
    conn: &Connection,
    target_type: &str,
    ids: &[i64],
) -> Result<HashMap<i64, Vec<String>>, String> {
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let list = ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT e.target_id, a.value FROM clock_anchors a
         JOIN clock_anchor_evidence e ON e.anchor_id = a.id
         WHERE e.target_type = ?1 AND e.target_id IN ({list}) AND a.kind = 'path' AND a.specificity >= 3"
    );
    let mut stmt = conn.prepare(&sql).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![target_type], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|err| err.to_string())?;
    for (target_id, value) in rows.flatten() {
        out.entry(target_id).or_default().push(value);
    }
    Ok(out)
}

pub(crate) fn target_scope_compatible(
    conn: &Connection,
    target_type: &str,
    target_id: i64,
    query_paths: &[String],
) -> Result<bool, String> {
    let query_paths = normalize_query_paths(query_paths);
    if query_paths.is_empty() {
        return Ok(true);
    }
    let candidate_paths = explicit_path_values(conn, target_type, target_id)?;
    Ok(read_path_sets(&query_paths, &candidate_paths))
}

fn path_context_compatible(conn: &Connection, candidate: &ScoredCandidate, frame: &QueryFrame) -> Result<bool, String> {
    target_scope_compatible(conn, &candidate.target_type, candidate.target_id, &frame.paths)
}

fn explicit_path_values(conn: &Connection, target_type: &str, target_id: i64) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT a.value FROM clock_anchors a
             JOIN clock_anchor_evidence e ON e.anchor_id = a.id
             WHERE e.target_type = ?1 AND e.target_id = ?2 AND a.kind = 'path' AND a.specificity >= 3",
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
            for witness in &incoming.witnesses {
                if !existing.witnesses.contains(witness) {
                    existing.witnesses.push(witness.clone());
                }
            }
            if existing.excerpt.len() < incoming.excerpt.len() && existing.excerpt.is_empty() {
                existing.excerpt = incoming.excerpt.clone();
            }
            for anchor in &incoming.anchors {
                if !existing.anchors.iter().any(|a| a.kind == anchor.kind && a.value == anchor.value) {
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

fn resolve_source<'a>(conn: &Connection, source: &'a str) -> Option<(&'a str, i64)> {
    if let Some(id) = source.strip_prefix("memory::").and_then(|s| s.parse().ok()) {
        return Some(("memory", id));
    }
    if let Some(id) = source.strip_prefix("decision::").and_then(|s| s.parse().ok()) {
        return Some(("decision", id));
    }
    // cortex-7db determinism contract: context/source are not unique (the
    // store path inserts freely and dedupes only on decision-text similarity;
    // the indexer deliberately leaves superseded rows sharing a source), and
    // the chosen id decides which row load_target loads. The cut must be
    // data-defined, never SQLite-plan-defined: minimum id, matching the
    // hop-frontier/shared-anchor tiebreak.
    conn.query_row("SELECT id FROM decisions WHERE context = ?1 ORDER BY id ASC LIMIT 1", params![source], |row| row.get(0))
        .optional()
        .ok()
        .flatten()
        .map(|id| ("decision", id))
        .or_else(|| {
            conn.query_row("SELECT id FROM memories WHERE source = ?1 ORDER BY id ASC LIMIT 1", params![source], |row| row.get(0))
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

pub fn clock_health_payload(conn: &Connection) -> Value {
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
pub fn estimate_query_tokens(query_text: &str) -> usize {
    estimate_tokens(query_text)
}

#[allow(dead_code)]
pub fn merge_search_candidates(items: Vec<SearchCandidate>) -> BTreeMap<String, SearchCandidate> {
    let mut map = BTreeMap::new();
    for item in items {
        map.entry(item.source.clone()).or_insert(item);
    }
    map
}

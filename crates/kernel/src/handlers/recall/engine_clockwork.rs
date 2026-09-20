pub(super) use super::{
    RecallContext, build_fts_query, build_search_term_groups, is_visible, quote_fts_match_term,
    strip_fts_operators,
};
use super::{RecallItem, SearchTableKind, like_prefix, pack_budget, round4, search_source_key};
use crate::clockwork::{
    ClockEvidence, LinkHit, RankKey, Rankable, WhyAnchor, Witness, WitnessDomain,
    admit_with_lineage, expand_query_frame, parse_query_frame,
};
use crate::graph;
use crate::protocol::nonempty_opt;
use crate::traces;
use rusqlite::Connection;
use std::collections::HashMap;

/// Per-arm provenance markers. Each collector arm stamps the candidates it
/// contributes to; `scored_to_item` surfaces the merged list as
/// `clockVotes.admittedArms` in the why payload. Additive metadata only:
/// markers never influence admission, ranking, or tie-breaking.
pub(super) const ARM_LEXICAL: &str = "lexical";
pub(super) const ARM_ANCHOR: &str = "anchor";
pub(super) const ARM_TRUTH: &str = "truth";
pub(super) const ARM_TASK: &str = "task";
pub(super) const ARM_HISTORY: &str = "history";
pub(super) const ARM_HOP: &str = "hop";
pub(super) const ARM_ACTIVITY: &str = "activity";

pub(super) fn mark_arm(arms: &mut Vec<&'static str>, arm: &'static str) {
    if !arms.contains(&arm) {
        arms.push(arm);
    }
}

#[derive(Clone)]
pub(super) struct ScoredCandidate {
    pub(super) target_type: String,
    pub(super) target_id: i64,
    pub(super) source: String,
    pub(super) excerpt: String,
    pub(super) owner_id: Option<i64>,
    pub(super) visibility: Option<String>,
    pub(super) ts: i64,
    pub(super) hops: u8,
    pub(super) write: u8,
    pub(super) truth: u8,
    pub(super) task: u8,
    pub(super) history: u8,
    pub(super) hard_anchor: bool,
    pub(super) strong_lexical: bool,
    pub(super) specificity: u8,
    pub(super) fts_rank: i64,
    pub(super) use_score: i64,
    pub(super) anchors: Vec<WhyAnchor>,
    pub(super) links: Vec<LinkHit>,
    pub(super) arms: Vec<&'static str>,
    pub(super) status: String,
    pub(super) valid_from: Option<String>,
    pub(super) valid_until: Option<String>,
    pub(super) witnesses: Vec<Witness>,
    pub(super) required_role: bool,
    pub(super) contradiction: bool,
}

/// Per-route-family quotas. A global top-k across memory kinds can hide a
/// rare old constraint behind lexical noise; each family is bounded on its
/// own and reports exhaustion in the trace.
pub const ROUTE_QUOTAS: [(&str, usize); 6] = [
    ("lexical", 24),
    ("anchor", 24),
    ("truth", 16),
    ("task", 16),
    ("history", 16),
    ("hop", 16),
];

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
pub(super) fn apply_route_quotas(
    by_key: &mut HashMap<(String, i64), ScoredCandidate>,
    trace: &mut RouteTrace,
) {
    let mut keep: std::collections::HashSet<(String, i64)> = std::collections::HashSet::new();
    for (family, cap) in ROUTE_QUOTAS {
        let mut members: Vec<(&(String, i64), &ScoredCandidate)> = by_key
            .iter()
            .filter(|(_, c)| c.arms.contains(&family))
            .collect();
        trace.collected.insert(family.to_string(), members.len());
        members.sort_by(|a, b| a.1.rank_key().cmp(&b.1.rank_key()));
        if members.len() > cap {
            trace.exhausted.push(family.to_string());
        }
        for (key, _) in members.into_iter().take(cap) {
            keep.insert(key.clone());
        }
        trace
            .kept
            .insert(family.to_string(), cap.min(trace.collected[family]));
    }
    by_key.retain(|key, _| keep.contains(key));
}

impl ScoredCandidate {
    pub(super) fn origin(&self) -> String {
        format!("{}::{}", self.target_type, self.target_id)
    }
    pub(super) fn witness(
        &mut self,
        domain: WitnessDomain,
        key: impl Into<String>,
        specificity: u8,
    ) {
        // Direct evidence channels on one row (its text, its anchor
        // projections, its history) are distinct origins; a derived hop is not.
        let origin = format!("{:?}:{}", domain, self.origin()).to_ascii_lowercase();
        self.witnesses
            .push(Witness::direct(domain, origin, key, specificity));
    }
    pub(super) fn evidence(&self) -> ClockEvidence {
        ClockEvidence {
            write: self.write,
            truth: self.truth,
            task: self.task,
            history: self.history,
        }
    }

    pub(super) fn rank_key(&self) -> RankKey {
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
    conn: &Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
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
    // Query-text `as of YYYY-MM-DD` lives on the frame. FTS, load_target,
    // and row_eligible only read ctx.as_of, so inferred as-of used to
    // report valid_at while still ranking under current gates (later
    // knowledge leaked; why.filters.valid_at lied).
    let mut ctx = ctx.clone();
    if as_of_bind(&ctx).is_none() {
        ctx.as_of = nonempty_opt(frame.as_of.as_deref()).map(str::to_string);
    }
    let ctx = &ctx;
    // `source_prefix` is a provenance filter (memories.source / decision
    // identity), not a filesystem path. Pushing it onto `frame.paths` made
    // the task arm LIKE-match children of the prefix — including after FTS
    // skipped an empty/stop-word query — and treated trailing `*` as a glob.
    let source_prefix = nonempty_opt(source_prefix);
    frame.entity_ids = graph::resolve_query(conn, query_text);
    expand_query_frame(conn, &mut frame, ctx.principal.as_deref());

    let mut by_key: HashMap<(String, i64), ScoredCandidate> = HashMap::new();
    collect_write_arm(conn, &frame, query_text, source_prefix, ctx, &mut by_key)?;
    collect_anchor_arm(conn, &frame, ctx, &mut by_key)?;
    collect_truth_arm(conn, &frame, ctx, &mut by_key)?;
    collect_task_arm(conn, &frame, ctx, &mut by_key)?;
    collect_history_arm(conn, &frame, ctx, source_prefix, &mut by_key)?;
    collect_hop_arm(conn, &frame, ctx, &mut by_key)?;
    // Activity runs last: hop traversal seeds from strong candidates only,
    // and members must still be traversable-to (a member that seeds the hop
    // would self-skip at distance 0 and lose the derived line).
    collect_activity_arm(conn, &frame, ctx, &mut by_key)?;

    let mut trace = RouteTrace {
        rank_tuple: crate::clockwork::RANK_TUPLE_VERSION,
        ..RouteTrace::default()
    };
    apply_route_quotas(&mut by_key, &mut trace);
    annotate_roles(conn, &mut by_key)?;
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
    let mut items: Vec<RecallItem> = admitted
        .into_iter()
        .map(|candidate| scored_to_item(candidate, &frame, &valid_at, ctx))
        .collect();
    if token_budget > 0 {
        items = pack_budget(items, token_budget, query_text);
    }
    items.truncate(k.max(1));
    Ok(items)
}

#[path = "engine_clockwork/arms.rs"]
mod arms;
use arms::*;
#[path = "engine_clockwork/gates.rs"]
mod gates;
use gates::*;
#[path = "engine_clockwork/load.rs"]
mod load;
use load::*;
#[path = "engine_clockwork/paths.rs"]
mod paths;
use paths::path_context_compatible;
pub(crate) use paths::{
    explicit_paths_by_target, jaccard_path_sets, normalize_query_paths, read_path_sets,
};
#[path = "engine_clockwork/score.rs"]
mod score;
pub use score::clock_health_payload;
use score::*;

use super::evidence::{ClockEvidence, Witness, direct_domains, independent_support};
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankKey {
    /// Versioned rank tuple (integers only; see `RANK_TUPLE_VERSION`):
    /// required role → exact applicable (hard anchor) → independent lineage
    /// → clock count → material contradiction → strength → specificity →
    /// path cost → scoped utility → valid-time recency → canonical id.
    pub required_role: bool,
    pub hard_anchor: bool,
    pub lineage: u8,
    pub contradiction: bool,
    pub clock_count: u8,
    pub strength: u8,
    pub specificity: u8,
    pub hops: u8,
    pub fts_rank: i64,
    pub use_score: i64,
    pub recency: i64,
    pub target_type: String,
    pub target_id: i64,
}

impl RankKey {
    pub fn from_parts(
        hard_anchor: bool,
        evidence: ClockEvidence,
        specificity: u8,
        hops: u8,
        fts_rank: i64,
        use_score: i64,
        recency: i64,
        target_type: impl Into<String>,
        target_id: i64,
    ) -> Self {
        Self {
            required_role: false,
            hard_anchor,
            lineage: evidence.nonzero_count().min(1),
            contradiction: false,
            clock_count: evidence.nonzero_count(),
            strength: evidence.strength_sum(),
            specificity,
            hops,
            fts_rank,
            use_score,
            recency,
            target_type: target_type.into(),
            target_id,
        }
    }
}

impl PartialOrd for RankKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RankKey {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_rank_keys(self, other)
    }
}

pub const RANK_TUPLE_VERSION: &str = "rank/2";

pub fn compare_rank_keys(a: &RankKey, b: &RankKey) -> Ordering {
    b.required_role
        .cmp(&a.required_role)
        .then_with(|| b.hard_anchor.cmp(&a.hard_anchor))
        .then_with(|| b.lineage.cmp(&a.lineage))
        .then_with(|| b.clock_count.cmp(&a.clock_count))
        .then_with(|| b.contradiction.cmp(&a.contradiction))
        .then_with(|| b.strength.cmp(&a.strength))
        .then_with(|| b.specificity.cmp(&a.specificity))
        .then_with(|| a.hops.cmp(&b.hops))
        .then_with(|| b.fts_rank.cmp(&a.fts_rank))
        .then_with(|| b.use_score.cmp(&a.use_score))
        .then_with(|| b.recency.cmp(&a.recency))
        .then_with(|| a.target_type.cmp(&b.target_type))
        .then_with(|| a.target_id.cmp(&b.target_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rankable {
    pub eligible: bool,
    pub hard_anchor: bool,
    pub evidence: ClockEvidence,
    pub strong_lexical: bool,
}

/// Legacy numeric admission (counters only). Retained for callers that have
/// no witness lineage; the engine uses `admit_with_lineage`.
pub fn admit(item: Rankable) -> Option<&'static str> {
    if !item.eligible {
        return None;
    }
    if item.hard_anchor {
        return Some("hard_anchor");
    }
    if item.evidence.nonzero_count() >= 2 {
        return Some("clock_quorum");
    }
    if item.strong_lexical && item.evidence.write >= 2 {
        return Some("strong_lexical");
    }
    None
}

/// Lineage-aware admission law:
/// 1. ineligible (policy/scope/validity) → not exposed;
/// 2. a *direct* hard anchor → `hard_anchor`;
/// 3. a *direct* strong lexical match → `strong_lexical`;
/// 4. two witness domains with independent origin groups → `clock_quorum`
///    (a derived route counts once, through its seed's origin; shared
///    ancestry is one family);
/// 5. otherwise not in the supported set. Derived-only candidates are leads.
pub fn admit_with_lineage(item: Rankable, witnesses: &[Witness]) -> Option<&'static str> {
    if !item.eligible {
        return None;
    }
    let has_direct_hard =
        item.hard_anchor && witnesses.iter().any(|w| !w.derived && w.specificity >= 3);
    if has_direct_hard {
        return Some("hard_anchor");
    }
    let direct = direct_domains(witnesses);
    let origins = independent_support(witnesses);
    // Two domains AND two origin groups. A hop-only row has one origin
    // (its seed) and zero direct domains; a row matched lexically and via a
    // hop from an unrelated seed has one direct domain plus one derived
    // family from a distinct origin.
    let domains_incl_derived = {
        let mut kinds: Vec<String> = witnesses
            .iter()
            .map(|w| format!("{:?}", w.domain))
            .collect();
        kinds.sort();
        kinds.dedup();
        kinds.len()
    };
    if direct >= 1 && domains_incl_derived >= 2 && origins >= 2 {
        return Some("clock_quorum");
    }
    if item.strong_lexical && item.evidence.write >= 2 && witnesses.iter().any(|w| !w.derived) {
        return Some("strong_lexical");
    }
    None
}

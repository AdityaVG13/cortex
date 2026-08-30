use super::evidence::ClockEvidence;
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankKey {
    pub hard_anchor: bool,
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
            hard_anchor,
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

pub fn compare_rank_keys(a: &RankKey, b: &RankKey) -> Ordering {
    b.hard_anchor
        .cmp(&a.hard_anchor)
        .then_with(|| b.clock_count.cmp(&a.clock_count))
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

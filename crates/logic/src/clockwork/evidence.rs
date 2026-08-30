use super::anchors::AnchorKind;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ClockEvidence {
    pub write: u8,
    pub truth: u8,
    pub task: u8,
    pub history: u8,
}

impl ClockEvidence {
    pub fn nonzero_count(self) -> u8 {
        u8::from(self.write > 0)
            + u8::from(self.truth > 0)
            + u8::from(self.task > 0)
            + u8::from(self.history > 0)
    }

    pub fn strength_sum(self) -> u8 {
        self.write
            .saturating_add(self.truth)
            .saturating_add(self.task)
            .saturating_add(self.history)
    }

    pub fn contradicting_history(self, write_ok: bool) -> bool {
        !write_ok && self.history == 0 && self.write == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhyAnchor {
    pub kind: AnchorKind,
    pub value: String,
    pub specificity: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkHit {
    pub relation: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterEvidence {
    pub acl: String,
    pub head: Option<i64>,
    pub valid_at: String,
    pub status_filters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TieBreak {
    pub clock_count: u8,
    pub strength: u8,
    pub hops: u8,
    pub specificity: u8,
    pub fts_rank: i64,
    pub use_score: i64,
    pub recency: i64,
    pub target_type: String,
    pub target_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockWhy {
    pub engine: &'static str,
    pub admitted_by: String,
    pub hard_anchor: bool,
    pub clock_votes: ClockEvidence,
    pub anchors: Vec<WhyAnchor>,
    pub links: Vec<LinkHit>,
    pub filters: FilterEvidence,
    pub tie_break: TieBreak,
}

impl ClockWhy {
    pub fn new(
        admitted_by: String,
        hard_anchor: bool,
        clock_votes: ClockEvidence,
        anchors: Vec<WhyAnchor>,
        links: Vec<LinkHit>,
        filters: FilterEvidence,
        tie_break: TieBreak,
    ) -> Self {
        Self {
            engine: "clock-quorum",
            admitted_by,
            hard_anchor,
            clock_votes,
            anchors,
            links,
            filters,
            tie_break,
        }
    }
}

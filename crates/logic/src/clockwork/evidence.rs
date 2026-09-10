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
    /// Provenance-bearing witnesses behind the admission.
    #[serde(default)]
    pub witnesses: Vec<Witness>,
    /// The four questions, answered separately: where it was found, why it
    /// applies, what it establishes, and what was searched.
    #[serde(default)]
    pub questions: Questions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Questions {
    /// Discovery: collector arms that surfaced the row.
    pub discovery: Vec<String>,
    /// Relevance: the admission law that applied and its independent support.
    pub relevance: String,
    pub independent_origins: usize,
    /// Epistemic status of the row itself (never inferred from relevance).
    pub epistemic: String,
    /// Coverage: the gates and validity perspective the answer was read under.
    pub coverage: String,
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
            witnesses: Vec::new(),
            questions: Questions::default(),
        }
    }
    pub fn with_lineage(mut self, witnesses: Vec<Witness>, arms: &[&str], status: &str) -> Self {
        let independent = independent_support(&witnesses);
        self.questions = Questions {
            discovery: arms.iter().map(|a| a.to_string()).collect(),
            relevance: self.admitted_by.clone(),
            independent_origins: independent,
            epistemic: match status {
                "archived" | "superseded" => "retracted".into(),
                "disputed" => "contested".into(),
                _ => "asserted".into(),
            },
            coverage: format!(
                "acl={} validAt={} head={:?}",
                self.filters.acl, self.filters.valid_at, self.filters.head
            ),
        };
        self.witnesses = witnesses;
        self
    }
}

/// Where a candidate witness came from. Direct witnesses match the query on
/// their own row; derived witnesses were reached through another row (a
/// graph hop, a composite member) and inherit that row's origin group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WitnessDomain {
    Lexical,
    Anchor,
    Entity,
    Task,
    History,
    Hop,
}

/// A provenance-bearing witness. `origin` identifies the evidentiary origin
/// group: for a direct witness the (domain, row) channel that produced the
/// match; for a derived witness the traversal/seed family it was reached
/// through. Two witnesses with the same origin are one lineage family and
/// never count twice, however many counters a traversal touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Witness {
    pub domain: WitnessDomain,
    pub derived: bool,
    pub origin: String,
    pub matched_key: String,
    pub path_length: u8,
    pub specificity: u8,
}

impl Witness {
    pub fn direct(
        domain: WitnessDomain,
        origin: impl Into<String>,
        matched_key: impl Into<String>,
        specificity: u8,
    ) -> Self {
        Self {
            domain,
            derived: false,
            origin: origin.into(),
            matched_key: matched_key.into(),
            path_length: 0,
            specificity,
        }
    }
    pub fn derived(
        domain: WitnessDomain,
        origin: impl Into<String>,
        matched_key: impl Into<String>,
        path_length: u8,
    ) -> Self {
        Self {
            domain,
            derived: true,
            origin: origin.into(),
            matched_key: matched_key.into(),
            path_length,
            specificity: 1,
        }
    }
}

/// Independent support = number of distinct origin groups among the
/// witnesses, where every derived witness contributes its *seed's* origin.
/// A single graph traversal that produced several derived witnesses therefore
/// counts once, however many counters it touched.
pub fn independent_support(witnesses: &[Witness]) -> usize {
    let mut origins: Vec<&str> = witnesses.iter().map(|w| w.origin.as_str()).collect();
    origins.sort_unstable();
    origins.dedup();
    origins.len()
}

pub fn direct_domains(witnesses: &[Witness]) -> usize {
    let mut domains: Vec<&WitnessDomain> = witnesses
        .iter()
        .filter(|w| !w.derived)
        .map(|w| &w.domain)
        .collect();
    domains.sort_by_key(|d| format!("{d:?}"));
    domains.dedup();
    domains.len()
}

use super::ids::{Frontier, LogicalId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Durability assumptions the acknowledgement was made under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AckProfile {
    /// Survives process death; not power loss (synchronous=NORMAL class).
    ProcessCrash,
    /// Platform-declared power-loss durability (synchronous=FULL class).
    PowerLossAssumed { platform_profile: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadAvailability {
    Retained,
    ExternalOnly,
    Pending,
    Unavailable,
}

/// Durability is a vector, not a boolean. Received is not committed;
/// committed is not projected; projected is not replicated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurabilityVector {
    /// The bounded request passed initial validation.
    pub accepted: bool,
    /// Frontier the local commit reached, if it committed.
    pub local_commit: Option<Frontier>,
    /// Per-projection frontier each index/materializer covers.
    #[serde(default)]
    pub projected_through: BTreeMap<String, Frontier>,
    /// Optional remote copies/quorum with the policy name that defines them.
    #[serde(default)]
    pub replicated_through: BTreeMap<String, Frontier>,
    pub payload_availability: PayloadAvailability,
    pub ack_profile: AckProfile,
}

/// What happened to one captured source. Silence never masquerades as
/// success: every capture reports one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus {
    Accepted,
    Redacted,
    Excluded,
    Rejected,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureReceipt {
    pub status: CaptureStatus,
    /// Retained representation: bytes actually stored (after redaction /
    /// truncation), not the bytes offered.
    pub retained_bytes: usize,
    pub offered_bytes: usize,
    /// Named policy that shaped the retained representation.
    pub policy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Result of a Deposit or any state-changing operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub receipt_id: LogicalId,
    pub request_id: String,
    pub durability: DurabilityVector,
    /// Client-local entry names → canonical ids assigned by the runtime.
    #[serde(default)]
    pub entries: BTreeMap<String, LogicalId>,
    /// Presentation aliases (`m1`) bound to this receipt and epoch.
    #[serde(default)]
    pub aliases: BTreeMap<String, LogicalId>,
    /// Things the runtime chose not to deliver, with reasons.
    #[serde(default)]
    pub omissions: Vec<String>,
    /// Needs declared by the caller that the response did not cover.
    #[serde(default)]
    pub unresolved_needs: Vec<String>,
}

impl Receipt {
    /// True only when a local commit frontier exists. `accepted` alone is
    /// never treated as durable.
    pub fn is_locally_durable(&self) -> bool {
        self.durability.local_commit.is_some()
    }
}

//! Four state variables that are never the same thing:
//! - brain frontier: which durable changes exist (`protocol::Frontier`);
//! - delivery acknowledgement: a client received a View;
//! - context presence: which exact representations the *current* invocation
//!   holds, attested by the host;
//! - Thread checkpoint: durable work state.
//!
//! Required content may be suppressed only when the exact representation is
//! attested present under matching brain, policy and context epochs. A prior
//! delivery, a session id, a prefix cache or a short alias never suffices.

use crate::protocol::{ContextPresence, Frontier, LogicalId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryAck {
    pub receipt_id: String,
    pub delivered_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadCheckpoint {
    pub thread: LogicalId,
    pub revision: LogicalId,
    pub frontier: Frontier,
}

/// Epochs the runtime is answering under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentEpochs {
    pub brain_epoch: String,
    pub policy_epoch: String,
}

/// Why a representation must still be delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceDecision {
    /// Exact representation attested present under matching epochs: the
    /// transport may omit the payload (this is a transport saving only).
    Suppress,
    /// No attestation for this invocation: deliver self-contained.
    DeliverUnknownPresence,
    /// Attestation exists but for another context epoch (compaction, new
    /// process): deliver.
    DeliverContextEpochChanged,
    /// Brain or policy epoch moved since the attestation: deliver.
    DeliverEpochChanged,
    /// The attested representation is a different revision or rendering.
    DeliverRepresentationDiffers,
}

impl PresenceDecision {
    pub fn suppresses(self) -> bool {
        matches!(self, Self::Suppress)
    }
}

/// Decide for one (revision, representation) pair. Exhaustive over the
/// presence inputs; every non-exact case delivers.
pub fn decide(
    presence: Option<&ContextPresence>,
    attested_brain_epoch: Option<&str>,
    attested_policy_epoch: Option<&str>,
    current: &CurrentEpochs,
    current_context_epoch: &str,
    revision: &LogicalId,
    representation: &str,
) -> PresenceDecision {
    let Some(presence) = presence else {
        return PresenceDecision::DeliverUnknownPresence;
    };
    if presence.context_epoch != current_context_epoch {
        return PresenceDecision::DeliverContextEpochChanged;
    }
    if attested_brain_epoch != Some(current.brain_epoch.as_str())
        || attested_policy_epoch != Some(current.policy_epoch.as_str())
    {
        return PresenceDecision::DeliverEpochChanged;
    }
    let exact = presence
        .present
        .iter()
        .any(|p| &p.revision == revision && p.representation == representation);
    if exact {
        PresenceDecision::Suppress
    } else {
        PresenceDecision::DeliverRepresentationDiffers
    }
}

/// A change cursor names a durable change boundary *and* the identity of the
/// filter it was taken under. Asking "since this cursor" in a wider scope, a
/// different restore epoch or a different rule version requires a new
/// baseline: `resnapshot_required`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeCursor {
    pub restore_epoch: String,
    pub scope_filter: String,
    pub sequence: i64,
    pub rule_version: String,
}

pub const CHANGE_RULE_VERSION: &str = "changes/1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorError {
    Malformed,
    ResnapshotRequired { reason: &'static str },
}

impl ChangeCursor {
    pub fn encode(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.restore_epoch, self.scope_filter, self.sequence, self.rule_version
        )
    }
    pub fn decode(raw: &str) -> Result<Self, CursorError> {
        let parts: Vec<&str> = raw.split('|').collect();
        if parts.len() != 4 {
            return Err(CursorError::Malformed);
        }
        let sequence = parts[2].parse().map_err(|_| CursorError::Malformed)?;
        Ok(Self {
            restore_epoch: parts[0].to_string(),
            scope_filter: parts[1].to_string(),
            sequence,
            rule_version: parts[3].to_string(),
        })
    }
    /// Validate against the current epoch, requested filter and rule version.
    pub fn validate(&self, restore_epoch: &str, scope_filter: &str) -> Result<i64, CursorError> {
        if self.restore_epoch != restore_epoch {
            return Err(CursorError::ResnapshotRequired {
                reason: "restore epoch changed",
            });
        }
        if self.rule_version != CHANGE_RULE_VERSION {
            return Err(CursorError::ResnapshotRequired {
                reason: "change rule version changed",
            });
        }
        if self.scope_filter != scope_filter {
            return Err(CursorError::ResnapshotRequired {
                reason: "scope/filter identity differs; a wider scope needs a new baseline",
            });
        }
        Ok(self.sequence)
    }
}

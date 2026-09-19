//! Exact observation intake. Source registration is an operator/library action,
//! never inferred from event text. A source cursor and all its accepted
//! occurrences commit together. This module does not interpret prose or learn.
use serde::{Deserialize, Serialize};

pub const MAX_CAPTURE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BATCH_EVENTS: usize = 128;
const DDL: &str = "CREATE TABLE IF NOT EXISTS observation_sources (principal TEXT NOT NULL, source_key TEXT NOT NULL, scope_id TEXT NOT NULL REFERENCES scopes(scope_id), scope_label TEXT NOT NULL, role TEXT NOT NULL, max_bytes INTEGER NOT NULL CHECK(max_bytes>0), enabled INTEGER NOT NULL CHECK(enabled IN (0,1)), policy_epoch TEXT NOT NULL, PRIMARY KEY(principal,source_key)); CREATE TABLE IF NOT EXISTS observation_events (principal TEXT NOT NULL, source_key TEXT NOT NULL, generation TEXT NOT NULL, event_key TEXT NOT NULL, source_id TEXT NOT NULL UNIQUE REFERENCES sources(source_id), revision_id TEXT NOT NULL REFERENCES revisions(revision_id), receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)), PRIMARY KEY(principal,source_key,generation,event_key), FOREIGN KEY(principal,source_key) REFERENCES observation_sources(principal,source_key)); CREATE TABLE IF NOT EXISTS observation_cursors (principal TEXT NOT NULL, source_key TEXT NOT NULL, generation TEXT NOT NULL, byte_offset INTEGER NOT NULL CHECK(byte_offset>=0), PRIMARY KEY(principal,source_key,generation), FOREIGN KEY(principal,source_key) REFERENCES observation_sources(principal,source_key));";

pub const EVENT_GRANT_JOIN: &str = "observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key";
pub const LIVE_HEAD_SQL: &str = "EXISTS(SELECT 1 FROM record_heads h WHERE h.revision_id=e.revision_id) AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id)";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationRole {
    Document,
    UserStatement,
    AgentAssertion,
    ToolReport,
    DeliveryOnly,
}
impl ObservationRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::UserStatement => "user_statement",
            Self::AgentAssertion => "agent_assertion",
            Self::ToolReport => "tool_report",
            Self::DeliveryOnly => "delivery_only",
        }
    }
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim() {
            "document" => Self::Document,
            "user_statement" => Self::UserStatement,
            "agent_assertion" => Self::AgentAssertion,
            "tool_report" => Self::ToolReport,
            "delivery_only" => Self::DeliveryOnly,
            _ => return None,
        })
    }
    pub fn may_cite(self) -> bool {
        self != Self::DeliveryOnly
    }
}

#[derive(Debug, Clone)]
pub struct SourceSpec {
    pub key: String,
    pub scope: String,
    pub role: ObservationRole,
    pub max_bytes: usize,
}
impl SourceSpec {
    pub fn document(key: impl Into<String>, scope: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            scope: scope.into(),
            role: ObservationRole::Document,
            max_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ObservationEvent {
    pub event_key: String,
    pub text: String,
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationReceipt {
    pub source_id: String,
    pub record_id: String,
    pub revision_id: String,
    pub sequence: i64,
    pub restore_epoch: String,
    pub ack_profile: crate::protocol::AckProfile,
    pub retained_bytes: usize,
    pub duplicate: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedObservation {
    pub source_id: String,
    pub source_key: String,
    pub scope_id: String,
    pub generation: String,
    pub role: String,
    pub event_key: String,
    pub text: String,
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailReceipt {
    pub accepted: Vec<ObservationReceipt>,
    pub next_offset: u64,
    pub uncommitted_tail_bytes: usize,
}

pub(super) struct GrantedSource {
    pub(super) scope_id: String,
    pub(super) role: String,
    max_bytes: usize,
}

pub(super) fn check_label(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 1024 {
        return Err("invalid_source_identity".into());
    }
    Ok(())
}

pub(crate) use crate::handlers::looks_like_fs_path as scope_is_path;

mod scope;
pub use scope::normalize_scope;
pub(crate) use scope::{cite_scope_allowed, resolve_query_scopes, scopes_compatible};

mod grant;
pub(in crate::runtime) use grant::*;
mod write;
pub(in crate::runtime) use write::capture;
pub(crate) use write::{capture_registered, source_capture_limit};
mod ops;

//! Exact assemblies over existing revision rows, plus an attributed learning
//! ledger and a rebuildable cue route. Ranking never changes epistemic status.
use cortex_logic::assembly::MembershipRole;
use serde::{Deserialize, Serialize};
use serde_json::Value;
mod helpers;
mod runtime;
pub(crate) use helpers::suggest_authorized_members;
pub use helpers::tokenize_cues;
use helpers::*;

const DDL: &str = "CREATE TABLE IF NOT EXISTS assemblies (assembly_id TEXT NOT NULL, principal TEXT NOT NULL, scope_label TEXT NOT NULL, kind TEXT NOT NULL, created_sequence INTEGER NOT NULL REFERENCES commits(sequence), PRIMARY KEY(principal,assembly_id)); CREATE TABLE IF NOT EXISTS assembly_revisions (assembly_revision_id TEXT PRIMARY KEY, principal TEXT NOT NULL, assembly_id TEXT NOT NULL, codec TEXT NOT NULL, envelope_json TEXT NOT NULL CHECK(json_valid(envelope_json)), recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence), FOREIGN KEY(principal,assembly_id) REFERENCES assemblies(principal,assembly_id)); CREATE TABLE IF NOT EXISTS assembly_members (assembly_revision_id TEXT NOT NULL REFERENCES assembly_revisions(assembly_revision_id), member_revision_id TEXT NOT NULL REFERENCES revisions(revision_id), ordinal INTEGER NOT NULL CHECK(ordinal>=0), role TEXT NOT NULL, PRIMARY KEY(assembly_revision_id,ordinal)); CREATE TABLE IF NOT EXISTS assembly_guards (assembly_revision_id TEXT NOT NULL REFERENCES assembly_revisions(assembly_revision_id), guard_kind TEXT NOT NULL, guard_key TEXT NOT NULL, guard_epoch TEXT NOT NULL, PRIMARY KEY(assembly_revision_id,guard_kind,guard_key)); CREATE TABLE IF NOT EXISTS learning_events (origin TEXT NOT NULL, origin_event_id TEXT NOT NULL, principal TEXT NOT NULL, scope_label TEXT NOT NULL, training_unit TEXT NOT NULL, target TEXT NOT NULL, kind TEXT NOT NULL, reward INTEGER NOT NULL CHECK(reward IN (-1,0,1)), cues_json TEXT NOT NULL CHECK(json_valid(cues_json)), observed_at INTEGER NOT NULL CHECK(observed_at>=0), receipt_ref TEXT NOT NULL, recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence), PRIMARY KEY(origin,origin_event_id)); CREATE TABLE IF NOT EXISTS learning_dependencies (origin TEXT NOT NULL, origin_event_id TEXT NOT NULL, source_id TEXT NOT NULL, PRIMARY KEY(origin,origin_event_id,source_id), FOREIGN KEY(origin,origin_event_id) REFERENCES learning_events(origin,origin_event_id)); CREATE TABLE IF NOT EXISTS learning_retractions (origin TEXT NOT NULL, origin_event_id TEXT NOT NULL, recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence), reason TEXT NOT NULL, PRIMARY KEY(origin,origin_event_id)); CREATE TABLE IF NOT EXISTS learning_source_erasures (source_id TEXT PRIMARY KEY, erasure_epoch TEXT NOT NULL, recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence)); CREATE TABLE IF NOT EXISTS assembly_route_state (principal TEXT NOT NULL, scope_label TEXT NOT NULL, enabled INTEGER NOT NULL, PRIMARY KEY(principal,scope_label)); CREATE TABLE IF NOT EXISTS assembly_route_edges (principal TEXT NOT NULL, scope_label TEXT NOT NULL, cue TEXT NOT NULL, assembly_id TEXT NOT NULL, positive REAL NOT NULL, negative REAL NOT NULL, PRIMARY KEY(principal,scope_label,cue,assembly_id));";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssemblyMemberSpec {
    pub revision_id: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssemblyGuardSpec {
    pub kind: String,
    pub key: String,
    pub epoch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssemblySpec {
    pub id: String,
    pub scope: String,
    pub kind: String,
    pub members: Vec<AssemblyMemberSpec>,
    #[serde(default)]
    pub guards: Vec<AssemblyGuardSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredAssembly {
    pub id: String,
    pub revision_id: String,
    pub principal: String,
    pub scope: String,
    pub kind: String,
    pub codec: String,
    pub members: Vec<AssemblyMemberSpec>,
    pub guards: Vec<AssemblyGuardSpec>,
    pub envelope: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteExplanation {
    pub assembly_id: String,
    pub utility: f64,
    pub mass: f64,
    pub cues: Vec<String>,
    pub training_units: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssemblyMemberView {
    pub role: String,
    pub revision_id: String,
    pub expand: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssemblyBundle {
    pub id: String,
    pub revision_id: String,
    pub kind: String,
    pub status: String,
    pub route: String,
    pub utility: f64,
    pub mass: f64,
    pub cues: Vec<String>,
    pub training_units: Vec<String>,
    pub members: Vec<AssemblyMemberView>,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AssemblyCompilation {
    pub status: String,
    pub scope: String,
    pub bundles: Vec<AssemblyBundle>,
    pub brief: String,
}

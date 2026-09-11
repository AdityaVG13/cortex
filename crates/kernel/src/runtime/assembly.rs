//! Exact assemblies over existing revision rows, plus an attributed learning
//! ledger and a rebuildable cue route. Ranking never changes epistemic status.
use super::CortexRuntime;
use crate::db::records;
use asupersync::Cx;
use cortex_logic::assembly::{
    expand_records, factor_records, rank_routes, route_edges, FactorCodec, LearningEvent,
    LearningKind, MembershipRole, RouteEdge,
};
use cortex_logic::presence::{decide, CurrentEpochs};
use cortex_logic::protocol::{ContextPresence, LogicalId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS assemblies (
 assembly_id TEXT NOT NULL, principal TEXT NOT NULL, scope_label TEXT NOT NULL,
 kind TEXT NOT NULL, created_sequence INTEGER NOT NULL REFERENCES commits(sequence),
 PRIMARY KEY(principal,assembly_id));
CREATE TABLE IF NOT EXISTS assembly_revisions (
 assembly_revision_id TEXT PRIMARY KEY,
 principal TEXT NOT NULL, assembly_id TEXT NOT NULL, codec TEXT NOT NULL,
 envelope_json TEXT NOT NULL CHECK(json_valid(envelope_json)),
 recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence),
 FOREIGN KEY(principal,assembly_id) REFERENCES assemblies(principal,assembly_id));
CREATE TABLE IF NOT EXISTS assembly_members (
 assembly_revision_id TEXT NOT NULL REFERENCES assembly_revisions(assembly_revision_id),
 member_revision_id TEXT NOT NULL REFERENCES revisions(revision_id),
 ordinal INTEGER NOT NULL CHECK(ordinal>=0),
 role TEXT NOT NULL,
 PRIMARY KEY(assembly_revision_id,ordinal));
CREATE TABLE IF NOT EXISTS assembly_guards (
 assembly_revision_id TEXT NOT NULL REFERENCES assembly_revisions(assembly_revision_id),
 guard_kind TEXT NOT NULL, guard_key TEXT NOT NULL, guard_epoch TEXT NOT NULL,
 PRIMARY KEY(assembly_revision_id,guard_kind,guard_key));
CREATE TABLE IF NOT EXISTS learning_events (
 origin TEXT NOT NULL, origin_event_id TEXT NOT NULL, principal TEXT NOT NULL,
 scope_label TEXT NOT NULL, training_unit TEXT NOT NULL, target TEXT NOT NULL,
 kind TEXT NOT NULL, reward INTEGER NOT NULL CHECK(reward IN (-1,0,1)),
 cues_json TEXT NOT NULL CHECK(json_valid(cues_json)),
 observed_at INTEGER NOT NULL CHECK(observed_at>=0),
 receipt_ref TEXT NOT NULL, recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence),
 PRIMARY KEY(origin,origin_event_id));
CREATE TABLE IF NOT EXISTS learning_dependencies (
 origin TEXT NOT NULL, origin_event_id TEXT NOT NULL, source_id TEXT NOT NULL,
 PRIMARY KEY(origin,origin_event_id,source_id),
 FOREIGN KEY(origin,origin_event_id) REFERENCES learning_events(origin,origin_event_id));
CREATE TABLE IF NOT EXISTS learning_retractions (
 origin TEXT NOT NULL, origin_event_id TEXT NOT NULL,
 recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence), reason TEXT NOT NULL,
 PRIMARY KEY(origin,origin_event_id));
CREATE TABLE IF NOT EXISTS learning_source_erasures (
 source_id TEXT PRIMARY KEY, erasure_epoch TEXT NOT NULL,
 recorded_sequence INTEGER NOT NULL REFERENCES commits(sequence));
CREATE TABLE IF NOT EXISTS assembly_route_state (
 principal TEXT NOT NULL, scope_label TEXT NOT NULL, enabled INTEGER NOT NULL,
 PRIMARY KEY(principal,scope_label));
CREATE TABLE IF NOT EXISTS assembly_route_edges (
 principal TEXT NOT NULL, scope_label TEXT NOT NULL, cue TEXT NOT NULL,
 assembly_id TEXT NOT NULL, positive REAL NOT NULL, negative REAL NOT NULL,
 PRIMARY KEY(principal,scope_label,cue,assembly_id));
"#;

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

fn ensure(conn: &Connection) -> Result<(), String> {
    records::ensure_authoritative_schema(conn).map_err(|err| err.to_string())?;
    conn.execute_batch(DDL).map_err(|err| err.to_string())
}

fn check_id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 1024 {
        return Err("invalid_assembly_identity".into());
    }
    Ok(())
}

fn current_revision(conn: &Connection, principal: &str, id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT assembly_revision_id FROM assembly_revisions WHERE principal=?1 AND assembly_id=?2 ORDER BY recorded_sequence DESC LIMIT 1",
        params![principal, id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|err| err.to_string())
}

fn load_member_bodies(conn: &Connection, members: &[AssemblyMemberSpec]) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    for member in members {
        check_id(&member.revision_id)?;
        let body = records::revision_body(conn, &member.revision_id)
            .map_err(|err| err.to_string())?
            .ok_or("member_revision_missing")?;
        rows.push(body);
    }
    Ok(rows)
}

fn live_events(conn: &Connection, principal: &str, scope: &str) -> Result<Vec<LearningEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT e.origin,e.origin_event_id,e.principal,e.scope_label,e.training_unit,e.target,e.kind,e.reward,e.cues_json,e.observed_at,e.receipt_ref
             FROM learning_events e
             WHERE e.principal=?1 AND e.scope_label=?2
             AND NOT EXISTS(SELECT 1 FROM learning_retractions r WHERE r.origin=e.origin AND r.origin_event_id=e.origin_event_id)
             AND NOT EXISTS(SELECT 1 FROM learning_dependencies d JOIN learning_source_erasures x ON x.source_id=d.source_id WHERE d.origin=e.origin AND d.origin_event_id=e.origin_event_id)",
        )
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![principal, scope], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, String>(10)?,
            ))
        })
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    drop(stmt);
    let mut events = Vec::new();
    for (origin, origin_event_id, principal, scope, training_unit, target, kind, reward, cues_json, observed_at, receipt_ref) in
        rows
    {
        let cues: Vec<String> = serde_json::from_str(&cues_json).map_err(|err| err.to_string())?;
        let mut source_stmt = conn
            .prepare("SELECT source_id FROM learning_dependencies WHERE origin=?1 AND origin_event_id=?2 ORDER BY source_id")
            .map_err(|err| err.to_string())?;
        let sources = source_stmt
            .query_map(params![origin, origin_event_id], |row| row.get(0))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|err| err.to_string())?;
        drop(source_stmt);
        events.push(LearningEvent {
            origin,
            origin_event_id,
            principal,
            scope,
            training_unit,
            target,
            kind: LearningKind::parse(&kind)?,
            reward: i8::try_from(reward).map_err(|_| "invalid_feedback_reward".to_string())?,
            cues,
            sources,
            observed_at,
            receipt_ref,
        });
    }
    Ok(events)
}

pub fn tokenize_cues(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn preview_text(body: &Value) -> String {
    body.get("text")
        .and_then(Value::as_str)
        .or_else(|| body.pointer("/observation/text").and_then(Value::as_str))
        .map(|text| text.chars().take(200).collect())
        .filter(|text: &String| !text.is_empty())
        .unwrap_or_else(|| canonical_preview(body))
}

fn canonical_preview(body: &Value) -> String {
    let raw = body.to_string();
    raw.chars().take(200).collect()
}

fn member_view(
    conn: &Connection,
    principal: &str,
    member: &AssemblyMemberSpec,
) -> Result<Option<AssemblyMemberView>, String> {
    let required = member.role.is_required_exception();
    let observation: Option<(String, Vec<u8>)> = conn
        .query_row(
            "SELECT e.source_id,s.inline_payload FROM observation_events e
             JOIN sources s ON s.source_id=e.source_id
             WHERE e.principal=?1 AND e.revision_id=?2 AND s.availability='owned_inline'
             AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id)",
            params![principal, member.revision_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|err| err.to_string())?;
    if let Some((source_id, bytes)) = observation {
        let preview = String::from_utf8(bytes)
            .map_err(|_| "source_not_utf8".to_string())?
            .chars()
            .take(200)
            .collect();
        return Ok(Some(AssemblyMemberView {
            role: member.role.as_str().into(),
            revision_id: member.revision_id.clone(),
            expand: format!("obs:{source_id}"),
            preview: Some(preview),
            required,
        }));
    }
    let retracted_observation: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM observation_events e JOIN observation_retractions t ON t.source_id=e.source_id WHERE e.principal=?1 AND e.revision_id=?2",
            params![principal, member.revision_id],
            |row| row.get(0),
        )
        .map_err(|err| err.to_string())?;
    if retracted_observation > 0 {
        return if required {
            Ok(None)
        } else {
            Ok(Some(AssemblyMemberView {
                role: member.role.as_str().into(),
                revision_id: member.revision_id.clone(),
                expand: format!("rev:{}", member.revision_id),
                preview: None,
                required,
            }))
        };
    }
    let Some(body) = records::revision_body(conn, &member.revision_id).map_err(|err| err.to_string())? else {
        return if required { Ok(None) } else {
            Ok(Some(AssemblyMemberView {
                role: member.role.as_str().into(),
                revision_id: member.revision_id.clone(),
                expand: format!("rev:{}", member.revision_id),
                preview: None,
                required,
            }))
        };
    };
    Ok(Some(AssemblyMemberView {
        role: member.role.as_str().into(),
        revision_id: member.revision_id.clone(),
        expand: format!("rev:{}", member.revision_id),
        preview: Some(preview_text(&body)),
        required,
    }))
}

fn render_assembly_brief(bundles: &[AssemblyBundle]) -> String {
    let mut lines = Vec::new();
    for bundle in bundles.iter().filter(|bundle| bundle.status == "ready") {
        if bundle.present {
            lines.push(format!(
                "Assembly {} (already present). Expand asm:{}",
                bundle.id, bundle.id
            ));
            continue;
        }
        lines.push(format!("Assembly {}:", bundle.id));
        for member in &bundle.members {
            let tag = match member.role.as_str() {
                "exception" | "contradiction" => "Exception",
                "qualification" => "Qualification",
                "support" => "Support",
                _ => "Observation",
            };
            if let Some(preview) = &member.preview {
                lines.push(format!("  {tag}: {preview}"));
            }
        }
        lines.push(format!("  Expand: asm:{}", bundle.id));
    }
    lines.join("\n")
}

fn compile_for_cues(
    conn: &Connection,
    principal: &str,
    scope: &str,
    cues: &[String],
    limit: usize,
    presence: Option<&ContextPresence>,
    attested_brain: Option<&str>,
    attested_policy: Option<&str>,
    current: &CurrentEpochs,
    context_epoch: &str,
) -> Result<AssemblyCompilation, String> {
    ensure(conn)?;
    if !routes_enabled(conn, principal, scope)? {
        return Ok(AssemblyCompilation {
            status: "disabled".into(),
            scope: scope.into(),
            bundles: Vec::new(),
            brief: String::new(),
        });
    }
    let explanations = {
        // Rank from the same live edges as explain, without a second lock.
        let mut stmt = conn
            .prepare("SELECT cue,assembly_id,positive,negative FROM assembly_route_edges WHERE principal=?1 AND scope_label=?2")
            .map_err(|err| err.to_string())?;
        let edges = stmt
            .query_map(params![principal, scope], |row| {
                Ok(RouteEdge {
                    cue: row.get(0)?,
                    target: row.get(1)?,
                    positive: row.get(2)?,
                    negative: row.get(3)?,
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        drop(stmt);
        let mut allowed_stmt = conn
            .prepare("SELECT assembly_id FROM assemblies WHERE principal=?1 AND scope_label=?2")
            .map_err(|err| err.to_string())?;
        let allowed = allowed_stmt
            .query_map(params![principal, scope], |row| row.get(0))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|err| err.to_string())?;
        drop(allowed_stmt);
        let ranked = rank_routes(&edges, cues, &allowed, 0.5);
        let events = live_events(conn, principal, scope)?;
        ranked
            .into_iter()
            .take(limit.min(8))
            .filter(|score| score.utility > 0.0)
            .map(|score| {
                let used: Vec<String> = events
                    .iter()
                    .filter(|event| event.target == score.target && event.cues.iter().any(|cue| cues.contains(cue)))
                    .map(|event| event.training_unit.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let matched = cues
                    .iter()
                    .filter(|cue| edges.iter().any(|edge| edge.cue == **cue && edge.target == score.target))
                    .cloned()
                    .collect();
                RouteExplanation {
                    assembly_id: score.target,
                    utility: score.utility,
                    mass: score.mass,
                    cues: matched,
                    training_units: used,
                }
            })
            .collect::<Vec<_>>()
    };
    let mut bundles = Vec::new();
    for explanation in explanations {
        let Some(revision) = current_revision(conn, principal, &explanation.assembly_id)? else {
            continue;
        };
        let stored = stored_from_revision(conn, principal, &revision)?;
        let mut members = Vec::new();
        let mut closed = true;
        for member in &stored.members {
            match member_view(conn, principal, member)? {
                Some(view) => members.push(view),
                None if member.role.is_required_exception() => {
                    closed = false;
                    break;
                }
                None => {}
            }
        }
        if !closed {
            bundles.push(AssemblyBundle {
                id: stored.id,
                revision_id: stored.revision_id,
                kind: stored.kind,
                status: "qualification_unavailable".into(),
                route: "assembly_exception".into(),
                utility: explanation.utility,
                mass: explanation.mass,
                cues: explanation.cues,
                training_units: explanation.training_units,
                members: Vec::new(),
                present: false,
            });
            continue;
        }
        let decision = decide(
            presence,
            attested_brain,
            attested_policy,
            current,
            context_epoch,
            &LogicalId::new("revision", stored.revision_id.clone()),
            "assembly_brief",
        );
        let present = decision.suppresses();
        if present {
            for member in &mut members {
                member.preview = None;
            }
        }
        bundles.push(AssemblyBundle {
            id: stored.id,
            revision_id: stored.revision_id,
            kind: stored.kind,
            status: "ready".into(),
            route: "learned_assembly_route".into(),
            utility: explanation.utility,
            mass: explanation.mass,
            cues: explanation.cues,
            training_units: explanation.training_units,
            members,
            present,
        });
    }
    let status = if bundles.iter().any(|bundle| bundle.status == "ready") {
        "ready"
    } else if bundles
        .iter()
        .any(|bundle| bundle.status == "qualification_unavailable")
    {
        "qualification_unavailable"
    } else {
        "no_match"
    };
    let brief = render_assembly_brief(&bundles);
    Ok(AssemblyCompilation {
        status: status.into(),
        scope: scope.into(),
        bundles,
        brief,
    })
}

fn routes_enabled(conn: &Connection, principal: &str, scope: &str) -> Result<bool, String> {
    Ok(conn
        .query_row(
            "SELECT enabled FROM assembly_route_state WHERE principal=?1 AND scope_label=?2",
            params![principal, scope],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map_err(|err| err.to_string())?
        .unwrap_or(false))
}

fn refresh_routes(conn: &Connection, principal: &str, scope: &str, now: i64) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM assembly_route_edges WHERE principal=?1 AND scope_label=?2",
        params![principal, scope],
    )
    .map_err(|err| err.to_string())?;
    if !routes_enabled(conn, principal, scope)? {
        return Ok(0);
    }
    let events = live_events(conn, principal, scope)?;
    let edges = route_edges(&events, now)?;
    for edge in &edges {
        conn.execute(
            "INSERT INTO assembly_route_edges VALUES(?1,?2,?3,?4,?5,?6)",
            params![principal, scope, edge.cue, edge.target, edge.positive, edge.negative],
        )
        .map_err(|err| err.to_string())?;
    }
    Ok(edges.len())
}

fn stored_from_revision(
    conn: &Connection,
    principal: &str,
    revision_id: &str,
) -> Result<StoredAssembly, String> {
    let (assembly_id, scope, kind, codec, envelope): (String, String, String, String, String) = conn
        .query_row(
            "SELECT a.assembly_id,a.scope_label,a.kind,r.codec,r.envelope_json
             FROM assembly_revisions r JOIN assemblies a ON a.principal=r.principal AND a.assembly_id=r.assembly_id
             WHERE r.assembly_revision_id=?1 AND r.principal=?2",
            params![revision_id, principal],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|err| err.to_string())?;
    let mut member_stmt = conn
        .prepare("SELECT member_revision_id,role FROM assembly_members WHERE assembly_revision_id=?1 ORDER BY ordinal")
        .map_err(|err| err.to_string())?;
    let members = member_stmt
        .query_map(params![revision_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|err| err.to_string())?
        .map(|row| {
            let (revision, role) = row.map_err(|err| err.to_string())?;
            Ok(AssemblyMemberSpec {
                revision_id: revision,
                role: MembershipRole::parse(&role)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    drop(member_stmt);
    let mut guard_stmt = conn
        .prepare("SELECT guard_kind,guard_key,guard_epoch FROM assembly_guards WHERE assembly_revision_id=?1")
        .map_err(|err| err.to_string())?;
    let guards = guard_stmt
        .query_map(params![revision_id], |row| {
            Ok(AssemblyGuardSpec {
                kind: row.get(0)?,
                key: row.get(1)?,
                epoch: row.get(2)?,
            })
        })
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    Ok(StoredAssembly {
        id: assembly_id,
        revision_id: revision_id.into(),
        principal: principal.into(),
        scope,
        kind,
        codec,
        members,
        guards,
        envelope: serde_json::from_str(&envelope).map_err(|err| err.to_string())?,
    })
}

impl CortexRuntime {
    pub async fn put_assembly(&self, cx: &Cx, spec: AssemblySpec) -> Result<StoredAssembly, String> {
        check_id(&spec.id)?;
        check_id(&spec.scope)?;
        check_id(&spec.kind)?;
        if spec.members.is_empty() || spec.members.len() > 32 {
            return Err("invalid_assembly_members".into());
        }
        if spec.guards.len() > 32 {
            return Err("invalid_assembly_guards".into());
        }
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| err.to_string())?;
        let bodies = load_member_bodies(&tx, &spec.members)?;
        let envelope = factor_records(&bodies);
        let codec = envelope
            .get("codec")
            .and_then(Value::as_str)
            .ok_or("unknown_factor_codec")?;
        FactorCodec::parse(codec)?;
        let sequence = records::append_commit(
            &tx,
            &principal,
            None,
            super::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(&tx)),
        )
        .map_err(|err| err.to_string())?;
        tx.execute(
            "INSERT INTO assemblies(assembly_id,principal,scope_label,kind,created_sequence) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(principal,assembly_id) DO UPDATE SET kind=excluded.kind",
            params![spec.id, principal, spec.scope, spec.kind, sequence],
        )
        .map_err(|err| err.to_string())?;
        let revision_id = format!("{}@{}", spec.id, sequence);
        tx.execute(
            "INSERT INTO assembly_revisions VALUES(?1,?2,?3,?4,?5,?6)",
            params![revision_id, principal, spec.id, codec, envelope.to_string(), sequence],
        )
        .map_err(|err| err.to_string())?;
        for (ordinal, member) in spec.members.iter().enumerate() {
            tx.execute(
                "INSERT INTO assembly_members VALUES(?1,?2,?3,?4)",
                params![revision_id, member.revision_id, ordinal as i64, member.role.as_str()],
            )
            .map_err(|err| err.to_string())?;
        }
        for guard in &spec.guards {
            check_id(&guard.kind)?;
            check_id(&guard.key)?;
            check_id(&guard.epoch)?;
            tx.execute(
                "INSERT INTO assembly_guards VALUES(?1,?2,?3,?4)",
                params![revision_id, guard.kind, guard.key, guard.epoch],
            )
            .map_err(|err| err.to_string())?;
        }
        tx.commit().map_err(|err| err.to_string())?;
        Ok(StoredAssembly {
            id: spec.id,
            revision_id,
            principal,
            scope: spec.scope,
            kind: spec.kind,
            codec: codec.into(),
            members: spec.members,
            guards: spec.guards,
            envelope,
        })
    }

    pub async fn get_assembly(&self, cx: &Cx, id: &str) -> Result<StoredAssembly, String> {
        check_id(id)?;
        let principal = self.observation_principal()?;
        let conn = self.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let revision = current_revision(&conn, &principal, id)?.ok_or("assembly_missing")?;
        stored_from_revision(&conn, &principal, &revision)
    }

    pub async fn expand_assembly(&self, cx: &Cx, id: &str) -> Result<Vec<Value>, String> {
        let stored = self.get_assembly(cx, id).await?;
        let expanded = expand_records(&stored.envelope)?;
        if expanded.len() != stored.members.len() {
            return Err("assembly_expand_mismatch".into());
        }
        Ok(expanded)
    }

    pub async fn record_learning_event(&self, cx: &Cx, event: LearningEvent) -> Result<bool, String> {
        event.validate()?;
        let principal = self.observation_principal()?;
        if event.principal != principal {
            return Err("feedback_not_authorized".into());
        }
        let mut conn = self.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| err.to_string())?;
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM assemblies WHERE principal=?1 AND assembly_id=?2 AND scope_label=?3",
                params![principal, event.target, event.scope],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        if exists == 0 {
            return Err("learning_target_missing".into());
        }
        let retracted: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM learning_retractions WHERE origin=?1 AND origin_event_id=?2",
                params![event.origin, event.origin_event_id],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        if retracted > 0 {
            return Ok(false);
        }
        for source in &event.sources {
            let erased: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM learning_source_erasures WHERE source_id=?1",
                    params![source],
                    |row| row.get(0),
                )
                .map_err(|err| err.to_string())?;
            if erased > 0 {
                return Ok(false);
            }
        }
        let previous: Option<(i64, String)> = tx
            .query_row(
                "SELECT reward,cues_json FROM learning_events WHERE origin=?1 AND origin_event_id=?2",
                params![event.origin, event.origin_event_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|err| err.to_string())?;
        if let Some((reward, cues_json)) = previous {
            let cues: Vec<String> = serde_json::from_str(&cues_json).map_err(|err| err.to_string())?;
            if reward != i64::from(event.reward) || cues != event.cues {
                return Err("feedback_identity_conflict".into());
            }
            return Ok(false);
        }
        let sequence = records::append_commit(
            &tx,
            &principal,
            None,
            super::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(&tx)),
        )
        .map_err(|err| err.to_string())?;
        tx.execute(
            "INSERT INTO learning_events VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                event.origin,
                event.origin_event_id,
                principal,
                event.scope,
                event.training_unit,
                event.target,
                event.kind.as_str(),
                event.reward,
                serde_json::to_string(&event.cues).map_err(|err| err.to_string())?,
                event.observed_at,
                event.receipt_ref,
                sequence
            ],
        )
        .map_err(|err| err.to_string())?;
        for source in &event.sources {
            tx.execute(
                "INSERT INTO learning_dependencies VALUES(?1,?2,?3)",
                params![event.origin, event.origin_event_id, source],
            )
            .map_err(|err| err.to_string())?;
        }
        tx.commit().map_err(|err| err.to_string())?;
        Ok(true)
    }

    pub async fn retract_learning_event(
        &self,
        cx: &Cx,
        origin: &str,
        origin_event_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        check_id(origin)?;
        check_id(origin_event_id)?;
        if reason.is_empty() || reason.len() > 1024 {
            return Err("invalid_retraction_reason".into());
        }
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| err.to_string())?;
        let sequence = records::append_commit(
            &tx,
            &principal,
            None,
            super::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(&tx)),
        )
        .map_err(|err| err.to_string())?;
        tx.execute(
            "INSERT INTO learning_retractions VALUES(?1,?2,?3,?4) ON CONFLICT(origin,origin_event_id) DO UPDATE SET reason=excluded.reason",
            params![origin, origin_event_id, sequence, reason],
        )
        .map_err(|err| err.to_string())?;
        tx.commit().map_err(|err| err.to_string())
    }

    pub async fn erase_learning_source(&self, cx: &Cx, source_id: &str) -> Result<usize, String> {
        check_id(source_id)?;
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| err.to_string())?;
        let sequence = records::append_commit(
            &tx,
            &principal,
            None,
            super::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(&tx)),
        )
        .map_err(|err| err.to_string())?;
        let (_, restore, _) = records::brain_epochs(&tx);
        tx.execute(
            "INSERT INTO learning_source_erasures VALUES(?1,?2,?3) ON CONFLICT(source_id) DO UPDATE SET erasure_epoch=excluded.erasure_epoch",
            params![source_id, restore, sequence],
        )
        .map_err(|err| err.to_string())?;
        let mut stmt = tx
            .prepare("SELECT origin,origin_event_id FROM learning_dependencies WHERE source_id=?1")
            .map_err(|err| err.to_string())?;
        let keys = stmt
            .query_map(params![source_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        drop(stmt);
        for (origin, origin_event_id) in &keys {
            tx.execute(
                "INSERT INTO learning_retractions VALUES(?1,?2,?3,?4) ON CONFLICT(origin,origin_event_id) DO UPDATE SET reason=excluded.reason",
                params![origin, origin_event_id, sequence, "source_erased"],
            )
            .map_err(|err| err.to_string())?;
        }
        tx.commit().map_err(|err| err.to_string())?;
        Ok(keys.len())
    }

    pub async fn rebuild_assembly_routes(&self, cx: &Cx, scope: &str) -> Result<usize, String> {
        check_id(scope)?;
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| err.to_string())?;
        tx.execute(
            "INSERT INTO assembly_route_state VALUES(?1,?2,1) ON CONFLICT(principal,scope_label) DO UPDATE SET enabled=1",
            params![principal, scope],
        )
        .map_err(|err| err.to_string())?;
        let count = refresh_routes(&tx, &principal, scope, chrono::Utc::now().timestamp())?;
        tx.commit().map_err(|err| err.to_string())?;
        Ok(count)
    }

    pub async fn reset_assembly_routes(&self, cx: &Cx, scope: &str) -> Result<(), String> {
        check_id(scope)?;
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| err.to_string())?;
        tx.execute(
            "INSERT INTO assembly_route_state VALUES(?1,?2,0) ON CONFLICT(principal,scope_label) DO UPDATE SET enabled=0",
            params![principal, scope],
        )
        .map_err(|err| err.to_string())?;
        tx.execute(
            "DELETE FROM assembly_route_edges WHERE principal=?1 AND scope_label=?2",
            params![principal, scope],
        )
        .map_err(|err| err.to_string())?;
        tx.commit().map_err(|err| err.to_string())
    }

    pub async fn explain_assembly_routes(
        &self,
        cx: &Cx,
        scope: &str,
        cues: &[String],
        limit: usize,
    ) -> Result<Vec<RouteExplanation>, String> {
        check_id(scope)?;
        let principal = self.observation_principal()?;
        let conn = self.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        ensure(&conn)?;
        if !routes_enabled(&conn, &principal, scope)? {
            return Ok(Vec::new());
        }
        let mut stmt = conn
            .prepare("SELECT cue,assembly_id,positive,negative FROM assembly_route_edges WHERE principal=?1 AND scope_label=?2")
            .map_err(|err| err.to_string())?;
        let edges = stmt
            .query_map(params![principal, scope], |row| {
                Ok(RouteEdge {
                    cue: row.get(0)?,
                    target: row.get(1)?,
                    positive: row.get(2)?,
                    negative: row.get(3)?,
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        drop(stmt);
        let mut allowed_stmt = conn
            .prepare("SELECT assembly_id FROM assemblies WHERE principal=?1 AND scope_label=?2")
            .map_err(|err| err.to_string())?;
        let allowed = allowed_stmt
            .query_map(params![principal, scope], |row| row.get(0))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|err| err.to_string())?;
        drop(allowed_stmt);
        let ranked = rank_routes(&edges, cues, &allowed, 0.5);
        let events = live_events(&conn, &principal, scope)?;
        Ok(ranked
            .into_iter()
            .take(limit.min(8))
            .map(|score| {
                let used: Vec<String> = events
                    .iter()
                    .filter(|event| event.target == score.target && event.cues.iter().any(|cue| cues.contains(cue)))
                    .map(|event| event.training_unit.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let matched = cues
                    .iter()
                    .filter(|cue| edges.iter().any(|edge| edge.cue == **cue && edge.target == score.target))
                    .cloned()
                    .collect();
                RouteExplanation {
                    assembly_id: score.target,
                    utility: score.utility,
                    mass: score.mass,
                    cues: matched,
                    training_units: used,
                }
            })
            .collect())
    }

    pub async fn compile_assemblies(
        &self,
        cx: &Cx,
        scope: &str,
        cues: &[String],
        limit: usize,
        presence: Option<&ContextPresence>,
        attested_brain: Option<&str>,
        attested_policy: Option<&str>,
        context_epoch: &str,
    ) -> Result<AssemblyCompilation, String> {
        check_id(scope)?;
        let principal = self.observation_principal()?;
        let conn = self.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        let (_, restore, policy) = records::brain_epochs(&conn);
        compile_for_cues(
            &conn,
            &principal,
            scope,
            cues,
            limit,
            presence,
            attested_brain,
            attested_policy,
            &CurrentEpochs {
                brain_epoch: restore,
                policy_epoch: policy,
            },
            context_epoch,
        )
    }
}

pub(crate) fn suggest_authorized_members(
    conn: &Connection,
    principal: &str,
    scope: &str,
    cues: &[String],
    limit: usize,
) -> Result<Vec<(String, String, MembershipRole)>, String> {
    ensure(conn)?;
    if !routes_enabled(conn, principal, scope)? {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare("SELECT cue,assembly_id,positive,negative FROM assembly_route_edges WHERE principal=?1 AND scope_label=?2")
        .map_err(|err| err.to_string())?;
    let edges = stmt
        .query_map(params![principal, scope], |row| {
            Ok(RouteEdge {
                cue: row.get(0)?,
                target: row.get(1)?,
                positive: row.get(2)?,
                negative: row.get(3)?,
            })
        })
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    drop(stmt);
    let mut allowed_stmt = conn
        .prepare("SELECT assembly_id FROM assemblies WHERE principal=?1 AND scope_label=?2")
        .map_err(|err| err.to_string())?;
    let allowed = allowed_stmt
        .query_map(params![principal, scope], |row| row.get(0))
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|err| err.to_string())?;
    drop(allowed_stmt);
    let ranked = rank_routes(&edges, cues, &allowed, 0.5);
    let mut out = Vec::new();
    for score in ranked.into_iter().take(limit.min(8)) {
        if score.utility <= 0.0 {
            continue;
        }
        let Some(revision) = current_revision(conn, principal, &score.target)? else {
            continue;
        };
        let mut member_stmt = conn
            .prepare("SELECT member_revision_id,role FROM assembly_members WHERE assembly_revision_id=?1 ORDER BY ordinal")
            .map_err(|err| err.to_string())?;
        let members = member_stmt
            .query_map(params![revision], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        drop(member_stmt);
        for (revision_id, role) in members {
            let role = MembershipRole::parse(&role)?;
            let source: Option<String> = conn
                .query_row(
                    "SELECT e.source_id FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key WHERE e.principal=?1 AND e.revision_id=?2 AND g.scope_label=?3",
                    params![principal, revision_id, scope],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|err| err.to_string())?;
            if let Some(source_id) = source {
                out.push((source_id, score.target.clone(), role));
            }
        }
    }
    Ok(out)
}

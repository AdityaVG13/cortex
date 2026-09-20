use super::*;
use crate::db::records;
use crate::handlers::truncate_chars;
use crate::runtime::observation;
use cortex_logic::assembly::{LearningEvent, LearningKind, MembershipRole};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

pub(super) fn ensure(conn: &Connection) -> Result<(), String> {
    records::ensure_authoritative_schema(conn).map_err(|err| err.to_string())?;
    conn.execute_batch(DDL).map_err(|err| err.to_string())
}

pub(super) fn check_id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 1024 {
        return Err("invalid_assembly_identity".into());
    }
    Ok(())
}

pub(super) fn canonical_scope(raw: &str) -> Result<String, String> {
    let scope = observation::normalize_scope(raw);
    check_id(&scope)?;
    Ok(scope)
}

pub(super) fn query_mapped<T>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, String> {
    let mut stmt = conn.prepare(sql).map_err(|err| err.to_string())?;
    stmt.query_map(params, map)
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())
}

pub(super) fn current_revision(
    conn: &Connection,
    principal: &str,
    id: &str,
) -> Result<Option<String>, String> {
    conn.query_row("SELECT assembly_revision_id FROM assembly_revisions WHERE principal=?1 AND assembly_id=?2 ORDER BY recorded_sequence DESC LIMIT 1", params![principal, id], |row| row.get(0)).optional().map_err(|err| err.to_string())
}

pub(super) fn load_member_bodies(
    conn: &Connection,
    members: &[AssemblyMemberSpec],
) -> Result<Vec<Value>, String> {
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

pub(super) fn live_events(
    conn: &Connection,
    principal: &str,
    scope: &str,
) -> Result<Vec<LearningEvent>, String> {
    let rows = query_mapped(
        conn,
        "SELECT e.origin,e.origin_event_id,e.principal,e.scope_label,e.training_unit,e.target,e.kind,e.reward,e.cues_json,e.observed_at,e.receipt_ref FROM learning_events e WHERE e.principal=?1 AND e.scope_label=?2 AND NOT EXISTS(SELECT 1 FROM learning_retractions r WHERE r.origin=e.origin AND r.origin_event_id=e.origin_event_id) AND NOT EXISTS(SELECT 1 FROM learning_dependencies d JOIN learning_source_erasures x ON x.source_id=d.source_id WHERE d.origin=e.origin AND d.origin_event_id=e.origin_event_id)",
        params![principal, scope],
        |row| {
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
        },
    )?;
    let mut events = Vec::new();
    for (
        origin,
        origin_event_id,
        principal,
        scope,
        training_unit,
        target,
        kind,
        reward,
        cues_json,
        observed_at,
        receipt_ref,
    ) in rows
    {
        let cues: Vec<String> = serde_json::from_str(&cues_json).map_err(|err| err.to_string())?;
        let sources = query_mapped(
            conn,
            "SELECT source_id FROM learning_dependencies WHERE origin=?1 AND origin_event_id=?2 ORDER BY source_id",
            params![origin, origin_event_id],
            |row| row.get(0),
        )?;
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
    crate::handlers::alnum_underscore_lower_set(text)
        .into_iter()
        .collect()
}

pub(super) fn preview_text(body: &Value) -> String {
    body.get("text")
        .and_then(Value::as_str)
        .or_else(|| body.pointer("/observation/text").and_then(Value::as_str))
        .map(|text| truncate_chars(text, 200))
        .filter(|text: &String| !text.is_empty())
        .unwrap_or_else(|| canonical_preview(body))
}

pub(super) fn canonical_preview(body: &Value) -> String {
    truncate_chars(&body.to_string(), 200)
}

fn member_shell(
    member: &AssemblyMemberSpec,
    expand: String,
    preview: Option<String>,
) -> AssemblyMemberView {
    AssemblyMemberView {
        role: member.role.as_str().into(),
        revision_id: member.revision_id.clone(),
        expand,
        preview,
        required: member.role.is_required_exception(),
    }
}

fn unavailable_member(member: &AssemblyMemberSpec) -> Result<Option<AssemblyMemberView>, String> {
    Ok((!member.role.is_required_exception())
        .then(|| member_shell(member, format!("rev:{}", member.revision_id), None)))
}

pub(super) fn member_view(
    conn: &Connection,
    principal: &str,
    member: &AssemblyMemberSpec,
) -> Result<Option<AssemblyMemberView>, String> {
    let observation: Option<(String, Vec<u8>)> = conn.query_row("SELECT e.source_id,s.inline_payload FROM observation_events e JOIN sources s ON s.source_id=e.source_id WHERE e.principal=?1 AND e.revision_id=?2 AND s.availability='owned_inline' AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id)", params![principal, member.revision_id], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(|err| err.to_string())?;
    if let Some((source_id, bytes)) = observation {
        let preview = truncate_chars(
            &String::from_utf8(bytes).map_err(|_| "source_not_utf8".to_string())?,
            200,
        );
        return Ok(Some(member_shell(
            member,
            format!("obs:{source_id}"),
            Some(preview),
        )));
    }
    if crate::db::count_sql(
        conn,
        "SELECT COUNT(*) FROM observation_events e JOIN observation_retractions t ON t.source_id=e.source_id WHERE e.principal=?1 AND e.revision_id=?2",
        params![principal, member.revision_id],
    )? > 0
    {
        return unavailable_member(member);
    }
    let Some(body) =
        records::revision_body(conn, &member.revision_id).map_err(|err| err.to_string())?
    else {
        return unavailable_member(member);
    };
    Ok(Some(member_shell(
        member,
        format!("rev:{}", member.revision_id),
        Some(preview_text(&body)),
    )))
}

pub(super) fn render_assembly_brief(bundles: &[AssemblyBundle]) -> String {
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

pub(super) fn load_member_rows(
    conn: &Connection,
    revision_id: &str,
) -> Result<Vec<(String, String)>, String> {
    query_mapped(
        conn,
        "SELECT member_revision_id,role FROM assembly_members WHERE assembly_revision_id=?1 ORDER BY ordinal",
        params![revision_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )
}

pub(super) fn load_member_specs(
    conn: &Connection,
    revision_id: &str,
) -> Result<Vec<AssemblyMemberSpec>, String> {
    load_member_rows(conn, revision_id)?
        .into_iter()
        .map(|(revision, role)| {
            Ok(AssemblyMemberSpec {
                revision_id: revision,
                role: MembershipRole::parse(&role)?,
            })
        })
        .collect()
}

pub(super) fn normalized_scopes(scopes: impl IntoIterator<Item = String>) -> Vec<String> {
    scopes
        .into_iter()
        .map(|scope| observation::normalize_scope(&scope))
        .filter(|scope| !scope.is_empty())
        .collect()
}

pub(super) fn bundle_from(
    stored: StoredAssembly,
    explanation: RouteExplanation,
    status: &'static str,
    route: &'static str,
    members: Vec<AssemblyMemberView>,
    present: bool,
) -> AssemblyBundle {
    AssemblyBundle {
        id: stored.id,
        revision_id: stored.revision_id,
        kind: stored.kind,
        status: status.into(),
        route: route.into(),
        utility: explanation.utility,
        mass: explanation.mass,
        cues: explanation.cues,
        training_units: explanation.training_units,
        members,
        present,
    }
}

pub(super) fn compilation_status(bundles: &[AssemblyBundle]) -> &'static str {
    ["ready", "qualification_unavailable"]
        .into_iter()
        .find(|status| bundles.iter().any(|bundle| bundle.status == *status))
        .unwrap_or("no_match")
}

pub(crate) mod compile;
pub(crate) use compile::suggest_authorized_members;
pub(in crate::runtime::assembly) use compile::{
    compile_for_cues, load_allowed_assemblies, load_route_edges, merge_compilations,
    ranked_route_explanations, resolve_compile_scopes, routes_enabled, stored_from_revision,
};

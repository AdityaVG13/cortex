use super::super::{
    AssemblyGuardSpec, StoredAssembly, current_revision, load_member_specs, query_mapped,
};
use super::loaded_routes;
use cortex_logic::assembly::{MembershipRole, rank_routes};
use rusqlite::{Connection, OptionalExtension, params};

pub(in crate::runtime::assembly) fn stored_from_revision(
    conn: &Connection,
    principal: &str,
    revision_id: &str,
) -> Result<StoredAssembly, String> {
    let (assembly_id, scope, kind, codec, envelope): (String, String, String, String, String) = conn.query_row("SELECT a.assembly_id,a.scope_label,a.kind,r.codec,r.envelope_json FROM assembly_revisions r JOIN assemblies a ON a.principal=r.principal AND a.assembly_id=r.assembly_id WHERE r.assembly_revision_id=?1 AND r.principal=?2", params![revision_id, principal], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))).map_err(|err| err.to_string())?;
    let members = load_member_specs(conn, revision_id)?;
    let guards = query_mapped(
        conn,
        "SELECT guard_kind,guard_key,guard_epoch FROM assembly_guards WHERE assembly_revision_id=?1",
        params![revision_id],
        |row| {
            Ok(AssemblyGuardSpec {
                kind: row.get(0)?,
                key: row.get(1)?,
                epoch: row.get(2)?,
            })
        },
    )?;
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

pub(crate) fn suggest_authorized_members(
    conn: &Connection,
    principal: &str,
    scope: &str,
    cues: &[String],
    limit: usize,
) -> Result<Vec<(String, String, MembershipRole)>, String> {
    let Some((edges, allowed)) = loaded_routes(conn, principal, scope)? else {
        return Ok(Vec::new());
    };
    let ranked = rank_routes(&edges, cues, &allowed, 0.5);
    let mut out = Vec::new();
    for score in ranked.into_iter().take(limit.min(8)) {
        if score.utility <= 0.0 {
            continue;
        }
        let Some(revision) = current_revision(conn, principal, &score.target)? else {
            continue;
        };
        let members = load_member_specs(conn, &revision)?;
        for member in members {
            let source: Option<String> = conn.query_row(&format!("SELECT e.source_id FROM {} WHERE e.principal=?1 AND e.revision_id=?2 AND g.scope_label=?3", crate::runtime::observation::EVENT_GRANT_JOIN), params![principal, member.revision_id, scope], |row| row.get(0)).optional().map_err(|err| err.to_string())?;
            if let Some(source_id) = source {
                out.push((source_id, score.target.clone(), member.role));
            }
        }
    }
    Ok(out)
}

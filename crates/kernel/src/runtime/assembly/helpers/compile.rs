use super::*;
use cortex_logic::assembly::{LearningEvent, RouteEdge, rank_routes, route_edges};
use cortex_logic::presence::{CurrentEpochs, decide};
use cortex_logic::protocol::{ContextPresence, LogicalId};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;

fn disabled_compilation(scope: impl Into<String>) -> AssemblyCompilation {
    AssemblyCompilation {
        status: "disabled".into(),
        scope: scope.into(),
        bundles: Vec::new(),
        brief: String::new(),
    }
}

pub(in crate::runtime::assembly) fn load_route_edges(
    conn: &Connection,
    principal: &str,
    scope: &str,
) -> Result<Vec<RouteEdge>, String> {
    query_mapped(
        conn,
        "SELECT cue,assembly_id,positive,negative FROM assembly_route_edges WHERE principal=?1 AND scope_label=?2",
        params![principal, scope],
        |row| {
            Ok(RouteEdge {
                cue: row.get(0)?,
                target: row.get(1)?,
                positive: row.get(2)?,
                negative: row.get(3)?,
            })
        },
    )
}

pub(in crate::runtime::assembly) fn load_allowed_assemblies(
    conn: &Connection,
    principal: &str,
    scope: &str,
) -> Result<Vec<String>, String> {
    query_mapped(
        conn,
        "SELECT assembly_id FROM assemblies WHERE principal=?1 AND scope_label=?2",
        params![principal, scope],
        |row| row.get(0),
    )
}

fn loaded_routes(
    conn: &Connection,
    principal: &str,
    scope: &str,
) -> Result<Option<(Vec<RouteEdge>, Vec<String>)>, String> {
    ensure(conn)?;
    if !routes_enabled(conn, principal, scope)? {
        return Ok(None);
    }
    Ok(Some((
        load_route_edges(conn, principal, scope)?,
        load_allowed_assemblies(conn, principal, scope)?,
    )))
}

pub(in crate::runtime::assembly) fn ranked_route_explanations(
    edges: &[RouteEdge],
    cues: &[String],
    allowed: &[String],
    events: &[LearningEvent],
    limit: usize,
    positive_only: bool,
) -> Vec<RouteExplanation> {
    rank_routes(edges, cues, allowed, 0.5)
        .into_iter()
        .take(limit.min(8))
        .filter(|score| !positive_only || score.utility > 0.0)
        .map(|score| {
            let used: Vec<String> = events
                .iter()
                .filter(|event| {
                    event.target == score.target && event.cues.iter().any(|cue| cues.contains(cue))
                })
                .map(|event| event.training_unit.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let matched = cues
                .iter()
                .filter(|cue| {
                    edges
                        .iter()
                        .any(|edge| edge.cue == **cue && edge.target == score.target)
                })
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
        .collect()
}

pub(in crate::runtime::assembly) fn compile_for_cues(
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
    let Some((edges, allowed)) = loaded_routes(conn, principal, scope)? else {
        return Ok(disabled_compilation(scope));
    };
    let events = live_events(conn, principal, scope)?;
    let explanations = ranked_route_explanations(&edges, cues, &allowed, &events, limit, true);
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
            bundles.push(bundle_from(
                stored,
                explanation,
                "qualification_unavailable",
                "assembly_exception",
                Vec::new(),
                false,
            ));
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
        bundles.push(bundle_from(
            stored,
            explanation,
            "ready",
            "learned_assembly_route",
            members,
            present,
        ));
    }
    let brief = render_assembly_brief(&bundles);
    Ok(AssemblyCompilation {
        status: compilation_status(&bundles).into(),
        scope: scope.into(),
        bundles,
        brief,
    })
}

pub(in crate::runtime::assembly) fn resolve_compile_scopes(
    conn: &Connection,
    principal: &str,
    paths: &[String],
    extra_scope: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut scopes = observation::resolve_query_scopes(conn, principal, paths, extra_scope)?;
    let query_paths: Vec<String> = paths
        .iter()
        .map(|path| observation::normalize_scope(path))
        .filter(|path| !path.is_empty())
        .collect();
    if query_paths.is_empty() {
        return Ok(normalized_scopes(scopes));
    }
    let stored = query_mapped(
        conn,
        "SELECT DISTINCT scope_label FROM assemblies WHERE principal=?1",
        params![principal],
        |row| row.get::<_, String>(0),
    )?;
    for label in stored {
        let normalized = observation::normalize_scope(&label);
        if !observation::scope_is_path(&normalized) {
            continue;
        }
        if query_paths
            .iter()
            .any(|query| observation::scopes_compatible(&normalized, query))
            && !scopes
                .iter()
                .any(|scope| observation::normalize_scope(scope) == normalized)
        {
            scopes.push(normalized);
        }
    }
    Ok(normalized_scopes(scopes))
}

pub(in crate::runtime::assembly) fn merge_compilations(
    parts: Vec<AssemblyCompilation>,
    display_scope: &str,
    limit: usize,
) -> AssemblyCompilation {
    let mut enabled = false;
    let mut bundles = Vec::new();
    let mut seen = BTreeSet::new();
    for part in parts {
        if part.status != "disabled" {
            enabled = true;
        }
        for bundle in part.bundles {
            if seen.insert(bundle.id.clone()) {
                bundles.push(bundle);
            }
        }
    }
    if !enabled {
        return disabled_compilation(display_scope);
    }
    bundles.sort_by(|left, right| {
        right
            .utility
            .partial_cmp(&left.utility)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });
    bundles.truncate(limit.min(8));
    let brief = render_assembly_brief(&bundles);
    AssemblyCompilation {
        status: compilation_status(&bundles).into(),
        scope: display_scope.into(),
        bundles,
        brief,
    }
}

pub(in crate::runtime::assembly) fn routes_enabled(
    conn: &Connection,
    principal: &str,
    scope: &str,
) -> Result<bool, String> {
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

pub(crate) fn refresh_routes(
    conn: &Connection,
    principal: &str,
    scope: &str,
    now: i64,
) -> Result<usize, String> {
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
            params![
                principal,
                scope,
                edge.cue,
                edge.target,
                edge.positive,
                edge.negative
            ],
        )
        .map_err(|err| err.to_string())?;
    }
    Ok(edges.len())
}

mod members;
pub(in crate::runtime::assembly) use members::stored_from_revision;
pub(crate) use members::suggest_authorized_members;

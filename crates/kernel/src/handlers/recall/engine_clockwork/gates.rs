use super::RecallContext;
use crate::handlers::starts_with_ascii_ignore_case;
use crate::protocol::nonempty_opt;

pub(super) fn caller_acl_param(ctx: &RecallContext) -> Option<i64> {
    if ctx.team_mode { ctx.caller_id } else { None }
}

/// Empty `as_of` is omitted JSON, not "valid at the empty instant".
/// `julianday('')` is NULL and would fail every temporal gate.
pub(super) fn as_of_bind(ctx: &RecallContext) -> Option<&str> {
    nonempty_opt(ctx.as_of.as_deref())
}

fn qualified_instant_gates(alias: &str, instant: &str) -> String {
    format!(
        "{} AND {}",
        crate::db::temporal_bounds_sql_at(alias, instant),
        crate::db::unorphaned_version_sql(alias)
    )
}

pub(super) fn qualified_validity_gates(alias: &str) -> String {
    qualified_instant_gates(alias, "'now'")
}

pub(super) fn qualified_current_gates(alias: &str) -> String {
    format!(
        "{a}.status NOT IN ('superseded','archived') AND {rest}",
        a = alias,
        rest = qualified_validity_gates(alias)
    )
}

pub(super) fn qualified_as_of_gates(alias: &str, bind: &str) -> String {
    qualified_instant_gates(alias, bind)
}

pub(super) fn temporal_gates(alias: &str, ctx: &RecallContext, as_of_slot: &str) -> String {
    if as_of_bind(ctx).is_some() {
        qualified_as_of_gates(alias, as_of_slot)
    } else {
        let rest = if ctx.include_cold {
            qualified_validity_gates(alias)
        } else {
            qualified_current_gates(alias)
        };
        format!("{rest} AND ({as_of_slot} IS NULL OR 1)")
    }
}

pub(super) fn qualified_acl(alias: &str, bind: &str) -> String {
    format!(
        " AND ({b} IS NULL OR {a}.owner_id IS NULL OR {a}.owner_id = {b} OR {a}.visibility IN ('shared','team','public'))",
        a = alias,
        b = bind
    )
}

pub(super) fn source_prefix_is_path(prefix: &str) -> bool {
    prefix.contains('/') && !prefix.contains("::")
}

pub(super) fn source_prefix_applies_to_kind(kind: &str, prefix: Option<&str>) -> bool {
    let Some(prefix) = nonempty_opt(prefix) else {
        return true;
    };
    let lower = prefix.to_ascii_lowercase();
    if lower.starts_with("memory::") {
        matches!(kind, "memory" | "memories")
    } else if lower.starts_with("decision::") {
        matches!(kind, "decision" | "decisions")
    } else {
        true
    }
}

pub(super) fn scoped_source_matches(source: &str, prefix: Option<&str>) -> bool {
    let Some(prefix) = nonempty_opt(prefix) else {
        return true;
    };
    if !starts_with_ascii_ignore_case(source, prefix) {
        return false;
    }
    if source_prefix_is_path(prefix) {
        let rest = &source.as_bytes()[prefix.len()..];
        return rest.is_empty() || rest.first() == Some(&b'/');
    }
    true
}

pub(super) fn candidate_matches_source_scope(
    kind: &str,
    id: i64,
    source: &str,
    prefix: Option<&str>,
) -> bool {
    if !source_prefix_applies_to_kind(kind, prefix) {
        return false;
    }
    if scoped_source_matches(source, prefix) {
        return true;
    }
    let ident = if matches!(kind, "decision" | "decisions") {
        format!("decision::{id}")
    } else {
        format!("memory::{id}")
    };
    scoped_source_matches(&ident, prefix)
}

/// LIKE `prefix%` plus a slash-boundary guard so `src/app` does not fill
/// FTS LIMIT with `src/application`. Identity prefixes skip the guard (`?6`).
/// Synthetic `{kind}::{id}` still matches when the display source is context.
pub(super) fn source_scope_sql(col: &str, ident: &str) -> String {
    format!(
        "(?3 IS NULL OR (({col} LIKE ?3 ESCAPE '\\' AND (?6 IS NULL OR instr('/' || lower({col}) || '/', '/' || lower(?6) || '/') > 0)) OR {ident} LIKE ?3 ESCAPE '\\'))"
    )
}

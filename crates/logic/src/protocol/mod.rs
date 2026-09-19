//! Protocol vocabulary shared by every surface (HTTP, MCP, CLI, SDKs).
//!
//! Determinism class: pure data; no clocks, no I/O.
//! Invariants:
//! - identity kinds are distinct Rust types and never interchangeable (`ids`);
//! - every response carries one `ResponseStatus`; transport codes derive from it,
//!   never the reverse (`status`);
//! - the request envelope separates adapter-owned authority from model-supplied
//!   selectors and keeps temporal/cursor/presence fields independent (`envelope`);
//! - a `Receipt` reports durability as a vector, never a boolean (`receipt`).
//! No-claim boundary: these types validate shape and vocabulary, not policy.

pub mod envelope;
pub mod ids;
pub mod json_args;
pub mod receipt;
pub mod status;

pub use envelope::{
    ContextPresence, Envelope, EnvelopeError, KNOWN_OPERATIONS, Limits, PrincipalContext, Scope,
};
pub use ids::{ExactRef, Frontier, Integrity, Locator, LogicalId, Principal};
pub use json_args::{
    CWD_KEYS, arg_bool, arg_f64, arg_i64, arg_list, arg_str, arg_usize, json_f64, json_i64,
    json_str,
};
pub use receipt::{
    AckProfile, CaptureReceipt, CaptureStatus, DurabilityVector, PayloadAvailability, Receipt,
};
pub use status::ResponseStatus;

fn like_affix(raw: &str, prefix: &str, suffix: &str) -> String {
    let mut out = String::with_capacity(prefix.len() + raw.len().saturating_mul(2) + suffix.len());
    out.push_str(prefix);
    for ch in raw.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push_str(suffix);
    out
}

/// Escape `%` `_` `\` for SQL `LIKE ? ESCAPE '\'` without adding a wildcard.
pub fn like_escape(raw: &str) -> String {
    like_affix(raw, "", "")
}

/// Prefix pattern: escaped `raw` plus `%`.
pub fn like_prefix(raw: &str) -> String {
    like_affix(raw, "", "%")
}

/// Contains pattern: `%` plus escaped `raw` plus `%`.
pub fn like_contains(raw: &str) -> String {
    like_affix(raw, "%", "%")
}

/// Trimmed nonempty borrow, or `None`.
pub fn nonempty_str(s: &str) -> Option<&str> {
    let t = s.trim();
    (!t.is_empty()).then_some(t)
}

/// `nonempty_str` over an optional borrow.
pub fn nonempty_opt(value: Option<&str>) -> Option<&str> {
    value.and_then(nonempty_str)
}

/// Trimmed nonempty owned string, or `None`.
pub fn nonempty_trimmed(s: String) -> Option<String> {
    nonempty_str(&s).map(str::to_string)
}

/// `None` when `s` is empty or whitespace-only; otherwise the original string.
pub fn nonempty_owned(s: String) -> Option<String> {
    (!s.trim().is_empty()).then_some(s)
}

/// Consecutive-token phrase match on an already-lowercased haystack.
/// Split on non-alnum so hyphenated `always-on` still sees `always`, without `never` inside `whenever`.
pub fn hay_has_phrase(hay_lower: &str, needle: &str) -> bool {
    let parts: Vec<&str> = needle
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return false;
    }
    hay_lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|tok| !tok.is_empty())
        .collect::<Vec<_>>()
        .windows(parts.len())
        .any(|window| window.iter().copied().eq(parts.iter().copied()))
}

/// Direct-child path pattern: escaped `raw` plus `/%`.
pub fn like_children(raw: &str) -> String {
    like_affix(raw, "", "/%")
}

/// Exact identity plus trailing ` (model)` for SQL `LIKE ? ESCAPE '\'`.
/// Bind `eq_bind` to the identity and `eq_bind + 1` to `{identity} (%`.
pub fn ident_match_sql(col: &str, eq_bind: u32) -> String {
    format!(
        "(lower(trim({col})) = ?{eq_bind} OR lower(trim({col})) LIKE ?{} ESCAPE '\\')",
        eq_bind + 1
    )
}

/// Skip the identity match when `bind` is NULL.
pub fn optional_ident_match_sql(col: &str, bind: u32) -> String {
    format!("(?{bind} IS NULL OR {})", ident_match_sql(col, bind))
}

/// Skip the source/context LIKE filter when `bind` is NULL.
pub fn optional_coalesce_like_sql(col: &str, prefix: &str, id_expr: &str, bind: u32) -> String {
    format!(
        "(?{bind} IS NULL OR COALESCE({col}, '{prefix}' || {id_expr}) LIKE ?{bind} ESCAPE '\\')"
    )
}

/// Currently valid temporal window. Empty strings are unbounded (same as NULL).
pub const TEMPORAL_BOUNDS_SQL: &str = "(expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now'))";

/// Active row that is currently valid.
pub const ACTIVE_TEMPORAL_SQL: &str = "status = 'active' AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now'))";

/// Expiry is set and already past (`< now`). Inverse of the expiry half of `TEMPORAL_BOUNDS_SQL`.
pub const EXPIRED_SQL: &str = "expires_at IS NOT NULL AND TRIM(expires_at) != '' AND julianday(expires_at) < julianday('now')";

/// Version pointer that is not an orphaned version row.
pub const UNORPHANED_VERSION_SQL: &str =
    "(version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))";

/// First non-blank `updated_at`, else `created_at`. Blank TEXT is unbounded, not NULL.
pub const UPDATED_CREATED_STAMP_SQL: &str =
    "COALESCE(NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))";

/// Decisions-delta order: `created_at` then `updated_at`. Do not swap with `UPDATED_CREATED_STAMP_SQL`.
pub const CREATED_UPDATED_STAMP_SQL: &str =
    "COALESCE(NULLIF(TRIM(created_at), ''), NULLIF(TRIM(updated_at), ''))";

/// First non-blank `last_accessed`, else `created_at`.
pub const LAST_ACCESSED_CREATED_STAMP_SQL: &str =
    "COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(created_at), ''))";

/// Decay/touch order: `last_accessed`, then `updated_at`, then `created_at`.
pub const LAST_TOUCHED_STAMP_SQL: &str = "COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))";

/// First non-blank `valid_from`, else `observed_at`, else `created_at`.
pub const VALIDITY_START_SQL: &str =
    "COALESCE(NULLIF(TRIM(valid_from), ''), NULLIF(TRIM(observed_at), ''), created_at)";

fn sql_alias_prefix(alias: &str) -> String {
    if alias.is_empty() {
        String::new()
    } else if alias.ends_with('.') {
        alias.to_string()
    } else {
        format!("{alias}.")
    }
}

/// Temporal bounds at `instant` (`"'now'"` or a bound parameter), optional alias.
pub fn temporal_bounds_sql_at(alias: &str, instant: &str) -> String {
    let p = sql_alias_prefix(alias);
    format!(
        "({p}expires_at IS NULL OR TRIM({p}expires_at) = '' OR julianday({p}expires_at) > julianday({instant})) AND ({p}valid_from IS NULL OR TRIM({p}valid_from) = '' OR julianday({p}valid_from) <= julianday({instant})) AND ({p}valid_until IS NULL OR TRIM({p}valid_until) = '' OR julianday({p}valid_until) > julianday({instant}))"
    )
}

/// `UNORPHANED_VERSION_SQL` with an optional table alias.
pub fn unorphaned_version_sql(alias: &str) -> String {
    let p = sql_alias_prefix(alias);
    format!(
        "({p}version_id IS NULL OR {p}version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))"
    )
}

/// `VALIDITY_START_SQL` with an optional table alias (`"d"` or `"d."`).
pub fn validity_start_sql(alias: &str) -> String {
    let p = sql_alias_prefix(alias);
    format!(
        "COALESCE(NULLIF(TRIM({p}valid_from), ''), NULLIF(TRIM({p}observed_at), ''), {p}created_at)"
    )
}

/// `ACTIVE_TEMPORAL_SQL` with an optional table alias (`"m"` or `"m."`).
pub fn active_temporal_sql(alias: &str) -> String {
    if alias.is_empty() {
        return ACTIVE_TEMPORAL_SQL.to_string();
    }
    format!(
        "{}status = 'active' AND {}",
        sql_alias_prefix(alias),
        temporal_bounds_sql_at(alias, "'now'")
    )
}

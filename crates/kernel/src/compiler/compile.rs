use super::*;
use crate::handlers::estimate_tokens;
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;

fn deterministic_now(conn: &Connection) -> chrono::DateTime<chrono::Utc> {
    if let Some(ts) = super::stored_max_timestamp(conn) {
        if let Some(dt) = super::parse_timestamp(Some(&ts)) {
            return dt;
        }
    }
    chrono::Utc::now()
}

fn stability_for_item(item: &ContextItem) -> u8 {
    if item.name == "identity" {
        return 100;
    }
    if item.name == "## Constraints" {
        return 99;
    }
    if item.name.starts_with("ranked:") {
        return 94;
    }
    match item.name.as_str() {
        "## TRUTH" => 95,
        "CONFLICTS:" => 85,
        "## Pending Tasks" => 75,
        "## Your Active Tasks" => 74,
        "## Active Focus" => 73,
        "## Pending Messages" => 60,
        "## Active Locks" => 55,
        "## Active Agents" => 50,
        "Recent decisions:" => 45,
        "New decisions:" => 44,
        "New knowledge:" => 43,
        "## Feed" => 30,
        "Activity since last boot:" => 10,
        "delta" => 50,
        _ => 50,
    }
}

/// Durable, active, currently valid decisions (constraint-like kinds first,
/// then oldest first so long-standing rules are never displaced by churn),
/// bounded to `BOOT_CONSTRAINTS_MAX` lines with an explicit omission count.
pub const BOOT_CONSTRAINTS_MAX: usize = 40;
pub fn build_constraints_capsule(conn: &Connection) -> (String, usize) {
    // Cheap cache key: the durable set's cardinality, newest id and newest
    // update; the capsule is rebuilt only when that changes. Project paths
    // are part of the key so a scoped boot cannot reuse an unscoped cache.
    let scope = super::capsules::owner_clause(
        conn,
        "decisions",
        super::capsules::boot_owner(),
    );
    let key: String = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) || ':' || COALESCE(MAX(id),0) || ':' || COALESCE(MAX(julianday(updated_at)),'') \
                 FROM decisions WHERE status = 'active' AND COALESCE(retention_class,'operational') = 'durable' \
                 AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) \
                 AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) \
                 AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now')) \
                 AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')){scope}"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let key = format!(
        "{key}|{}|{}",
        super::capsules::with_boot_paths(|paths| paths.join("\u{1f}")),
        super::capsules::boot_owner()
            .map(|id| id.to_string())
            .unwrap_or_default()
    );
    if let Some((cached, omitted)) = super::cache::cache_get(conn, "constraints_capsule", &key) {
        return (cached, omitted);
    }
    let (text, omitted) = build_constraints_capsule_uncached(conn);
    super::cache::cache_set(conn, "constraints_capsule", &key, &text, omitted);
    (text, omitted)
}
fn build_constraints_capsule_uncached(conn: &Connection) -> (String, usize) {
    let scope = super::capsules::owner_clause(conn, "decisions", super::capsules::boot_owner());
    let Ok(mut stmt) = conn.prepare_cached(
        &format!(
            "SELECT id, decision, COALESCE(type,'decision') FROM decisions WHERE status = 'active' AND COALESCE(retention_class,'operational') = 'durable' \
             AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) \
             AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')){scope} \
             ORDER BY CASE WHEN type IN ('constraint','policy','rule','convention','contract','preference') THEN 0 ELSE 1 END, julianday(created_at) ASC, id ASC"
        ),
    ) else {
        return (String::new(), 0);
    };
    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();
    let ids: Vec<i64> = rows.iter().map(|row| row.0).collect();
    let allow = super::capsules::boot_scope_allowlist(conn, "decision", &ids);
    let rows: Vec<(i64, String, String)> = rows
        .into_iter()
        .filter(|(id, ..)| super::capsules::keep_boot_id(&allow, *id))
        .collect();
    if rows.is_empty() {
        return (String::new(), 0);
    }
    let total = rows.len();
    let omitted = total.saturating_sub(BOOT_CONSTRAINTS_MAX);
    let mut lines: Vec<String> = rows
        .into_iter()
        .take(BOOT_CONSTRAINTS_MAX)
        .map(|(id, text, kind)| format!("- [{kind} d{id}] {text}"))
        .collect();
    if omitted > 0 {
        lines.push(format!(
            "- ({omitted} more durable decisions not shown; query profile=map for the full set)"
        ));
    }
    (format!("## Constraints\n{}", lines.join("\n")), omitted)
}
struct BootCompileGuard;
impl Drop for BootCompileGuard {
    fn drop(&mut self) {
        super::capsules::set_boot_owner(None);
        super::capsules::set_boot_paths(&[]);
    }
}

/// Owner-scoped boot: in team mode every capsule query is filtered to the
/// caller's rows so one owner's messages, tasks, locks, feed and decisions
/// never appear in another owner's brief.
pub fn compile_for_owner(
    conn: &Connection,
    home: &Path,
    agent: &str,
    max_tokens: usize,
    owner: Option<i64>,
    paths: &[String],
) -> BootResult {
    super::capsules::set_boot_owner(owner);
    super::capsules::set_boot_paths(paths);
    let _guard = BootCompileGuard;
    compile(conn, home, agent, max_tokens)
}
pub fn compile(conn: &Connection, home: &Path, agent: &str, max_tokens: usize) -> BootResult {
    let mut items: Vec<ContextItem> = Vec::new();
    let (identity_text, _) = build_identity_capsule(conn);
    if !identity_text.is_empty() {
        items.push(ContextItem::new(
            "identity",
            format!("## Identity\n{identity_text}"),
            1.0,
        ));
    }
    // Self-contained base: durable decisions are delivered on every boot,
    // independent of the delta cursor. A fresh context after compaction has
    // none of the earlier deliveries, so "since last boot" can never be the
    // only route to a constraint. When the capsule must be bounded, the cut is
    // stated, never silent.
    let (constraints_text, constraints_omitted) = build_constraints_capsule(conn);
    if !constraints_text.is_empty() {
        items.push(ContextItem::new("## Constraints", constraints_text, 0.99));
    }
    let _ = constraints_omitted;
    let (delta_text, _, _delta_freshness) = build_delta_capsule(conn, agent);
    if !delta_text.is_empty() {
        let sections: Vec<(&str, f64)> = vec![
            ("CONFLICTS:", 0.90),
            ("## Pending Tasks", 0.75),
            ("## Your Active Tasks", 0.80),
            ("## Active Focus", 0.85),
            ("## Pending Messages", 0.95),
            ("## Active Locks", 0.70),
            ("## Active Agents", 0.60),
            ("Recent decisions:", 0.50),
            ("New decisions:", 0.55),
            ("New knowledge:", 0.45),
            ("## Feed", 0.40),
            ("Activity since last boot:", 0.30),
        ];
        let remaining_delta = delta_text.as_str();
        let mut matched_any = false;
        for (header, priority) in &sections {
            if let Some(start) = remaining_delta.find(header) {
                let content_start = start;
                let after_header = start + header.len();
                let end = remaining_delta[after_header..]
                    .find("\n\n")
                    .map(|p| after_header + p)
                    .unwrap_or(remaining_delta.len());
                let section_text = remaining_delta[content_start..end].trim().to_string();
                if !section_text.is_empty() {
                    items.push(ContextItem::new(header, section_text, *priority));
                    matched_any = true;
                }
            }
        }
        if !matched_any {
            items.push(ContextItem::new(
                "delta",
                format!("## Delta\n{delta_text}"),
                0.70,
            ));
        }
    }
    let mut truth_candidates = rank_candidates(
        fetch_rank_candidates(conn),
        40,
        deterministic_now(conn),
    );
    super::capsules::with_boot_paths(|paths| {
        if paths.is_empty() {
            return;
        }
        let mut decision_ids = Vec::new();
        let mut memory_ids = Vec::new();
        for candidate in &truth_candidates {
            match candidate.source_kind {
                "decision" => decision_ids.push(candidate.source_id),
                "memory" => memory_ids.push(candidate.source_id),
                _ => {}
            }
        }
        let decisions = crate::handlers::recall::explicit_paths_by_target(conn, "decision", &decision_ids)
            .unwrap_or_default();
        let memories = crate::handlers::recall::explicit_paths_by_target(conn, "memory", &memory_ids)
            .unwrap_or_default();
        truth_candidates.retain(|candidate| {
            let map = match candidate.source_kind {
                "decision" => &decisions,
                "memory" => &memories,
                _ => return true,
            };
            crate::handlers::recall::read_path_sets(
                paths,
                map.get(&candidate.source_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            )
        });
    });
    truth_candidates.truncate(boot_rank_top_n());
    if !truth_candidates.is_empty() {
        items.push(ContextItem::new(
            "## TRUTH",
            "## TRUTH\nSigils: FACT! confirmed; FACT? unconfirmed; FACT~ disputed.".to_string(),
            0.95,
        ));
    }
    for candidate in truth_candidates {
        items.push(ContextItem::from_ranked_candidate(candidate));
    }
    record_boot(conn, agent);
    items.sort_by(|a, b| {
        let sa = stability_for_item(a);
        let sb = stability_for_item(b);
        sb.cmp(&sa)
            .then_with(|| {
                b.utility
                    .partial_cmp(&a.utility)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                b.priority
                    .partial_cmp(&a.priority)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.text.cmp(&b.text))
    });
    let packed = pack_context_items(&items, max_tokens, boot_source_token_bounds());
    let admitted = packed.admitted;
    let rejected = packed.rejected;
    let assembled_parts = packed.assembled_parts;
    let assembled = assembled_parts.join("\n\n");
    let token_estimate = estimate_tokens(&assembled);
    let raw_baseline = estimate_raw_baseline(conn, home);
    let saved = raw_baseline.saturating_sub(token_estimate);
    let percent = if raw_baseline > 0 {
        (saved * 100) / raw_baseline
    } else {
        0
    };
    if raw_baseline > 0 {
        let _ = crate::handlers::log_event(
            conn,
            "boot_savings",
            json!({"agent":agent,"served":token_estimate,"baseline":raw_baseline,"saved":saved,"percent":percent,
"admitted":admitted.len(),"rejected":rejected.len()}),
            "rust-daemon",
        );
    }
    BootResult {
        boot_prompt: assembled,
        token_estimate,
        // Accounting: this is D_read against a *raw dump* baseline (sum of all
        // active memory/decision text), in chars/4 estimates, not model tokens.
        // See docs/internal/accounting/token-savings-ledger.md.
        savings: json!({"rawBaseline":raw_baseline,"served":token_estimate,"saved":saved,"percent":percent,
            "quantity":"D_read","baseline":"raw_dump_all_active_text","unit":"chars_div_4_estimate","tokenizer":null,"reasoning_tokens":"unobserved"}),
        capsules: admitted,
    }
}

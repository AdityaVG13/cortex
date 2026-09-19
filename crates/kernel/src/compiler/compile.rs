use super::*;
use crate::handlers::estimate_tokens;
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;

#[path = "compile/constraints.rs"]
mod constraints;
use constraints::{build_constraints_capsule, ranked_facts_unavailable, source_ids};

fn deterministic_now(conn: &Connection) -> chrono::DateTime<chrono::Utc> {
    super::stored_max_timestamp(conn)
        .and_then(|ts| super::parse_timestamp(Some(&ts)))
        .unwrap_or_else(chrono::Utc::now)
}

const DELTA_SECTIONS: &[(&str, u8, f64)] = &[
    ("CONFLICTS:", 85, 0.90),
    ("## Pending Tasks", 75, 0.75),
    ("## Your Active Tasks", 74, 0.80),
    ("## Active Focus", 73, 0.85),
    ("## Pending Messages", 60, 0.95),
    ("## Active Locks", 55, 0.70),
    ("## Active Agents", 50, 0.60),
    ("Recent decisions:", 45, 0.50),
    ("New decisions:", 44, 0.55),
    ("New knowledge:", 43, 0.45),
    ("## Feed", 30, 0.40),
    ("Activity since last boot:", 10, 0.30),
];

fn stability_for_item(item: &ContextItem) -> u8 {
    if item.name.starts_with("ranked:") {
        return 94;
    }
    DELTA_SECTIONS
        .iter()
        .find(|(name, _, _)| *name == item.name)
        .map(|(_, stability, _)| *stability)
        .or(match item.name.as_str() {
            "identity" => Some(100),
            "## Constraints" => Some(99),
            "## TRUTH" => Some(95),
            "delta" => Some(50),
            _ => None,
        })
        .unwrap_or(50)
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

fn push_nonempty(items: &mut Vec<ContextItem>, name: &'static str, text: String, priority: f64) {
    if !text.is_empty() {
        items.push(ContextItem::new(name, text, priority));
    }
}

fn append_delta_sections(items: &mut Vec<ContextItem>, delta_text: &str) {
    if delta_text.is_empty() {
        return;
    }
    let mut matched_any = false;
    for (header, _, priority) in DELTA_SECTIONS {
        if let Some(start) = delta_text.find(header) {
            let after_header = start + header.len();
            let end = delta_text[after_header..]
                .find("\n\n")
                .map(|p| after_header + p)
                .unwrap_or(delta_text.len());
            let section_text = delta_text[start..end].trim().to_string();
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

fn append_truth_items(conn: &Connection, items: &mut Vec<ContextItem>) {
    let mut truth_candidates = match fetch_rank_candidates(conn) {
        Ok(candidates) => rank_candidates(candidates, 40, deterministic_now(conn)),
        Err(_) => {
            items.push(ranked_facts_unavailable());
            Vec::new()
        }
    };
    let mut truth_scope_failed = false;
    super::capsules::with_boot_paths(|paths| {
        if paths.is_empty() || truth_candidates.is_empty() {
            return;
        }
        let decision_ids = source_ids(&truth_candidates, "decision");
        let memory_ids = source_ids(&truth_candidates, "memory");
        // Read law treats missing paths as unscoped (visible). A failed
        // lookup is not "no paths": it would admit every foreign project
        // into a path-scoped boot, the same leak store already fail-closes.
        let Ok(decisions) =
            crate::handlers::recall::explicit_paths_by_target(conn, "decision", &decision_ids)
        else {
            truth_scope_failed = true;
            truth_candidates.clear();
            return;
        };
        let Ok(memories) =
            crate::handlers::recall::explicit_paths_by_target(conn, "memory", &memory_ids)
        else {
            truth_scope_failed = true;
            truth_candidates.clear();
            return;
        };
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
    if truth_scope_failed {
        items.push(ranked_facts_unavailable());
        return;
    }
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
}

fn sort_boot_items(items: &mut [ContextItem]) {
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
}

fn finish_boot(
    conn: &Connection,
    home: &Path,
    agent: &str,
    items: &[ContextItem],
    max_tokens: usize,
) -> BootResult {
    let packed = pack_context_items(items, max_tokens, boot_source_token_bounds());
    let admitted = packed.admitted;
    let rejected = packed.rejected;
    let assembled = packed.assembled_parts.join("\n\n");
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
            json!({"agent":agent,"served":token_estimate,"baseline":raw_baseline,"saved":saved,"percent":percent,"admitted":admitted.len(),"rejected":rejected.len()}),
            "rust-daemon",
        );
    }
    // Accounting: this is D_read against a *raw dump* baseline (sum of all
    // active memory/decision text), in chars/4 estimates, not model tokens.
    // See docs/internal/accounting/token-savings-ledger.md.
    BootResult {
        boot_prompt: assembled,
        token_estimate,
        savings: json!({"rawBaseline":raw_baseline,"served":token_estimate,"saved":saved,"percent":percent,"quantity":"D_read","baseline":"raw_dump_all_active_text","unit":"chars_div_4_estimate","tokenizer":null,"reasoning_tokens":"unobserved"}),
        capsules: admitted,
    }
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
    push_nonempty(&mut items, "## Constraints", constraints_text, 0.99);
    let _ = constraints_omitted;
    let (delta_text, _, _delta_freshness) = build_delta_capsule(conn, agent);
    append_delta_sections(&mut items, &delta_text);
    append_truth_items(conn, &mut items);
    record_boot(conn, agent);
    sort_boot_items(&mut items);
    finish_boot(conn, home, agent, &items, max_tokens)
}

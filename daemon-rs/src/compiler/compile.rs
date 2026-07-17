// SPDX-License-Identifier: MIT
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::env;
use std::path::Path;

use crate::handlers::{estimate_tokens, estimate_tokens_from_chars};


use super::*;
/// Compile the boot prompt for an agent within a token budget.
///
/// Prompt Compiler Pipeline (v3 -- score-adaptive context packing):
///  1. Gather all context items with priority scores
///  2. Sort by utility (priority / token_cost) -- best bang-per-token first
///  3. Pack within budget using score-adaptive truncation when score variance exists
///  4. Record admitted vs rejected for observability
///  5. Return prompt with compilation metadata and savings
pub fn compile(conn: &Connection, home: &Path, agent: &str, max_tokens: usize) -> BootResult {
    // ── 1. Gather context items with priorities ─────────────────────────────

    let mut items: Vec<ContextItem> = Vec::new();

    // Identity capsule: must-have (priority 1.0)
    let (identity_text, _) = build_identity_capsule(conn);
    if !identity_text.is_empty() {
        items.push(ContextItem::new(
            "identity",
            format!("## Identity\n{identity_text}"),
            1.0,
        ));
    }

    // Delta capsule: broken into sub-items with individual priorities
    let (delta_text, _, _delta_freshness) = build_delta_capsule(conn, agent);
    if !delta_text.is_empty() {
        // Split delta into sections, each scored independently
        let sections: Vec<(&str, f64)> = vec![
            ("## Pending Messages", 0.95),  // Messages from other agents = urgent
            ("## Active Agents", 0.60),     // Who's online = coordination
            ("## Active Locks", 0.70),      // Locks = collision prevention
            ("## Feed", 0.40),              // Feed = nice context
            ("## Pending Tasks", 0.75),     // Task board = actionable
            ("## Your Active Tasks", 0.80), // Your tasks = high priority
            ("CONFLICTS:", 0.90),           // Conflicts = must resolve
            ("## Active Focus", 0.85),      // Focus scope = context boundary
            ("New decisions:", 0.55),       // Recent decisions = orientation
            ("New knowledge:", 0.45),       // New memories
            ("Activity since last boot:", 0.30), // Activity summary = low value
            ("Recent decisions:", 0.50),    // First-boot orientation
        ];

        // Try to split delta into scored sub-sections
        let remaining_delta = delta_text.as_str();
        let mut matched_any = false;

        for (header, priority) in &sections {
            if let Some(start) = remaining_delta.find(header) {
                // Find end: next section header or end of string
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

        // Fallback: if no sections matched, treat delta as one block
        if !matched_any {
            items.push(ContextItem::new(
                "delta",
                format!("## Delta\n{delta_text}"),
                0.70,
            ));
        }
    }

    // ── 2. Record boot ──────────────────────────────────────────────────────
    for candidate in rank_candidates(fetch_rank_candidates(conn), boot_rank_top_n(), Utc::now()) {
        items.push(ContextItem::from_ranked_candidate(candidate));
    }

    record_boot(conn, agent);

    // ── 3. Sort by utility (priority / token_cost) descending ───────────────
    items.sort_by(|a, b| {
        b.utility
            .partial_cmp(&a.utility)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ── 4. Score-adaptive budget packing ────────────────────────────────────
    let packed = pack_context_items(&items, max_tokens, boot_source_token_bounds());
    let admitted = packed.admitted;
    let rejected = packed.rejected;
    let assembled_parts = packed.assembled_parts;

    let assembled = assembled_parts.join("\n\n");
    let token_estimate = estimate_tokens(&assembled);

    // ── 5. Savings and observability ────────────────────────────────────────
    let raw_baseline = estimate_raw_baseline(conn, home);
    let saved = raw_baseline.saturating_sub(token_estimate);
    let percent = if raw_baseline > 0 {
        (saved * 100) / raw_baseline
    } else {
        0
    };

    // Only record savings when baseline is meaningful (skip empty-DB boots)
    if raw_baseline > 0 {
        let _ = conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
            params![
                "boot_savings",
                serde_json::to_string(&json!({
                    "agent": agent,
                    "served": token_estimate,
                    "baseline": raw_baseline,
                    "saved": saved,
                    "percent": percent,
                    "admitted": admitted.len(),
                    "rejected": rejected.len()
                }))
                .unwrap_or_default(),
                "rust-daemon"
            ],
        );
    }

    BootResult {
        boot_prompt: assembled,
        token_estimate,
        savings: json!({
            "rawBaseline": raw_baseline,
            "served": token_estimate,
            "saved": saved,
            "percent": percent
        }),
        capsules: admitted,
    }
}

// Dead code removed: find_memory_dir, read_memory_files, read_lessons
// (indexer.rs has its own implementation; these were ported but unused)

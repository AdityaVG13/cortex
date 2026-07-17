use super::*;
use crate::handlers::estimate_tokens;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json::json;
use std::path::Path;
pub fn compile(conn: &Connection, home: &Path, agent: &str, max_tokens: usize) -> BootResult {
    let mut items: Vec<ContextItem> = Vec::new();
    let (identity_text, _) = build_identity_capsule(conn);
    if !identity_text.is_empty() {
        items.push(ContextItem::new("identity", format!("## Identity\n{identity_text}"), 1.0));
    }
    let (delta_text, _, _delta_freshness) = build_delta_capsule(conn, agent);
    if !delta_text.is_empty() {
        let sections: Vec<(&str, f64)> = vec![
            ("## Pending Messages", 0.95),
            ("## Active Agents", 0.60),
            ("## Active Locks", 0.70),
            ("## Feed", 0.40),
            ("## Pending Tasks", 0.75),
            ("## Your Active Tasks", 0.80),
            ("CONFLICTS:", 0.90),
            ("## Active Focus", 0.85),
            ("New decisions:", 0.55),
            ("New knowledge:", 0.45),
            ("Activity since last boot:", 0.30),
            ("Recent decisions:", 0.50),
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
            items.push(ContextItem::new("delta", format!("## Delta\n{delta_text}"), 0.70));
        }
    }
    for candidate in rank_candidates(fetch_rank_candidates(conn), boot_rank_top_n(), Utc::now()) {
        items.push(ContextItem::from_ranked_candidate(candidate));
    }
    record_boot(conn, agent);
    items.sort_by(|a, b| b.utility.partial_cmp(&a.utility).unwrap_or(std::cmp::Ordering::Equal));
    let packed = pack_context_items(&items, max_tokens, boot_source_token_bounds());
    let admitted = packed.admitted;
    let rejected = packed.rejected;
    let assembled_parts = packed.assembled_parts;
    let assembled = assembled_parts.join("\n\n");
    let token_estimate = estimate_tokens(&assembled);
    let raw_baseline = estimate_raw_baseline(conn, home);
    let saved = raw_baseline.saturating_sub(token_estimate);
    let percent = if raw_baseline > 0 { (saved * 100) / raw_baseline } else { 0 };
    if raw_baseline > 0 {
        let _ = conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
            params![
                "boot_savings",
                serde_json::to_string(
                    &json!({"agent":agent,"served":token_estimate,"baseline":raw_baseline,"saved":saved,"percent":percent,
"admitted":admitted.len(),"rejected":rejected.len()})
                )
                .unwrap_or_default(),
                "rust-daemon"
            ],
        );
    }
    BootResult {
        boot_prompt: assembled,
        token_estimate,
        savings: json!({"rawBaseline":raw_baseline,"served":token_estimate,"saved":saved,"percent":percent}),
        capsules: admitted,
    }
}

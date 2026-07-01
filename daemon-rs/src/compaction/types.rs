// SPDX-License-Identifier: MIT
use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;


use super::*;
// ─── Constants ──────────────────────────────────────────────────────────────

/// Non-boot events older than this are deleted.
pub(crate) const EVENT_RETENTION_DAYS: i64 = 14;

/// Raw boot savings rows older than this are compacted into a single rollup row.
/// The dashboard only needs recent raw points, while all-time totals are preserved
/// via `boot_savings_rollup`.
pub(crate) const BOOT_SAVINGS_RETENTION_DAYS: i64 = 45;

/// Only VACUUM when SQLite reports enough reclaimable pages to justify the IO.
pub(crate) const VACUUM_FREELIST_THRESHOLD_PAGES: i64 = 100;

/// Archived entries older than this have their text stripped (metadata kept).
pub(crate) const ARCHIVED_TEXT_RETENTION_DAYS: i64 = 90;

/// Feedback signals older than this are aggregated into summaries.
pub(crate) const FEEDBACK_AGGREGATION_DAYS: i64 = 60;

/// Roll analytics-heavy savings events older than this into compact hourly rows.
pub(crate) const SAVINGS_EVENT_ROLLUP_RETENTION_DAYS: i64 = 7;
/// Keep rolled-up savings analytics bounded; /savings only reads the recent window.
pub(crate) const EVENT_SAVINGS_ROLLUP_RETENTION_DAYS: i64 = 120;

/// Elevated-pressure storage governor soft limit (no hard failures, compaction only).
pub const STORAGE_SOFT_LIMIT_BYTES: i64 = 256 * 1024 * 1024; // 256MB
/// Critical-pressure storage governor hard limit (triggers aggressive safe compaction).
pub const STORAGE_HARD_LIMIT_BYTES: i64 = 512 * 1024 * 1024; // 512MB

/// Under critical pressure, compact events more aggressively.
pub(crate) const AGGRESSIVE_EVENT_RETENTION_DAYS: i64 = 3;
/// Under critical pressure, compact boot savings history more aggressively.
pub(crate) const AGGRESSIVE_BOOT_SAVINGS_RETENTION_DAYS: i64 = 14;
/// Under critical pressure, compact archived text sooner.
pub(crate) const AGGRESSIVE_ARCHIVED_TEXT_RETENTION_DAYS: i64 = 30;
/// Under critical pressure, aggregate feedback sooner.
pub(crate) const AGGRESSIVE_FEEDBACK_AGGREGATION_DAYS: i64 = 14;
/// Under critical pressure, roll savings events even sooner.
pub(crate) const AGGRESSIVE_SAVINGS_EVENT_ROLLUP_RETENTION_DAYS: i64 = 2;
/// Under critical pressure, keep a shorter event rollup history to reclaim space faster.
pub(crate) const AGGRESSIVE_EVENT_SAVINGS_ROLLUP_RETENTION_DAYS: i64 = 45;
/// Keep benchmark artifacts only briefly in production databases.
pub(crate) const BENCHMARK_RETENTION_DAYS: i64 = 2;
/// Tighten benchmark retention further under critical pressure.
pub(crate) const AGGRESSIVE_BENCHMARK_RETENTION_DAYS: i64 = 1;

/// Canonical source-agent prefix emitted by benchmark harnesses.
///
/// Keep this broad enough to match both modern namespaced agents
/// (`amb-cortex::<run>`) and legacy plain labels (`amb-cortex`).
pub const BENCHMARK_SOURCE_AGENT_PREFIX: &str = "amb-cortex";

/// Non-boot event volume triggers compaction even when DB file size is moderate.
pub const EVENT_NONBOOT_SOFT_LIMIT_ROWS: i64 = 72_000;
/// Critical non-boot event pressure threshold.
pub const EVENT_NONBOOT_HARD_LIMIT_ROWS: i64 = 120_000;
/// Keep newest non-boot rows at or under this level during normal governor runs.
pub(crate) const EVENT_NONBOOT_SOFT_KEEP_ROWS: i64 = 52_000;
/// Keep newest non-boot rows at or under this level during critical pressure runs.
pub(crate) const EVENT_NONBOOT_HARD_KEEP_ROWS: i64 = 28_000;
/// Startup governor mode should avoid single huge DELETE statements that hold
/// the write lock for too long while the daemon is still coming online.
pub(crate) const STARTUP_EVENT_PRUNE_BATCH_ROWS: i64 = 8_000;

/// Per-event-type row caps to prevent high-frequency streams from dominating storage.
pub(crate) const EVENT_TYPE_SOFT_CAPS: &[(&str, i64)] = &[
    ("agent_boot", 4_000),
    ("boot_savings", 6_000),
    ("store_savings", 10_000),
    ("tool_call_savings", 10_000),
    ("decision_stored", 18_000),
    ("decision_supersede", 10_000),
    ("decision_refine_pending", 10_000),
    ("decision_agreement_merge", 8_000),
    ("decision_truncated", 8_000),
    ("recall_query", 14_000),
    ("merge", 6_000),
    ("decision_conflict", 6_000),
    ("decision_rejected_duplicate", 6_000),
    ("decision_resolve", 6_000),
    ("forget", 3_000),
    ("diary_write", 3_000),
];

/// More aggressive caps used under critical pressure.
pub(crate) const EVENT_TYPE_HARD_CAPS: &[(&str, i64)] = &[
    ("agent_boot", 1_500),
    ("boot_savings", 2_500),
    ("store_savings", 4_000),
    ("tool_call_savings", 4_000),
    ("decision_stored", 8_000),
    ("decision_supersede", 4_000),
    ("decision_refine_pending", 4_000),
    ("decision_agreement_merge", 3_000),
    ("decision_truncated", 3_000),
    ("recall_query", 6_000),
    ("merge", 2_000),
    ("decision_conflict", 2_000),
    ("decision_rejected_duplicate", 2_000),
    ("decision_resolve", 2_000),
    ("forget", 1_000),
    ("diary_write", 1_000),
];


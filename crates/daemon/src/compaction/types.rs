pub(crate) const EVENT_RETENTION_DAYS: i64 = 14;
pub(crate) const BOOT_SAVINGS_RETENTION_DAYS: i64 = 45;
pub(crate) const VACUUM_FREELIST_THRESHOLD_PAGES: i64 = 100;
pub(crate) const ARCHIVED_TEXT_RETENTION_DAYS: i64 = 90;
pub(crate) const FEEDBACK_AGGREGATION_DAYS: i64 = 60;
pub(crate) const SAVINGS_EVENT_ROLLUP_RETENTION_DAYS: i64 = 7;
pub(crate) const EVENT_SAVINGS_ROLLUP_RETENTION_DAYS: i64 = 120;
pub const STORAGE_SOFT_LIMIT_BYTES: i64 = 256 * 1024 * 1024;
pub const STORAGE_HARD_LIMIT_BYTES: i64 = 512 * 1024 * 1024;
pub(crate) const AGGRESSIVE_EVENT_RETENTION_DAYS: i64 = 3;
pub(crate) const AGGRESSIVE_BOOT_SAVINGS_RETENTION_DAYS: i64 = 14;
pub(crate) const AGGRESSIVE_ARCHIVED_TEXT_RETENTION_DAYS: i64 = 30;
pub(crate) const AGGRESSIVE_FEEDBACK_AGGREGATION_DAYS: i64 = 14;
pub(crate) const AGGRESSIVE_SAVINGS_EVENT_ROLLUP_RETENTION_DAYS: i64 = 2;
pub(crate) const AGGRESSIVE_EVENT_SAVINGS_ROLLUP_RETENTION_DAYS: i64 = 45;
pub(crate) const BENCHMARK_RETENTION_DAYS: i64 = 2;
pub(crate) const AGGRESSIVE_BENCHMARK_RETENTION_DAYS: i64 = 1;
pub const BENCHMARK_SOURCE_AGENT_PREFIX: &str = "amb-cortex";
pub const EVENT_NONBOOT_SOFT_LIMIT_ROWS: i64 = 72_000;
pub const EVENT_NONBOOT_HARD_LIMIT_ROWS: i64 = 120_000;
pub(crate) const EVENT_NONBOOT_SOFT_KEEP_ROWS: i64 = 52_000;
pub(crate) const EVENT_NONBOOT_HARD_KEEP_ROWS: i64 = 28_000;
pub(crate) const STARTUP_EVENT_PRUNE_BATCH_ROWS: i64 = 8_000;
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

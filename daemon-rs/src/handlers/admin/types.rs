// SPDX-License-Identifier: MIT
use serde::Deserialize;

pub(crate) const OWNER_TABLES: &[&str] = &[
    "memories",
    "decisions",
    "memory_clusters",
    "recall_feedback",
    "sessions",
    "locks",
    "tasks",
    "messages",
    "feed",
    "feed_acks",
    "activities",
    "focus_sessions",
];

pub(crate) const VISIBILITY_TABLES: &[&str] = &["memories", "decisions", "memory_clusters", "feed"];

pub(crate) fn is_allowed_table(table: &str, allowlist: &[&str]) -> bool {
    allowlist.contains(&table)
}

// ─── Request bodies ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UserAddBody {
    pub username: String,
    pub display_name: Option<String>,
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub struct UsernameBody {
    pub username: String,
}

#[derive(Deserialize)]
pub struct TeamCreateBody {
    pub name: String,
}

#[derive(Deserialize)]
pub struct TeamMemberBody {
    pub team_name: String,
    pub username: String,
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub struct TeamRemoveMemberBody {
    pub team_name: String,
    pub username: String,
}

#[derive(Deserialize)]
pub struct AssignOwnerBody {
    pub from_user: Option<String>,
    pub to_user: String,
    pub table: Option<String>,
}

#[derive(Deserialize)]
pub struct SetVisibilityBody {
    pub table: String,
    pub ids: Vec<i64>,
    pub visibility: String,
}

#[derive(Deserialize)]
pub struct ArchiveBody {
    pub table: String,
    pub ids: Vec<i64>,
}

// ─── User Management ────────────────────────────────────────────────────────

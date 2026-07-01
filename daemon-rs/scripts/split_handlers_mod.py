#!/usr/bin/env python3
"""Extract auth.rs, redaction.rs, event_log.rs from handlers/mod.rs."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MOD = ROOT / "src/handlers/mod.rs"
SPDX = "// SPDX-License-Identifier: MIT\n"

lines = MOD.read_text(encoding="utf-8").splitlines(keepends=True)


def sl(start: int, end: int) -> str:
    return "".join(lines[start - 1 : end])


REDACTION = SPDX + """use regex::Regex;
use std::sync::OnceLock;

static BEARER_REDACTION_RE: OnceLock<Option<Regex>> = Once all();
static HASH_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static CREDENTIAL_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();

""" + sl(135, 153).replace("pub(super) ", "pub(crate) ")

AUTH_BODY = (
    sl(31, 35)
    + "\n"
    + sl(37, 38)
    + sl(44, 44)
    + "\n"
    + sl(155, 516)
    + sl(522, 587)
    + sl(588, 724)
)

AUTH = (
    SPDX
    + """use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use std::net::IpAddr;

use crate::budgets::{BudgetDecision, BudgetEndpoint};
use crate::rate_limit::RequestClass;
use crate::state::RuntimeState;

use super::{json_response, now_iso};
use super::event_log::log_event;

"""
    + AUTH_BODY
    + """

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn create_sessions_table(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent TEXT NOT NULL,
                owner_id INTEGER,
                session_id TEXT NOT NULL,
                project TEXT,
                files_json TEXT NOT NULL DEFAULT '[]',
                description TEXT,
                started_at TEXT NOT NULL,
                last_heartbeat TEXT NOT NULL,
                expires_at TEXT,
                UNIQUE(agent),
                UNIQUE(owner_id, agent)
            );",
        )
        .expect("create sessions table");
    }

"""
    + sl(1137, 1161)
    + sl(1163, 1191)
    + sl(1222, 1227)
    + sl(1538, 1649)
    + "}\n"
)

EVENT_LOG_BODY = sl(39, 43) + sl(49, 82) + "\n" + sl(726, 1078).replace("pub(super) ", "pub(crate) ")

EVENT_LOG = (
    SPDX
    + """use chrono::Utc;
use rusqlite;
use serde_json::{json, Value};

use super::truncate_chars;

"""
    + EVENT_LOG_BODY
    + """

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

"""
    + sl(1229, 1536)
    + "}\n"
)

MOD_NEW = (
    SPDX
    + """pub mod admin;
pub mod auth;
pub mod boot;
pub mod conductor;
pub mod diary;
pub mod event_log;
pub mod events;
pub mod export;
pub mod feed;
pub mod feedback;
pub mod health;
pub mod mcp;
pub mod mutate;
pub mod recall;
pub mod redaction;
pub mod store;

pub use auth::{
    client_ip, ensure_admin, ensure_auth, ensure_auth_rated, ensure_auth_rated_for_class,
    ensure_auth_with_caller, ensure_auth_with_caller_rated, ensure_auth_with_caller_rated_for_class,
    ensure_endpoint_budget, ensure_events_stream_auth, ensure_ssrf_protection, extract_auth_token,
    log_budget_rejection, register_agent_presence, register_agent_presence_from_headers,
    resolve_caller_id, resolve_source_identity, runtime_token_matches, SourceIdentity,
    CORTEX_PEER_IP_HEADER,
};
pub use event_log::log_event;
pub use redaction::redact_secrets;

// ─── Shared helpers ──────────────────────────────────────────────────────────

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{NaiveDateTime, TimeZone, Utc};
use serde_json::{json, Value};

"""
    + sl(84, 133)
    + "\n"
    + sl(1080, 1110)
    + """

#[cfg(test)]
mod tests {
    use super::*;

"""
    + sl(1193, 1220)
    + sl(1630, 1640)
    + "}\n"
)

# fix typo in REDACTION
REDACTION = REDACTION.replace("OnceLock::all()", "OnceLock::new()")

(ROOT / "src/handlers/redaction.rs").write_text(REDACTION, encoding="utf-8")
(ROOT / "src/handlers/auth.rs").write_text(AUTH, encoding="utf-8")
(ROOT / "src/handlers/event_log.rs").write_text(EVENT_LOG, encoding="utf-8")
MOD.write_text(MOD_NEW, encoding="utf-8")
print("split handlers/mod.rs -> auth.rs, redaction.rs, event_log.rs")

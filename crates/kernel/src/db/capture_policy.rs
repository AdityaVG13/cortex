//! Inspectable capture policy: per scope, capture is `active`, `paused`
//! (nothing new is retained, deliveries continue) or `stopped` (no capture,
//! no automatic delivery). The state is data the control center can show and
//! flip; hooks consult it before every capture and never infer it.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS capture_policy (
  scope TEXT PRIMARY KEY,
  state TEXT NOT NULL CHECK(state IN ('active','paused','stopped')),
  reason TEXT,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
"#;

pub const DEFAULT_SCOPE: &str = "*";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    Active,
    Paused,
    Stopped,
}

impl CaptureState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "active" | "resume" | "on" => Some(Self::Active),
            "paused" | "pause" => Some(Self::Paused),
            "stopped" | "stop" | "off" => Some(Self::Stopped),
            _ => None,
        }
    }
    pub fn allows_capture(self) -> bool {
        self == Self::Active
    }
    pub fn allows_delivery(self) -> bool {
        self != Self::Stopped
    }
}

pub fn ensure(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(DDL)
}

/// Effective state for a scope: the exact scope row wins, then the global
/// row, then `active`.
pub fn state_for(conn: &Connection, scope: &str) -> CaptureState {
    if ensure(conn).is_err() {
        return CaptureState::Active;
    }
    let lookup = |s: &str| -> Option<CaptureState> {
        conn.query_row(
            "SELECT state FROM capture_policy WHERE scope = ?1",
            params![s],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|v| CaptureState::parse(&v))
    };
    if !scope.is_empty() && scope != DEFAULT_SCOPE {
        if let Some(s) = lookup(scope) {
            return s;
        }
    }
    lookup(DEFAULT_SCOPE).unwrap_or(CaptureState::Active)
}

pub fn set_state(
    conn: &Connection,
    scope: &str,
    state: CaptureState,
    reason: Option<&str>,
) -> rusqlite::Result<()> {
    ensure(conn)?;
    let scope = if scope.trim().is_empty() {
        DEFAULT_SCOPE
    } else {
        scope.trim()
    };
    conn.execute(
        "INSERT INTO capture_policy (scope, state, reason) VALUES (?1, ?2, ?3)
         ON CONFLICT(scope) DO UPDATE SET state = excluded.state, reason = excluded.reason, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![scope, state.as_str(), reason],
    )?;
    Ok(())
}

pub fn inspect(conn: &Connection) -> Value {
    if ensure(conn).is_err() {
        return json!({"global": "active", "scopes": []});
    }
    let mut scopes = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT scope, state, reason, updated_at FROM capture_policy ORDER BY scope")
    {
        if let Ok(rows) = stmt.query_map([], |r| Ok(json!({"scope": r.get::<_, String>(0)?, "state": r.get::<_, String>(1)?, "reason": r.get::<_, Option<String>>(2)?, "updated_at": r.get::<_, String>(3)?}))) {
            scopes.extend(rows.flatten());
        }
    }
    json!({"global": state_for(conn, DEFAULT_SCOPE).as_str(), "scopes": scopes})
}

// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::Path;

use super::{ensure_auth, json_error, json_response, log_event, now_iso, resolve_source_identity};
use crate::state::RuntimeState;

// ─── Request type ─────────────────────────────────────────────────────────────

#[derive(Clone, Deserialize)]
pub struct DiaryRequest {
    pub accomplished: Option<String>,
    #[serde(rename = "nextSteps")]
    pub next_steps: Option<String>,
    pub decisions: Option<String>,
    /// Legacy alias for decisions
    #[serde(rename = "keyDecisions")]
    pub key_decisions: Option<String>,
    pub pending: Option<String>,
    #[serde(rename = "knownIssues")]
    pub known_issues: Option<String>,
}

impl DiaryRequest {
    pub(crate) fn decisions_text(&self) -> Option<&str> {
        self.decisions.as_deref().or(self.key_decisions.as_deref())
    }
}

// ─── POST /diary ──────────────────────────────────────────────────────────────

pub async fn handle_diary(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<DiaryRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let source = resolve_source_identity(&headers, "http");
    match write_diary_entry(&state, &body, &source.agent).await {
        Ok(path) => json_response(
            StatusCode::OK,
            json!({ "written": true, "agent": source.agent, "path": path }),
        ),
        Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &err),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

pub(crate) async fn write_diary_entry(
    state: &RuntimeState,
    body: &DiaryRequest,
    agent: &str,
) -> Result<String, String> {
    let state_path = state.home.join(".claude").join("state.md");

    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    let existing = fs::read_to_string(&state_path).unwrap_or_default();
    let content = build_diary_content(&existing, body);
    fs::write(&state_path, &content).map_err(|e| format!("Failed to write state.md: {e}"))?;

    let conn = state.db.lock().await;
    let _ = log_event(
        &conn,
        "diary_write",
        json!({ "agent": agent, "timestamp": now_iso() }),
        agent,
    );

    Ok(state_path.display().to_string())
}

fn build_diary_content(existing: &str, body: &DiaryRequest) -> String {
    let permanent = extract_section(existing, "## DO NOT REMOVE").unwrap_or_default();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut lines: Vec<String> = vec![format!("# Session State — {today}"), String::new()];

    if !permanent.is_empty() {
        lines.push("## DO NOT REMOVE".to_string());
        lines.push(permanent);
        lines.push(String::new());
    }

    append_section(
        &mut lines,
        "## What Was Done This Session",
        body.accomplished.as_deref(),
    );
    append_section(&mut lines, "## Next Session", body.next_steps.as_deref());
    append_section(&mut lines, "## Pending", body.pending.as_deref());
    append_section(&mut lines, "## Known Issues", body.known_issues.as_deref());

    if let Some(text) = body.decisions_text() {
        append_section(&mut lines, "## Key Decisions", Some(text));
    } else if let Some(content) = extract_section(existing, "## Key Decisions") {
        lines.push("## Key Decisions".to_string());
        lines.push(content);
        lines.push(String::new());
    }

    lines.join("\n")
}

fn append_section(lines: &mut Vec<String>, header: &str, value: Option<&str>) {
    let Some(text) = value else {
        return;
    };
    let safe = sanitize_markdown(text);
    if safe.is_empty() {
        return;
    }
    lines.push(header.to_string());
    lines.push(safe);
    lines.push(String::new());
}

/// Extract the text body of a markdown section identified by `header`.
/// Returns `None` if the header is not found or the body is empty.
fn extract_section(content: &str, header: &str) -> Option<String> {
    let idx = content.find(header)?;
    let start = idx + header.len();
    let rest = &content[start..];
    let end = rest.find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
    let text = rest[..end].trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Escape any user-provided `##` headers to prevent document structure breakage.
fn sanitize_markdown(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.chars().all(|c| c == '-') && trimmed.len() >= 3 {
                return line.to_string();
            }
            if trimmed.starts_with("##") {
                return format!("<!-- {line} -->");
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(dead_code)]
fn path_to_string(path: &Path) -> String {
    path.display().to_string()
}

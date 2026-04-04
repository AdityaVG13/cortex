use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::fs;

use super::{ensure_auth, json_error, json_response};
use crate::state::RuntimeState;

// ─── Request type ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
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

// ─── POST /diary ──────────────────────────────────────────────────────────────

pub async fn handle_diary(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<DiaryRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let agent = headers
        .get("x-source-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http")
        .to_string();
    let state_path = state.home.join(".claude").join("state.md");

    // Ensure parent directory exists
    if let Some(parent) = state_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to create directory: {e}"),
            );
        }
    }

    // Read existing content
    let existing = fs::read_to_string(&state_path).unwrap_or_default();

    // Preserve permanent sections (## DO NOT REMOVE and its content)
    let permanent = extract_permanent_sections(&existing);

    // Today's date
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let mut lines: Vec<String> = vec![format!("# Session State — {today}"), String::new()];

    // Write permanent sections first
    if !permanent.is_empty() {
        lines.push("## DO NOT REMOVE".to_string());
        lines.push(permanent);
        lines.push(String::new());
    }

    // Dynamic sections — sanitize each field to prevent header injection
    if let Some(ref text) = body.accomplished {
        let safe = sanitize_markdown(text);
        if !safe.is_empty() {
            lines.push("## What Was Done This Session".to_string());
            lines.push(safe);
            lines.push(String::new());
        }
    }

    if let Some(ref text) = body.next_steps {
        let safe = sanitize_markdown(text);
        if !safe.is_empty() {
            lines.push("## Next Session".to_string());
            lines.push(safe);
            lines.push(String::new());
        }
    }

    if let Some(ref text) = body.pending {
        let safe = sanitize_markdown(text);
        if !safe.is_empty() {
            lines.push("## Pending".to_string());
            lines.push(safe);
            lines.push(String::new());
        }
    }

    if let Some(ref text) = body.known_issues {
        let safe = sanitize_markdown(text);
        if !safe.is_empty() {
            lines.push("## Known Issues".to_string());
            lines.push(safe);
            lines.push(String::new());
        }
    }

    // decisions field takes priority over keyDecisions (legacy alias)
    let decisions_text = body.decisions.as_deref().or(body.key_decisions.as_deref());
    if let Some(text) = decisions_text {
        let safe = sanitize_markdown(text);
        if !safe.is_empty() {
            lines.push("## Key Decisions".to_string());
            lines.push(safe);
            lines.push(String::new());
        }
    } else {
        // Preserve existing Key Decisions section if not being overwritten
        let existing_decisions = extract_section(&existing, "## Key Decisions");
        if let Some(content) = existing_decisions {
            lines.push("## Key Decisions".to_string());
            lines.push(content);
            lines.push(String::new());
        }
    }

    let content = lines.join("\n");

    match fs::write(&state_path, &content) {
        Ok(_) => {
            // Log diary_write event
            let conn = state.db.lock().await;
            let _ = super::log_event(
                &conn,
                "diary_write",
                json!({ "agent": agent, "timestamp": super::now_iso() }),
                &agent,
            );
            json_response(StatusCode::OK, json!({ "written": true }))
        }
        Err(e) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to write state.md: {e}"),
        ),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Extract the body of the "## DO NOT REMOVE" section (everything after the
/// header up to, but not including, the next `## ` header or EOF).
fn extract_permanent_sections(content: &str) -> String {
    extract_section(content, "## DO NOT REMOVE").unwrap_or_default()
}

/// Extract the text body of a markdown section identified by `header`.
/// Returns `None` if the header is not found or the body is empty.
fn extract_section(content: &str, header: &str) -> Option<String> {
    let idx = content.find(header)?;
    let start = idx + header.len();
    // Find next ## header or end of string
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
            // Pass through separator lines (---)
            if trimmed.chars().all(|c| c == '-') && trimmed.len() >= 3 {
                return line.to_string();
            }
            // Comment out embedded headers
            if trimmed.starts_with("##") {
                return format!("<!-- {line} -->");
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

//! Model-free capture from tool results. Deterministic parsers only: paths,
//! symbols, exit status, typed checks (tests/typecheck/lint/build). Nothing
//! here explains *why*; a model's explanation stays attributed to the model.
//! The output is a proposal for a deposit, never a durable ack.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Test,
    Typecheck,
    Lint,
    Build,
    Format,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedCheck {
    pub kind: CheckKind,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CapturedFacts {
    pub tool: String,
    pub command: Option<String>,
    pub exit_status: Option<i32>,
    pub paths: Vec<String>,
    pub symbols: Vec<String>,
    pub checks: Vec<TypedCheck>,
    pub error_codes: Vec<String>,
    /// Bytes offered by the host; the retained text is bounded separately.
    pub offered_bytes: usize,
}

impl CapturedFacts {
    /// True when there is something a deposit could stand on.
    pub fn is_material(&self) -> bool {
        self.exit_status.is_some_and(|c| c != 0)
            || !self.checks.is_empty()
            || !self.error_codes.is_empty()
            || (!self.paths.is_empty() && self.command.is_some())
    }
    /// Deterministic one-line statement for a deposit. No adjectives, no
    /// causes: a receipt of what the tool reported.
    pub fn statement(&self) -> String {
        let mut parts = Vec::new();
        if let Some(cmd) = &self.command {
            parts.push(format!("`{}`", truncate(cmd, 120)));
        } else {
            parts.push(format!("{} result", self.tool));
        }
        if let Some(code) = self.exit_status {
            parts.push(format!("exit {code}"));
        }
        for check in &self.checks {
            let verdict = if check.passed { "passed" } else { "failed" };
            match (check.passed_count, check.failed_count) {
                (Some(p), Some(f)) => parts.push(
                    format!("{:?} {verdict} ({p} passed, {f} failed)", check.kind)
                        .to_ascii_lowercase(),
                ),
                _ => parts.push(format!("{:?} {verdict}", check.kind).to_ascii_lowercase()),
            }
        }
        if !self.error_codes.is_empty() {
            parts.push(format!("errors {}", self.error_codes.join(",")));
        }
        if !self.paths.is_empty() {
            parts.push(format!(
                "touching {}",
                self.paths
                    .iter()
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        parts.join("; ")
    }
    /// A stable idempotency key so a re-delivered tool result is not
    /// deposited twice (duplicate-capture avoidance costs no tokens).
    pub fn idempotency_key(&self) -> String {
        let canonical = serde_json::to_string(self).unwrap_or_default();
        format!("capture:{}", crate::traces::content_hash(&canonical))
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// Parse a host tool result. `tool` is the host's tool name (Bash, Edit,
/// Read…), `command` the invocation when known, `output` the observed text.
pub fn parse_tool_result(
    tool: &str,
    command: Option<&str>,
    output: &str,
    exit_status: Option<i32>,
) -> CapturedFacts {
    let mut facts = CapturedFacts {
        tool: tool.to_string(),
        command: command.map(str::to_string),
        exit_status,
        offered_bytes: output.len(),
        ..Default::default()
    };
    let mut paths = BTreeSet::new();
    let mut symbols = BTreeSet::new();
    let mut codes = BTreeSet::new();
    let scan = |text: &str,
                paths: &mut BTreeSet<String>,
                symbols: &mut BTreeSet<String>,
                codes: &mut BTreeSet<String>| {
        for token in text.split(|c: char| {
            c.is_whitespace() || matches!(c, '(' | ')' | ',' | ';' | '"' | '\'' | '`' | '[' | ']')
        }) {
            let t = token.trim_end_matches(|c: char| matches!(c, ':' | '.'));
            if t.is_empty() {
                continue;
            }
            if let Some((left, right)) = t.split_once("::") {
                if left.contains('/') {
                    paths.insert(strip_position(left).to_string());
                }
                if !right.is_empty()
                    && right
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
                {
                    symbols.insert(t.to_string());
                }
                continue;
            }
            if looks_like_path(t) {
                paths.insert(strip_position(t).to_string());
                continue;
            }
            if is_error_code(t) {
                codes.insert(t.to_ascii_uppercase());
            }
        }
    };
    if let Some(cmd) = command {
        scan(cmd, &mut paths, &mut symbols, &mut codes);
    }
    scan(output, &mut paths, &mut symbols, &mut codes);
    facts.paths = paths.into_iter().take(32).collect();
    facts.symbols = symbols.into_iter().take(32).collect();
    facts.error_codes = codes.into_iter().take(16).collect();
    facts.checks = detect_checks(command.unwrap_or(""), output, exit_status);
    facts
}

fn strip_position(path: &str) -> &str {
    // src/lib.rs:12:5 → src/lib.rs
    match path.find(':') {
        Some(i)
            if path[i + 1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit()) =>
        {
            &path[..i]
        }
        _ => path,
    }
}

fn looks_like_path(t: &str) -> bool {
    if t.starts_with("http://") || t.starts_with("https://") || t.len() < 4 {
        return false;
    }
    let base = strip_position(t);
    base.contains('/')
        && !base.ends_with('/')
        && base
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~' | '@' | '+'))
        && base
            .rsplit('/')
            .next()
            .is_some_and(|f| f.contains('.') || f.chars().any(|c| c.is_ascii_lowercase()))
}

fn is_error_code(t: &str) -> bool {
    let u = t.to_ascii_uppercase();
    (u.starts_with('E') && u.len() >= 4 && u[1..].chars().all(|c| c.is_ascii_digit()))
        || (u.starts_with("TS") && u.len() >= 5 && u[2..].chars().all(|c| c.is_ascii_digit()))
}

fn detect_checks(command: &str, output: &str, exit_status: Option<i32>) -> Vec<TypedCheck> {
    let cmd = command.to_ascii_lowercase();
    let out = output.to_ascii_lowercase();
    let mut checks = Vec::new();
    let ok = exit_status.map(|c| c == 0);
    if cmd.contains("cargo test")
        || cmd.contains("pytest")
        || cmd.contains("vitest")
        || cmd.contains("npm test")
        || cmd.contains("bun test")
        || out.contains("test result:")
    {
        let (p, f) = count_tests(&out);
        let passed = match (f, ok) {
            (Some(f), _) => f == 0,
            (None, Some(ok)) => ok,
            (None, None) => !out.contains("failed"),
        };
        checks.push(TypedCheck {
            kind: CheckKind::Test,
            passed,
            passed_count: p,
            failed_count: f,
        });
    }
    if cmd.contains("cargo check")
        || cmd.contains("tsc")
        || cmd.contains("mypy")
        || cmd.contains("typecheck")
    {
        checks.push(TypedCheck {
            kind: CheckKind::Typecheck,
            passed: ok.unwrap_or(!out.contains("error")),
            passed_count: None,
            failed_count: None,
        });
    }
    if cmd.contains("clippy")
        || cmd.contains("eslint")
        || cmd.contains("ruff")
        || cmd.contains(" lint")
    {
        checks.push(TypedCheck {
            kind: CheckKind::Lint,
            passed: ok.unwrap_or(!out.contains("error")),
            passed_count: None,
            failed_count: None,
        });
    }
    if cmd.contains("cargo build")
        || cmd.contains("npm run build")
        || cmd.contains("bun run build")
        || cmd.contains("make")
    {
        checks.push(TypedCheck {
            kind: CheckKind::Build,
            passed: ok.unwrap_or(!out.contains("error")),
            passed_count: None,
            failed_count: None,
        });
    }
    if cmd.contains("cargo fmt") || cmd.contains("prettier") || cmd.contains("black ") {
        checks.push(TypedCheck {
            kind: CheckKind::Format,
            passed: ok.unwrap_or(true),
            passed_count: None,
            failed_count: None,
        });
    }
    checks
}

fn count_tests(out: &str) -> (Option<u32>, Option<u32>) {
    // cargo: "test result: ok. 21 passed; 0 failed" ; pytest: "3 passed, 1 failed"
    let mut passed = None;
    let mut failed = None;
    let words: Vec<&str> = out
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | ',' | '.'))
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .filter(|w| !w.is_empty())
        .collect();
    for pair in words.windows(2) {
        if let Ok(n) = pair[0].parse::<u32>() {
            match pair[1] {
                "passed" => passed = Some(passed.unwrap_or(0) + n),
                "failed" => failed = Some(failed.unwrap_or(0) + n),
                _ => {}
            }
        }
    }
    if passed.is_some() && failed.is_none() {
        failed = Some(0);
    }
    (passed, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_test_output_yields_a_typed_check_with_counts() {
        let out = "running 3 tests\ntest a ... ok\ntest result: FAILED. 2 passed; 1 failed; 0 ignored\n  --> crates/logic/src/lens.rs:41:9";
        let f = parse_tool_result("Bash", Some("cargo test -p cortex-logic"), out, Some(101));
        assert_eq!(
            f.checks,
            vec![TypedCheck {
                kind: CheckKind::Test,
                passed: false,
                passed_count: Some(2),
                failed_count: Some(1)
            }]
        );
        assert_eq!(f.paths, vec!["crates/logic/src/lens.rs"]);
        assert!(f.is_material());
        assert!(f.statement().contains("exit 101"));
        assert!(
            f.statement().contains("test failed (2 passed, 1 failed)"),
            "{}",
            f.statement()
        );
        assert_eq!(
            f.idempotency_key(),
            parse_tool_result("Bash", Some("cargo test -p cortex-logic"), out, Some(101))
                .idempotency_key()
        );
    }

    #[test]
    fn symbols_error_codes_and_non_material_results() {
        let f = parse_tool_result("Bash", Some("cargo check"), "error[E0277]: the trait bound ... in crate::store_spi::sqlite::SqliteStore\n --> src/a.rs:3:1", Some(1));
        assert_eq!(f.error_codes, vec!["E0277"]);
        assert!(
            f.symbols.iter().any(|s| s.contains("SqliteStore")),
            "{:?}",
            f.symbols
        );
        assert_eq!(f.checks[0].kind, CheckKind::Typecheck);
        assert!(!f.checks[0].passed);
        let idle = parse_tool_result("Read", None, "just some prose without anything", None);
        assert!(!idle.is_material(), "{idle:?}");
        let explained = parse_tool_result(
            "Bash",
            Some("ls"),
            "because the cache was cold the build was slow",
            Some(0),
        );
        assert!(
            !explained.statement().contains("because"),
            "explanations are never captured as facts"
        );
    }
}

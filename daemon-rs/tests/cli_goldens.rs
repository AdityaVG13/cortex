use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

#[test]
fn cli_help_matches_golden() {
    let output = run_cortex(&["help"]);
    assert_success(&output);
    assert_empty_stderr(&output);
    assert_golden("cli/help", &stdout_text(output));
}

#[test]
fn cli_capabilities_json_matches_golden() {
    let output = run_cortex(&["capabilities", "--json"]);
    assert_success(&output);
    assert_empty_stderr(&output);
    assert_golden("cli/capabilities_json", &stdout_text(output));
}

#[test]
fn cli_robot_docs_guide_matches_golden() {
    let output = run_cortex(&["robot-docs", "guide"]);
    assert_success(&output);
    assert_empty_stderr(&output);
    assert_golden("cli/robot_docs_guide", &stdout_text(output));
}

#[test]
fn cli_unknown_command_diagnostic_matches_golden() {
    let output = run_cortex(&["capability"]);
    assert!(
        !output.status.success(),
        "unknown command should fail, stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "unknown command should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_golden(
        "cli/unknown_command_capability_stderr",
        &stderr_text(output),
    );
}

fn run_cortex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|err| panic!("failed to run cortex {args:?}: {err}"))
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed with status {}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_empty_stderr(output: &Output) {
    assert!(
        output.stderr.is_empty(),
        "expected empty stderr, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout_text(output: Output) -> String {
    String::from_utf8(output.stdout).expect("stdout should be valid UTF-8")
}

fn stderr_text(output: Output) -> String {
    String::from_utf8(output.stderr).expect("stderr should be valid UTF-8")
}

fn assert_golden(name: &str, actual: &str) {
    let actual = canonicalize(actual);
    let golden_path = golden_path(name);

    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::create_dir_all(golden_path.parent().expect("golden parent")).unwrap_or_else(|err| {
            panic!(
                "failed to create golden directory {}: {err}",
                golden_path.display()
            )
        });
        fs::write(&golden_path, actual).unwrap_or_else(|err| {
            panic!("failed to update golden {}: {err}", golden_path.display())
        });
        return;
    }

    let expected = fs::read_to_string(&golden_path).unwrap_or_else(|err| {
        panic!(
            "golden file missing: {}\nerror: {err}\nrun with UPDATE_GOLDENS=1 cargo test --test cli_goldens, then review git diff daemon-rs/tests/golden/",
            golden_path.display()
        )
    });
    let expected = canonicalize(&expected);
    if actual != expected {
        let actual_path = golden_path.with_extension("actual");
        fs::write(&actual_path, &actual).unwrap_or_else(|err| {
            panic!(
                "failed to write actual output {}: {err}",
                actual_path.display()
            )
        });
        panic!(
            "GOLDEN MISMATCH: {name}\n{}\nactual output written to {}\nreview with: git diff --no-index {} {}",
            unified_diff(&expected, &actual),
            actual_path.display(),
            golden_path.display(),
            actual_path.display()
        );
    }
}

fn golden_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut parts = name.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            path.push(part);
        } else {
            path.push(format!("{part}.golden"));
        }
    }
    path
}

fn canonicalize(output: &str) -> String {
    let text = output.replace("\r\n", "\n").replace('\\', "/");
    let mut canonical = text
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    canonical.push('\n');
    canonical
}

fn unified_diff(expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    let max_len = expected_lines.len().max(actual_lines.len());
    let first_diff = (0..max_len)
        .find(|&idx| expected_lines.get(idx) != actual_lines.get(idx))
        .unwrap_or(0);
    let start = first_diff.saturating_sub(3);
    let end = (first_diff + 4).min(max_len);
    let mut diff = String::from("--- expected\n+++ actual\n");

    for idx in start..end {
        match (expected_lines.get(idx), actual_lines.get(idx)) {
            (Some(expected), Some(actual)) if expected == actual => {
                diff.push_str(&format!(" {:>4} {expected}\n", idx + 1));
            }
            (Some(expected), Some(actual)) => {
                diff.push_str(&format!("-{:>4} {expected}\n", idx + 1));
                diff.push_str(&format!("+{:>4} {actual}\n", idx + 1));
            }
            (Some(expected), None) => {
                diff.push_str(&format!("-{:>4} {expected}\n", idx + 1));
            }
            (None, Some(actual)) => {
                diff.push_str(&format!("+{:>4} {actual}\n", idx + 1));
            }
            (None, None) => {}
        }
    }

    diff
}

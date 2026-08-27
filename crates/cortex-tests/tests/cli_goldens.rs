use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

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
fn cli_status_json_unavailable_matches_golden() {
    let home = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("cli-goldens")
        .join("status-unavailable-home");
    let _ = fs::remove_dir_all(&home);
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex_with_env(
        &[
            "status",
            "--json",
            "--home",
            &home_arg,
            "--port",
            "65534",
            "--bind",
            "127.0.0.1",
        ],
        &[("CORTEX_DISABLE_IPC", "1")],
    );
    assert_failure(&output);
    assert_empty_stderr(&output);
    let status_json = scrub_status_json(&stdout_text(output), &home);
    assert_golden("cli/status_json_unavailable", &status_json);
}

#[test]
fn cli_robot_docs_guide_matches_golden() {
    let output = run_cortex(&["robot-docs", "guide"]);
    assert_success(&output);
    assert_empty_stderr(&output);
    assert_golden("cli/robot_docs_guide", &stdout_text(output));
}

#[test]
fn cli_robot_docs_help_alias_matches_golden() {
    let output = run_cortex(&["robot-docs", "help"]);
    assert_success(&output);
    assert_empty_stderr(&output);
    assert_golden("cli/robot_docs_guide", &stdout_text(output));
}

#[test]
fn cli_service_help_succeeds() {
    let output = run_cortex(&["service", "help"]);
    assert_success(&output);
    assert_empty_stderr(&output);
    assert_eq!(
        canonicalize("Usage: cortex service <install|uninstall|start|stop|status|ensure>\n"),
        canonicalize(&stdout_text(output))
    );
}

#[test]
fn cli_service_unknown_subcommand_fails() {
    let output = run_cortex(&["service", "bogus"]);
    assert!(
        !output.status.success(),
        "unknown service subcommand should fail, stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "unknown service subcommand should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Usage: cortex service <install|uninstall|start|stop|status|ensure>\n"),
        canonicalize(&stderr_text(output))
    );
}

#[test]
fn cli_admin_budgets_validate_missing_path_value_fails() {
    let output = run_cortex(&["admin", "budgets", "validate", "--path", "--json"]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "missing path value should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Missing value for --path\n"),
        canonicalize(&stderr_text(output))
    );
}

#[test]
fn cli_export_unknown_option_fails_before_database_work() {
    let output = run_cortex(&["export", "--bogus"]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown export option should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unknown option: --bogus\n"),
        canonicalize(&stderr_text(output))
    );
}

#[test]
fn cli_eval_window_days_is_accepted() {
    let home = test_home("eval-window-days-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["eval", "--window-days", "7", "--json", "--home", &home_arg]);
    assert_success(&output);
    assert_empty_stderr(&output);
    let payload: Value =
        serde_json::from_str(&stdout_text(output)).expect("eval output should be JSON");
    assert_eq!(payload["windowDays"].as_i64(), Some(7));
}

#[test]
fn cli_sync_export_unknown_option_fails_before_lock() {
    let home = test_home("sync-export-unknown-option-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["sync", "export", "--bogus", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown sync export option should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unknown option: --bogus\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.join("sync.lock").exists(),
        "invalid options should fail before creating the sync lock"
    );
}

#[test]
fn cli_sync_unknown_subcommand_fails_before_lock() {
    let home = test_home("sync-unknown-subcommand-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["sync", "bogus", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown sync subcommand should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Usage: cortex sync <export|import|watch> [options]\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.join("sync.lock").exists(),
        "unknown sync subcommand should fail before creating the sync lock"
    );
}

#[test]
fn cli_status_unknown_option_fails_before_status_work() {
    let home = unused_test_home("status-unknown-option-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["status", "--bogus", "--home", &home_arg, "--port", "65534"]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown status option should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unknown option: --bogus\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.exists(),
        "invalid status options should fail before creating Cortex home"
    );
}

#[test]
fn cli_serve_unknown_option_fails_before_daemon_start() {
    let home = unused_test_home("serve-unknown-option-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex_with_timeout(
        &["serve", "--bogus", "--home", &home_arg, "--port", "65534"],
        Duration::from_secs(2),
    );
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown serve option should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unknown option: --bogus\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.exists(),
        "invalid serve options should fail before creating Cortex home"
    );
}

#[test]
fn cli_setup_dry_run_without_team_fails_before_setup() {
    let home = unused_test_home("setup-dry-run-without-team-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["setup", "--dry-run", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "invalid setup options should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("--dry-run requires --team\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.exists(),
        "invalid setup options should fail before creating Cortex home"
    );
}

#[test]
fn cli_reindex_unknown_option_fails_before_database_work() {
    let home = unused_test_home("reindex-unknown-option-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["reindex", "--bogus", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown reindex option should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unknown option: --bogus\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.join("cortex.db").exists(),
        "invalid reindex options should fail before opening the database"
    );
}

#[test]
fn cli_cleanup_unknown_option_fails_before_home_work() {
    let home = unused_test_home("cleanup-unknown-option-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["cleanup", "--bogus", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown cleanup option should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unknown option: --bogus\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.exists(),
        "invalid cleanup options should fail before creating Cortex home"
    );
}

#[test]
fn cli_admin_rollback_missing_session_value_fails_before_database_work() {
    let home = unused_test_home("rollback-missing-session-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&[
        "admin",
        "rollback",
        "--session-id",
        "--json",
        "--home",
        &home_arg,
    ]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "missing rollback session id should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Missing value for --session-id\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.join("cortex.db").exists(),
        "invalid rollback options should fail before opening the database"
    );
}

#[test]
fn cli_backup_unknown_option_fails_before_database_work() {
    let home = unused_test_home("backup-unknown-option-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["backup", "--bogus", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown backup option should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unknown option: --bogus\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.join("cortex.db").exists(),
        "invalid backup options should fail before opening the database"
    );
}

#[test]
fn cli_restore_unexpected_argument_fails_before_restore_work() {
    let home = test_home("restore-unexpected-argument-home");
    let backup_file = home.join("backup.db");
    fs::write(&backup_file, "not a sqlite database").expect("write placeholder backup");
    let home_arg = home.to_string_lossy().to_string();
    let backup_arg = backup_file.to_string_lossy().to_string();
    let output = run_cortex(&["restore", &backup_arg, "extra", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "invalid restore options should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unexpected argument: extra\n"),
        canonicalize(&stderr_text(output))
    );
    let has_pre_restore_backup = fs::read_dir(&home)
        .expect("read test home")
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("cortex.pre-restore.")
        });
    assert!(
        !has_pre_restore_backup,
        "invalid restore options should fail before creating a pre-restore backup"
    );
}

#[test]
fn cli_user_list_unknown_option_fails_before_token_read() {
    let home = unused_test_home("user-list-unknown-option-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["user", "list", "--bogus", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown user list option should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unknown option: --bogus\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.exists(),
        "invalid user list options should fail before touching the Cortex home"
    );
}

#[test]
fn cli_user_add_missing_role_value_fails_before_token_read() {
    let home = unused_test_home("user-add-missing-role-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["user", "add", "alice", "--role", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "invalid user add options should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Missing value for --role\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.exists(),
        "invalid user add options should fail before touching the Cortex home"
    );
}

#[test]
fn cli_team_add_extra_positional_fails_before_token_read() {
    let home = unused_test_home("team-add-extra-positional-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&[
        "team", "add", "platform", "alice", "extra", "--home", &home_arg,
    ]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "invalid team add options should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unexpected argument: extra\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.exists(),
        "invalid team add options should fail before touching the Cortex home"
    );
}

#[test]
fn cli_admin_assign_owner_missing_to_value_fails_before_token_read() {
    let home = unused_test_home("admin-assign-owner-missing-to-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&[
        "admin",
        "assign-owner",
        "--to",
        "--table",
        "memories",
        "--home",
        &home_arg,
    ]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "invalid assign-owner options should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Missing value for --to\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.exists(),
        "invalid assign-owner options should fail before touching the Cortex home"
    );
}

#[test]
fn cli_admin_stats_unknown_option_fails_before_token_read() {
    let home = unused_test_home("admin-stats-unknown-option-home");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["admin", "stats", "--bogus", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "unknown admin stats option should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        canonicalize("Unknown option: --bogus\n"),
        canonicalize(&stderr_text(output))
    );
    assert!(
        !home.exists(),
        "invalid admin stats options should fail before touching the Cortex home"
    );
}

#[test]
fn cli_admin_requests_use_home_override_for_token_path() {
    let home = unused_test_home("admin-request-home-override");
    let home_arg = home.to_string_lossy().to_string();
    let output = run_cortex(&["user", "list", "--home", &home_arg]);
    assert_failure(&output);
    assert!(
        output.stdout.is_empty(),
        "token-read failure should not write stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = stderr_text(output);
    assert!(
        stderr.contains(&home_arg),
        "admin request should read the token from --home, stderr={stderr}"
    );
    assert!(
        stderr.contains("cortex.token"),
        "admin request should report the Cortex token path, stderr={stderr}"
    );
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
    run_cortex_with_env(args, &[])
}

fn run_cortex_with_timeout(args: &[&str], timeout: Duration) -> Output {
    run_cortex_with_env_timeout(args, &[], timeout)
}

fn run_cortex_with_env(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(cortex_tests::cortex_bin());
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    command
        .output()
        .unwrap_or_else(|err| panic!("failed to run cortex {args:?}: {err}"))
}

fn run_cortex_with_env_timeout(args: &[&str], envs: &[(&str, &str)], timeout: Duration) -> Output {
    let mut command = Command::new(cortex_tests::cortex_bin());
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }

    let mut child = command
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn cortex {args:?}: {err}"));
    let started_at = Instant::now();
    loop {
        if child
            .try_wait()
            .unwrap_or_else(|err| panic!("failed to poll cortex {args:?}: {err}"))
            .is_some()
        {
            return child
                .wait_with_output()
                .unwrap_or_else(|err| panic!("failed to collect cortex {args:?}: {err}"));
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .unwrap_or_else(|err| panic!("failed to collect timed-out cortex {args:?}: {err}"));
            panic!(
                "cortex {args:?} timed out after {:?}, stdout={}, stderr={}",
                timeout,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn unused_test_home(name: &str) -> PathBuf {
    let home = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("cli-goldens")
        .join(name);
    let _ = fs::remove_dir_all(&home);
    home
}

fn test_home(name: &str) -> PathBuf {
    let home = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("cli-goldens")
        .join(name);
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home)
        .unwrap_or_else(|err| panic!("failed to create test home {}: {err}", home.display()));
    home
}

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded, stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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

fn scrub_status_json(raw: &str, home: &Path) -> String {
    let mut payload: Value = serde_json::from_str(raw).expect("status output should be JSON");
    scrub_json_strings(&mut payload, home);
    serde_json::to_string_pretty(&payload).expect("status JSON should re-serialize")
}

fn scrub_json_strings(value: &mut Value, home: &Path) {
    match value {
        Value::String(text) => *text = scrub_status_text(text, home),
        Value::Array(values) => {
            for value in values {
                scrub_json_strings(value, home);
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                scrub_json_strings(value, home);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn scrub_status_text(text: &str, home: &Path) -> String {
    let home_native = home.display().to_string();
    let home_slash = home_native.replace('\\', "/");
    text.replace(&home_native, "[CORTEX_HOME]")
        .replace(&home_slash, "[CORTEX_HOME]")
        .replace('\\', "/")
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

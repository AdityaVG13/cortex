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
    assert_golden("help", &String::from_utf8_lossy(&output.stdout));
}

#[test]
fn cli_capabilities_json_matches_golden_and_is_deterministic() {
    let home = unused_test_home("capabilities");
    let args = ["capabilities", "--json", "--home", home.to_str().unwrap()];
    let output = run_cortex(&args);
    assert_success(&output);
    let repeated = run_cortex(&args);
    assert_success(&repeated);
    assert_eq!(output.stdout, repeated.stdout);
    let payload: Value = serde_json::from_slice(&output.stdout).expect("capabilities JSON");
    assert_golden(
        "capabilities_json",
        &serde_json::to_string_pretty(&payload).unwrap(),
    );
    assert!(!home.exists(), "discovery must not initialize a brain");
}

#[test]
fn cli_status_json_unavailable_matches_golden_without_creating_home() {
    let home = unused_test_home("status-unavailable");
    let args = ["status", "--json", "--home", home.to_str().unwrap()];
    let output = run_cortex(&args);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let repeated = run_cortex(&args);
    assert_eq!(repeated.status.code(), Some(1));
    assert!(repeated.stderr.is_empty());
    assert_eq!(output.stdout, repeated.stdout);
    let raw = String::from_utf8(output.stdout).unwrap();
    let encoded_home = serde_json::to_string(home.to_str().unwrap()).unwrap();
    let scrubbed = raw.replace(&encoded_home[1..encoded_home.len() - 1], "[CORTEX_HOME]");
    let payload: Value = serde_json::from_str(&scrubbed).expect("status JSON");
    assert_golden(
        "status_json_unavailable",
        &serde_json::to_string_pretty(&payload).unwrap(),
    );
    assert!(
        !home.exists(),
        "missing-brain status must not initialize the home"
    );
}

#[test]
fn cli_robot_docs_guide_and_help_alias_match_golden() {
    for subcommand in ["guide", "help"] {
        let output = run_cortex(&["robot-docs", subcommand]);
        assert_success(&output);
        assert_golden("robot_docs_guide", &String::from_utf8_lossy(&output.stdout));
    }
}

#[test]
fn retired_commands_are_rejected_before_any_home_or_database_work() {
    let cases: &[&[&str]] = &[
        &["service", "help"],
        &["service", "bogus"],
        &["service", "install"],
        &["admin", "budgets", "validate", "--path", "--json"],
        &["admin", "rollback", "--session-id", "--json"],
        &["admin", "assign-owner", "--to", "--table", "memories"],
        &["admin", "stats", "--bogus"],
        &["export", "--bogus"],
        &["export"],
        &["import"],
        &["sync", "export", "--bogus"],
        &["sync", "bogus"],
        &["sync", "watch"],
        &["user", "list"],
        &["user", "list", "--bogus"],
        &["user", "add", "alice", "--role"],
        &["team", "add", "platform", "alice", "extra"],
        &["team", "list"],
        &["migrate", "--owner", "alice"],
        &["plugin", "ensure-daemon"],
    ];
    for (index, case) in cases.iter().enumerate() {
        let home = unused_test_home(&format!("retired-{index}"));
        let mut args = case.to_vec();
        args.extend(["--home", home.to_str().unwrap()]);
        let output = run_cortex(&args);
        assert_rejection(
            &output,
            &format!(
                "[cortex] Unknown command: {}\nRun `cortex help` or `cortex capabilities --json` for supported commands.\n",
                case[0]
            ),
        );
        assert!(!home.exists(), "retired command {case:?} touched home");
    }
}

#[test]
fn surviving_commands_validate_arguments_before_side_effects() {
    let cases: &[(&[&str], &str)] = &[
        (&["status", "--bogus"], "Unknown option: --bogus\n"),
        (&["serve", "--bogus"], "Unknown option: --bogus\n"),
        (&["setup", "--dry-run"], "Unknown option: --dry-run\n"),
        (&["setup", "--team"], "Unknown option: --team\n"),
        (&["reindex", "--bogus"], "Unknown option: --bogus\n"),
        (&["cleanup", "--bogus"], "Unknown option: --bogus\n"),
        (&["backup", "--bogus"], "Unknown option: --bogus\n"),
        (&["capabilities", "--bogus"], "Unknown option: --bogus\n"),
        (&["mcp", "--agent", "--json"], "Missing value for --agent\n"),
        (&["hook-boot", "--home", "--agent", "cursor"], "Missing value for --home\n"),
        (&["hook-status", "--bogus"], "Unknown option: --bogus\n"),
        (
            &["hook", "PostToolUse", "--agent", "--json"],
            "Missing value for --agent\n",
        ),
        (
            &["mcp", "--url", "http://127.0.0.1:1"],
            "Unknown option: --url\n",
        ),
        (
            &["mcp", "--api-key", "retired"],
            "Unknown option: --api-key\n",
        ),
        (
            &["plugin", "mcp", "--url", "http://127.0.0.1:1"],
            "Unknown option: --url\n",
        ),
        (
            &["cleanup", "--max-passes", "--dry-run"],
            "Missing value for --max-passes\n",
        ),
        (
            &["boot", "--url", "http://127.0.0.1:1"],
            "[cortex] Unknown option: --url\n",
        ),
        (
            &["boot", "--api-key", "retired"],
            "[cortex] Unknown option: --api-key\n",
        ),
        (
            &["boot", "--budget", "--json"],
            "[cortex] Missing value for --budget\n",
        ),
    ];
    for (index, (case, diagnostic)) in cases.iter().enumerate() {
        let home = unused_test_home(&format!("validation-{index}"));
        let mut args = case.to_vec();
        args.extend(["--home", home.to_str().unwrap()]);
        let output = run_cortex(&args);
        assert_rejection(&output, diagnostic);
        assert!(!home.exists(), "invalid {case:?} touched home");
    }
}

#[test]
fn cli_eval_window_days_is_accepted() {
    let home = unused_test_home("eval-window-days");
    fs::create_dir_all(&home).unwrap();
    let output = run_cortex(&[
        "eval",
        "--window-days",
        "7",
        "--json",
        "--home",
        home.to_str().unwrap(),
    ]);
    assert_success(&output);
    let payload: Value = serde_json::from_slice(&output.stdout).expect("eval JSON");
    assert_eq!(payload["windowDays"].as_i64(), Some(7));
}

#[test]
fn cli_restore_unexpected_argument_fails_before_restore_work() {
    let home = unused_test_home("restore-unexpected-argument");
    fs::create_dir_all(&home).unwrap();
    let backup = home.join("backup.db");
    fs::write(&backup, "not a sqlite database").unwrap();
    let output = run_cortex(&[
        "restore",
        backup.to_str().unwrap(),
        "extra",
        "--home",
        home.to_str().unwrap(),
    ]);
    assert_rejection(&output, "Unexpected argument: extra\n");
    assert_eq!(fs::read(&backup).unwrap(), b"not a sqlite database");
    assert_eq!(
        fs::read_dir(&home).unwrap().count(),
        1,
        "invalid restore created artifacts"
    );
}

#[test]
fn cli_unknown_command_diagnostic_suggests_capabilities() {
    let output = run_cortex(&["capability"]);
    assert_rejection(
        &output,
        "[cortex] Unknown command: capability\nDid you mean: `cortex capabilities --json`?\nRun `cortex help` or `cortex capabilities --json` for supported commands.\n",
    );
}

fn run_cortex(args: &[&str]) -> Output {
    let mut child = Command::new(cortex_tests::cortex_bin())
        .args(args)
        .env_remove("CORTEX_DB")
        .env_remove("CORTEX_API_BASE")
        .env_remove("CORTEX_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn cortex {args:?}: {err}"));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().expect("poll cortex").is_some() {
            return child.wait_with_output().expect("collect cortex output");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().expect("reap cortex");
            panic!("cortex {args:?} timed out: {output:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn unused_test_home(name: &str) -> PathBuf {
    // tempfile uniqueness, then remove: discovery must see a path that does not exist yet.
    let home = tempfile::Builder::new()
        .prefix(&format!("cli-goldens-{name}-"))
        .tempdir()
        .expect("unique unused home")
        .keep();
    fs::remove_dir_all(&home).expect("unused home must not exist yet");
    home
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "command failed: {output:?}");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_rejection(output: &Output, diagnostic: &str) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected input error: {output:?}"
    );
    assert!(
        output.stdout.is_empty(),
        "rejected command wrote stdout: {output:?}"
    );
    assert_eq!(
        canonicalize(&String::from_utf8_lossy(&output.stderr)),
        canonicalize(diagnostic)
    );
}

fn canonicalize(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\\', "/");
    format!(
        "{}\n",
        text.lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn assert_golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("golden/cli")
        .join(format!("{name}.golden"));
    let expected = fs::read_to_string(&path).expect("read reviewed golden");
    // Contract migrations require explicit review, never environment-driven regeneration.
    assert_eq!(
        canonicalize(actual),
        canonicalize(&expected),
        "golden mismatch: {}",
        path.display()
    );
}

#[test]
fn parse_flag_values_does_not_eat_the_next_option() {
    let args = vec![
        "--path".into(),
        "--scope".into(),
        "proj".into(),
        "--path".into(),
        "/tmp/ok".into(),
    ];
    assert_eq!(
        cortex_daemon::cli::parse_flag_values(&args, "--path"),
        vec!["/tmp/ok"]
    );
}

#[test]
fn parse_flag_value_quotes_values_that_start_with_dashes() {
    let quoted = vec!["--home".into(), "--".into(), "--odd-home".into()];
    assert_eq!(
        cortex_daemon::cli::parse_flag_value(&quoted, "--home").as_deref(),
        Some("--odd-home")
    );
    let skipped = vec!["--home".into(), "--db".into(), "/tmp/x".into()];
    assert_eq!(cortex_daemon::cli::parse_flag_value(&skipped, "--home"), None);
    let paths = vec!["--path".into(), "--".into(), "--odd-dir".into()];
    assert_eq!(
        cortex_daemon::cli::parse_flag_values(&paths, "--path"),
        vec!["--odd-dir"]
    );
}

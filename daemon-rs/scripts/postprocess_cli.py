#!/usr/bin/env python3
"""Post-process extracted CLI modules."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAIN = ROOT / "src" / "main.rs"
CLI = ROOT / "src" / "cli"


def lines(start: int, end: int) -> str:
    return "\n".join(MAIN.read_text().splitlines()[start - 1 : end])


def dedent_block(start: int, end: int) -> str:
    out = []
    for line in lines(start, end).splitlines():
        out.append(line[12:] if line.startswith("            ") else line)
    return "\n".join(out)


def wrap_match_fn(name: str, start: int, end: int) -> str:
    body = dedent_block(start, end)
    return (
        f"pub(crate) async fn {name}(paths: &auth::CortexPaths, args: &[String]) {{\n"
        f"    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or(\"\");\n"
        f"    match subcmd {{\n{body}\n    }}\n}}\n"
    )


def fix_common() -> None:
    path = CLI / "common.rs"
    text = path.read_text()
    if "use crate::transport;" not in text:
        text = text.replace("use crate::db;\n", "use crate::db;\nuse crate::transport;\n")
    header = (
        'pub(crate) const SINGLE_DAEMON_TEST_BYPASS_ENV: &str = '
        '"CORTEX_SINGLE_DAEMON_TEST_BYPASS";\n\n'
        "pub(crate) fn read_auth_token(paths: &auth::CortexPaths) -> Result<String, String> {\n"
        "    let token_path = paths.token.clone();\n"
        "    std::fs::read_to_string(&token_path)\n"
        "        .map(|v| v.trim().to_string())\n"
        "        .map_err(|_| format!(\n"
        '            "Cannot read auth token at {}. Is the daemon running?",\n'
        "            token_path.display()\n"
        "        ))\n"
        "}\n\n"
    )
    if "read_auth_token" not in text:
        text = text.replace("use crate::transport;\n\n", f"use crate::transport;\n\n{header}")
    path.write_text(text)


def fix_daemon() -> None:
    path = CLI / "daemon.rs"
    text = path.read_text()
    text = text.replace(
        "install_daemon_panic_hook(&paths)",
        "crate::install_daemon_panic_hook(&paths)",
    )
    text = text.replace(
        'eprintln!("[cortex] Cleaned {cleaned_backups} old backups, kept {BACKUP_RETENTION_COUNT}");',
        'eprintln!(\n        "[cortex] Cleaned {cleaned_backups} old backups, kept {}",\n        super::cleanup::BACKUP_RETENTION_COUNT\n    );',
    )
    marker = "// ── Admin CLI helpers ───────────────────────────────────────────────────────"
    if marker in text:
        text = text[: text.index(marker)] + text[text.index("// ── Shared daemon logic") :]
    extra = """
use super::boot::boot_agent;
use super::cleanup::{
    cleanup_backup_retention, cleanup_bridge_backups, cleanup_expired_rows, create_backup,
    rotate_startup_logs, should_backup,
};
use super::common::{
    env_trimmed, local_daemon_base_url, normalize_option, parse_env_u64, parse_env_usize,
    parse_truthy_flag, single_daemon_test_bypass_enabled,
};

#[cfg(not(windows))]
use daemon_lifecycle::issue_owner_token_for_spawn;
use daemon_lifecycle::{
    daemon_healthy, is_cortex_health_payload, readiness_state_from_payload,
    validate_spawned_owner_claim, wait_for_health, DAEMON_OWNER_TOKEN_ENV,
    SPAWN_PARENT_START_TIME_ENV,
};
"""
    old = """use super::cleanup::{
    cleanup_backup_retention, cleanup_bridge_backups, cleanup_expired_rows, rotate_startup_logs,
};
use super::common::{
    env_trimmed, parse_env_u64, parse_env_usize, parse_truthy_flag,
    single_daemon_test_bypass_enabled,
};
"""
    text = text.replace(old, extra)
    path.write_text(text)


def fix_cleanup() -> None:
    path = CLI / "cleanup.rs"
    text = path.read_text().replace(
        "const BACKUP_RETENTION_COUNT", "pub(crate) const BACKUP_RETENTION_COUNT", 1
    )
    if "use crate::compaction;" not in text:
        text = text.replace("use crate::db;\n", "use crate::db;\nuse crate::compaction;\n")
    backup_fn = """pub(crate) fn run_backup_cli(paths: &auth::CortexPaths) {
    let db_path = paths.db.clone();
    let home_dir = paths.home.clone();
    let conn = match db::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: failed to open database: {e}");
            std::process::exit(1);
        }
    };
    db::checkpoint_wal_best_effort(&conn);
    drop(conn);
    let backup_dir = home_dir.join("backups");
    match create_backup(&db_path, &backup_dir) {
        Ok(path) => println!("Backup created: {path}"),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
"""
    restore_fn = (
        f"pub(crate) fn run_restore_cli(paths: &auth::CortexPaths, args: &[String]) {{\n"
        f"{dedent_block(850, 953)}\n}}\n"
    )
    path.write_text(text + "\n\n" + backup_fn + "\n" + restore_fn)


def fix_admin() -> None:
    path = CLI / "admin.rs"
    base = path.read_text()
    imports = """use super::common::{
    admin_request, api_key_output_masked, confirm_action, format_api_key_for_output, json_field,
    json_str, json_str_or, parse_flag_value, required_cli_positional_or_exit,
    validate_cli_options_or_exit,
};
"""
    base = base.replace(
        "use super::common::{\n    admin_request, confirm_action, format_api_key_for_output, json_field,\n    json_str,\n    json_str_or, parse_flag_value, required_cli_positional_or_exit,\n    validate_cli_options_or_exit,\n};",
        imports,
    )
    if "use crate::db;" not in base:
        base = base.replace("use crate::budgets;", "use crate::budgets;\nuse crate::db;")
    base = base.replace(
        "admin::rollback_session_by_id",
        "crate::admin::rollback_session_by_id",
    )
    base += "\n" + wrap_match_fn("run_user_cli", 960, 1112)
    base += "\n" + wrap_match_fn("run_team_cli", 1120, 1243)
    base += "\n" + wrap_match_fn("run_admin_cli", 1251, 1407)
    path.write_text(base)


def fix_sync() -> None:
    path = CLI / "sync.rs"
    text = path.read_text()
    dup = """pub(crate) fn open_cli_connection(db_path: &Path) -> Result<rusqlite::Connection, String> {
    let conn = db::open(db_path)
        .map_err(|e| format!("Failed to open database at {}: {e}", db_path.display()))?;
    db::configure(&conn).map_err(|e| format!("Failed to configure database: {e}"))?;
    db::initialize_schema(&conn).map_err(|e| format!("Failed to initialize schema: {e}"))?;
    db::run_pending_migrations_quiet(&conn);
    crystallize::migrate_crystal_tables(&conn);
    Ok(conn)
}

"""
    if dup in text:
        text = text.replace(dup, "")
    if "use crate::crystallize;" not in text:
        text = text.replace("use crate::db;\n", "use crate::db;\nuse crate::crystallize;\n")
    path.write_text(text)


def fix_doctor() -> None:
    path = CLI / "doctor.rs"
    text = path.read_text()
    orphan = "/// CLI runs offline; no live daemon connection is required."
    if orphan in text:
        text = text.split(orphan)[0].rstrip() + "\n"
    header = """use serde_json::Value;
use std::collections::HashSet;

use crate::auth;
use crate::compaction;
use crate::db;

use super::cleanup::{event_type_count, top_event_type_counts};
use super::common::{json_field, json_str, json_str_or};
"""
    text = text.replace(
        """use serde_json::Value;

use crate::auth;
use crate::db;

use super::common::{json_field, json_str, json_str_or};
""",
        header,
    )
    path.write_text(text)


def fix_tests() -> None:
    path = CLI / "tests.rs"
    text = path.read_text()
    text = text.replace("#[cfg(test)]\nmod tests {\n\nmod tests {", "#[cfg(test)]\nmod tests {", 1)
    text = text.replace("use super::*;", "use crate::cli::*;\n    use crate::*;", 1)
    path.write_text(text)


def main() -> None:
    fix_common()
    fix_daemon()
    fix_cleanup()
    fix_admin()
    fix_sync()
    fix_doctor()
    fix_tests()


if __name__ == "__main__":
    main()

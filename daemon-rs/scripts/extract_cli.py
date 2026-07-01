#!/usr/bin/env python3
"""Extract CLI modules from main.rs into src/cli/."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
MAIN = SRC / "main.rs"
CLI = SRC / "cli"


def lines(main_text: str, start: int, end: int) -> str:
    rows = main_text.splitlines()
    return "\n".join(rows[start - 1 : end])


def write_module(name: str, header: str, body: str) -> None:
    path = CLI / f"{name}.rs"
    content = f"// SPDX-License-Identifier: MIT\n\n{header}\n{body}\n"
    path.write_text(content)
    print(f"Wrote {path} ({len(content.splitlines())} lines)")


def main() -> None:
    text = MAIN.read_text()
    CLI.mkdir(exist_ok=True)

    COMMON_USES = """use chrono::Utc;
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::auth;
use crate::crystallize;
use crate::db;
use crate::transport;
"""

    CLEANUP_USES = """use chrono::Utc;
use std::path::Path;
use std::time::Duration;

use crate::auth;
use crate::db;

use super::common::{is_cli_option_token, validate_cli_options_or_exit};
"""

    STATUS_USES = """use serde_json::{json, Value};
use std::time::Duration;

use crate::auth;
use crate::daemon_lifecycle::{is_cortex_health_payload, readiness_state_from_payload};
use crate::transport;

use super::common::json_str;
"""

    USAGE_USES = """use serde_json::{json, Value};

use crate::DEFAULT_CORTEX_PORT;
"""

    ADMIN_USES = """use serde_json::{json, Value};

use crate::auth;
use crate::budgets;

use super::common::{
    admin_request, confirm_action, format_api_key_for_output, json_field, json_str,
    json_str_or, parse_flag_value, required_cli_positional_or_exit,
    validate_cli_options_or_exit,
};
"""

    DOCTOR_USES = """use serde_json::Value;

use crate::auth;
use crate::db;

use super::common::{json_field, json_str, json_str_or};
"""

    REINDEX_USES = """use serde_json::{json, Value};

use crate::auth;
use crate::crystallize;
use crate::db;
use crate::indexer;

use super::common::{open_cli_connection, validate_cli_options_or_exit};
"""

    EMBEDDINGS_USES = """use serde_json::{json, Value};

use crate::auth;
use crate::db;
use crate::embeddings;

use super::common::{
    parse_flag_usize, parse_flag_value, validate_cli_options, validate_cli_options_or_exit,
};
use super::daemon::ensure_daemon;
"""

    SYNC_USES = """use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::auth;
use crate::db;
use crate::export_data;

use super::common::{
    open_cli_connection, parse_flag_value, parse_flag_usize, validate_cli_options,
    validate_cli_options_or_exit,
};
"""

    EVAL_USES = """use serde_json::{json, Value};

use crate::auth;
use crate::eval;

use super::common::{
    open_cli_connection, parse_flag_usize, parse_flag_value, validate_cli_options_or_exit,
};
"""

    BOOT_USES = """use serde_json::Value;
use std::time::Duration;

use crate::auth;
use crate::daemon_lifecycle::daemon_healthy;
use crate::transport;

use super::common::{
    ensure_remote_target_has_api_key, is_local_client_base_url, local_daemon_base_url,
    parse_flag_usize, parse_flag_value, resolve_client_target, validate_cli_options,
};
use super::daemon::ensure_daemon;
"""

    DAEMON_USES = """use chrono::Utc;
use fs2::FileExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::admin;
use crate::aging;
use crate::auth;
use crate::budgets;
use crate::compaction;
use crate::crystallize;
use crate::db;
use crate::daemon_lifecycle;
use crate::embeddings;
use crate::indexer;
use crate::server;
use crate::state;
use crate::transport;

use super::cleanup::{
    cleanup_backup_retention, cleanup_bridge_backups, cleanup_expired_rows, rotate_startup_logs,
};
use super::common::{
    env_trimmed, parse_env_u64, parse_env_usize, parse_truthy_flag,
    single_daemon_test_bypass_enabled,
};
"""

    # common.rs
    common_body = lines(text, 4210, 4423)
    common_body += "\n\n" + lines(text, 3808, 3816)
    common_body += "\n\n" + lines(text, 5389, 5508)
    write_module("common", COMMON_USES, common_body)

    # cleanup.rs (backup/log/bridge + run_cleanup_cli)
    cleanup_body = lines(text, 97, 446)
    cleanup_body += "\n\n" + lines(text, 2274, 2429)
    write_module("cleanup", CLEANUP_USES, cleanup_body)

    # status.rs
    status_body = "pub(crate) const STATUS_SCHEMA_VERSION: u32 = 1;\n\n"
    status_body += lines(text, 1505, 1951)
    write_module("status", STATUS_USES, status_body)

    # usage.rs
    usage_body = 'pub(crate) const CLI_CAPABILITIES_CONTRACT_VERSION: &str = "1";\n\n'
    usage_body += lines(text, 1952, 2273)
    write_module("usage", USAGE_USES, usage_body)

    # admin.rs (budgets + rollback)
    admin_body = lines(text, 1417, 1503)
    admin_body += "\n\n" + lines(text, 2617, 2747)
    write_module("admin", ADMIN_USES, admin_body)

    write_module("doctor", DOCTOR_USES, lines(text, 2430, 2616))
    write_module("reindex", REINDEX_USES, lines(text, 2748, 2947))
    write_module("embeddings", EMBEDDINGS_USES, lines(text, 2948, 3148))
    write_module("eval", EVAL_USES, lines(text, 3200, 3375))

    sync_body = lines(text, 3149, 3198)
    sync_body += "\n\n" + lines(text, 3377, 4208)
    write_module("sync", SYNC_USES, sync_body)

    boot_body = "const DEFAULT_BOOT_BUDGET: usize = 600;\n\n"
    boot_body += lines(text, 4442, 4585)
    write_module("boot", BOOT_USES, boot_body)

    daemon_body = lines(text, 100, 142)
    daemon_body += "\n\n" + lines(text, 4432, 4440)
    daemon_body += "\n\n" + lines(text, 4587, 5387)
    daemon_body += "\n\n" + lines(text, 5510, 6245)
    write_module("daemon", DAEMON_USES, daemon_body)

    tests_body = lines(text, 6248, 8402)
    tests_body = tests_body.replace("use super::*;", "use crate::cli::*;\n    use crate::*;")
    write_module(
        "tests",
        "#[cfg(test)]\nmod tests {\n",
        tests_body,
    )

    print("Done extracting modules.")


if __name__ == "__main__":
    main()

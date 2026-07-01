#!/usr/bin/env python3
"""Split remaining large daemon-rs modules."""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
SPDX = "// SPDX-License-Identifier: MIT\n"
SUPER = "\nuse super::*;\n"


@dataclass
class SplitSpec:
    src: Path
    dest: Path
    header: str
    segments: list[tuple[str, int, int]]
    test_range: tuple[int, int] | None = None
    pub_exports: list[str] | None = None
    super_use: bool = True
    pub_crate: bool = True


def lines(path: Path) -> list[str]:
    return path.read_text(encoding="utf-8").splitlines(keepends=True)


def slice_lines(all_lines: list[str], start: int, end: int) -> str:
    return "".join(all_lines[start - 1 : end])


def strip_tests(content: str) -> str:
    content = re.sub(r"^#\[allow\(clippy::items_after_test_module\)\]\n", "", content)
    content = re.sub(r"^#\[cfg\(test\)\]\n", "", content)
    content = re.sub(r"^mod tests \{\n", "", content)
    s = content.rstrip()
    if s.endswith("}"):
        content = s[:-1] + "\n"
    return content


def add_pub_crate(content: str) -> str:
    out: list[str] = []
    pending: list[str] = []
    for line in content.splitlines(keepends=True):
        s = line.lstrip()
        if s.startswith("#[") and not s.startswith("#[cfg(test)]"):
            pending.append(line)
            continue
        if (
            s.startswith("pub fn ")
            or s.startswith("pub async fn ")
            or s.startswith("pub struct ")
            or s.startswith("pub enum ")
            or s.startswith("pub const ")
            or s.startswith("pub type ")
            or s.startswith("pub(crate) ")
            or s.startswith("pub(super) ")
        ):
            out.extend(pending)
            pending = []
            out.append(line)
        elif s.startswith("async fn "):
            out.extend(pending)
            pending = []
            out.append(line.replace("async fn ", "pub(crate) async fn ", 1))
        elif s.startswith("fn "):
            out.extend(pending)
            pending = []
            out.append(line.replace("fn ", "pub(crate) fn ", 1))
        elif s.startswith("enum ") or s.startswith("struct "):
            out.extend(pending)
            pending = []
            out.append(
                line.replace("enum ", "pub(crate) enum ", 1).replace(
                    "struct ", "pub(crate) struct ", 1
                )
            )
        elif s.startswith("const ") and not s.startswith("pub "):
            out.extend(pending)
            pending = []
            out.append(line.replace("const ", "pub(crate) const ", 1))
        elif s.startswith("type ") and not s.startswith("pub "):
            out.extend(pending)
            pending = []
            out.append(line.replace("type ", "pub(crate) type ", 1))
        else:
            out.extend(pending)
            pending = []
            out.append(line)
    out.extend(pending)
    return "".join(out)


def apply(spec: SplitSpec) -> None:
    if not spec.src.exists():
        print(f"skip missing {spec.src}")
        return
    all_lines = lines(spec.src)
    names = [n for n, _, _ in spec.segments]
    spec.dest.mkdir(parents=True, exist_ok=True)
    for name, start, end in spec.segments:
        body = slice_lines(all_lines, start, end)
        if spec.pub_crate:
            body = add_pub_crate(body)
        extra = SUPER if spec.super_use else ""
        (spec.dest / f"{name}.rs").write_text(SPDX + spec.header + extra + body, encoding="utf-8")
    if spec.test_range:
        t0, t1 = spec.test_range
        tb = strip_tests(slice_lines(all_lines, t0, t1))
        (spec.dest / "tests.rs").write_text(SPDX + SUPER + tb, encoding="utf-8")
    mod = [SPDX]
    for n in names:
        mod.append(f"mod {n};\n")
    if spec.test_range:
        mod.append("\n#[cfg(test)]\nmod tests;\n")
    mod.append("\n")
    for n in names:
        mod.append(f"pub(crate) use {n}::*;\n")
    if spec.pub_exports:
        mod.append("\n")
        for e in spec.pub_exports:
            mod.append(f"pub use {e};\n")
    (spec.dest / "mod.rs").write_text("".join(mod), encoding="utf-8")
    spec.src.unlink()
    print(f"split {spec.src} -> {spec.dest.name}/ ({len(names)} files)")


CONDUCTOR_HEADER = """use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::checkpoint_wal_best_effort;
use crate::handlers::{
    ensure_auth_rated, json_response, now_iso, parse_duration_to_seconds, parse_json_array,
    parse_timestamp_ms, redact_secrets, resolve_caller_id,
};
use crate::state::RuntimeState;

"""

MCP_PROXY_HEADER = """use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use sysinfo::{ProcessesToUpdate, System};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::auth::CortexPaths;
use crate::daemon_lifecycle;

"""

COMPILER_HEADER = """use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::env;
use std::path::Path;

use crate::handlers::{estimate_tokens, estimate_tokens_from_chars};

"""

SERVER_HEADER = """use axum::body::Bytes;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tower::Service;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::CorsLayer;

use crate::budgets::BudgetEndpoint;
use crate::handlers;
use crate::handlers::mcp::handle_mcp_message_with_caller;
use crate::state::RuntimeState;

"""

CLI_DAEMON_HEADER = """use chrono::Utc;
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


def specs() -> list[SplitSpec]:
    return [
        SplitSpec(
            SRC / "handlers/conductor.rs",
            SRC / "handlers/conductor",
            CONDUCTOR_HEADER,
            [
                ("types", 19, 139),
                ("helpers", 140, 499),
                ("locks", 500, 739),
                ("activity", 740, 870),
                ("messages", 871, 953),
                ("sessions", 954, 1222),
                ("tasks", 1223, 1822),
            ],
            (1823, 1841),
            [
                "locks::{handle_lock, handle_unlock, handle_locks}",
                "activity::{handle_post_activity, handle_get_activity}",
                "messages::{handle_post_message, handle_get_messages}",
                "sessions::{handle_session_start, handle_session_heartbeat, handle_session_end, handle_sessions}",
                "tasks::{handle_create_task, handle_get_tasks, handle_claim_task, handle_complete_task, handle_delete_task, handle_abandon_task, handle_next_task}",
            ],
        ),
        SplitSpec(
            SRC / "mcp_proxy.rs",
            SRC / "mcp_proxy",
            MCP_PROXY_HEADER,
            [
                ("session", 18, 927),
                ("run", 928, 1554),
            ],
            (1555, 1775),
            ["run::run", "session::read_auth_token"],
            pub_crate=False,
        ),
        SplitSpec(
            SRC / "compiler.rs",
            SRC / "compiler",
            COMPILER_HEADER,
            [
                ("types", 23, 80),
                ("cache", 82, 212),
                ("capsules", 213, 708),
                ("ranking", 709, 945),
                ("packing", 946, 1338),
                ("compile", 1339, 1474),
            ],
            (1475, 1676),
            ["compile::compile", "types::BootResult"],
        ),
        SplitSpec(
            SRC / "server.rs",
            SRC / "server",
            SERVER_HEADER,
            [
                ("router", 23, 355),
                ("handlers", 356, 545),
                ("runtime", 546, 1052),
            ],
            (1053, 1552),
            ["router::build_router", "runtime::run"],
        ),
        SplitSpec(
            SRC / "cli/daemon.rs",
            SRC / "cli/daemon",
            CLI_DAEMON_HEADER,
            [
                ("startup", 41, 881),
                ("run", 883, 1420),
                ("backfill", 1422, 1621),
            ],
            None,
            [
                "run::{run_daemon, ensure_daemon, startup_single_daemon_preflight, is_disallowed_startup_binary_path}",
                "backfill::{EmbeddingBackfillPassResult, build_embeddings_async, count_unembedded_targets_for_model}",
                "startup::{background_db_lock_max_wait, backfill_batch_may_have_more, collect_unembedded_targets_for_model, DEFAULT_EMBED_BACKFILL_BATCH_SIZE, DEFAULT_EMBED_BACKFILL_MAX_BATCHES_PER_PASS}",
            ],
        ),
    ]


def main() -> None:
    only = sys.argv[1:] if len(sys.argv) > 1 else ["all"]
    for spec in specs():
        key = spec.src.stem
        if only != ["all"] and key not in only and spec.dest.name not in only:
            continue
        apply(spec)


if __name__ == "__main__":
    main()

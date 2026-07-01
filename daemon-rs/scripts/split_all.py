#!/usr/bin/env python3
"""Split large Rust modules into subdirectories."""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
SPDX = "// SPDX-License-Identifier: MIT\n"
SUPER_USE = "\nuse super::*;\n"


@dataclass
class SplitSpec:
    src: Path
    dest: Path
    header: str
    segments: list[tuple[str, int, int]]
    test_range: tuple[int, int] | None = None
    pub_exports: list[str] | None = None
    super_use: bool = True


def lines(path: Path) -> list[str]:
    return path.read_text().splitlines(keepends=True)


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


def pub_crate_fns(content: str) -> str:
    out: list[str] = []
    pending: list[str] = []
    for line in content.splitlines(keepends=True):
        s = line.lstrip()
        if s.startswith("#[") and not s.startswith("#[cfg(test)]"):
            pending.append(line)
            continue
        if s.startswith("pub fn ") or s.startswith("pub async fn ") or s.startswith("pub struct ") or s.startswith("pub enum ") or s.startswith("pub const ") or s.startswith("pub type "):
            out.extend(pending)
            pending = []
            out.append(line)
        elif s.startswith("fn ") or s.startswith("async fn "):
            out.extend(pending)
            pending = []
            out.append(line.replace("fn ", "pub(crate) fn ", 1).replace("async fn ", "pub(crate) async fn ", 1))
        elif s.startswith("enum ") or s.startswith("struct "):
            out.extend(pending)
            pending = []
            out.append(line.replace("enum ", "pub(crate) enum ", 1).replace("struct ", "pub(crate) struct ", 1))
        elif s.startswith("const ") and not s.startswith("pub "):
            out.extend(pending)
            pending = []
            out.append(line.replace("const ", "pub(crate) const ", 1))
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
        body = pub_crate_fns(slice_lines(all_lines, start, end))
        extra = SUPER_USE if spec.super_use else ""
        (spec.dest / f"{name}.rs").write_text(SPDX + spec.header + extra + body, encoding="utf-8")
    if spec.test_range:
        t0, t1 = spec.test_range
        tb = strip_tests(slice_lines(all_lines, t0, t1))
        (spec.dest / "tests.rs").write_text(SPDX + SUPER_USE + tb, encoding="utf-8")
    mod = [SPDX]
    for n in names:
        mod.append(f"mod {n};\n")
    mod.append("\n#[cfg(test)]\nmod tests;\n\n")
    for n in names:
        mod.append(f"pub(crate) use {n}::*;\n")
    if spec.pub_exports:
        mod.append("\n")
        for e in spec.pub_exports:
            mod.append(f"pub use {e};\n")
    (spec.dest / "mod.rs").write_text("".join(mod), encoding="utf-8")
    spec.src.unlink()
    print(f"split {spec.src.name} -> {spec.dest.name}/ ({len(names)} files)")


HEADERS = {
    "health": """use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::Utc;
use rusqlite::{params, OpenFlags};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use crate::handlers::{client_ip, ensure_auth_rated, ensure_ssrf_protection, json_response, truncate_chars};
use crate::state::RuntimeState;

""",
    "store": """use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use crate::handlers::{ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget, json_response, log_event, now_iso, resolve_source_identity, truncate_chars};
use crate::api_types::{RetentionClass, StoreRequest};
use crate::budgets::BudgetEndpoint;
use crate::conflict::{detect_conflict, jaccard_similarity, ConflictClassification, ConflictResult};
use crate::db::checkpoint_wal_best_effort;
use crate::rate_limit::RequestClass;
use crate::state::RuntimeState;

""",
    "db": """use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use rusqlite::{params, Connection, OptionalExtension};

""",
    "compaction": """use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

""",
    "conductor": """use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::handlers::{ensure_auth_rated, json_response, now_iso, resolve_source_identity};
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

""",
    "mcp_proxy": """use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use crate::state::RuntimeState;

""",
    "compiler": """use std::collections::HashSet;
use std::env;
use std::path::Path;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use crate::handlers::{estimate_tokens, estimate_tokens_from_chars};

""",
    "handlers_mod": """use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Duration, NaiveDateTime, TimeZone, Utc};
use regex::Regex;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::OnceLock;
use crate::budgets::{BudgetDecision, BudgetEndpoint};
use crate::rate_limit::RequestClass;
use crate::state::RuntimeState;

""",
    "mutate": """use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use crate::handlers::{ensure_auth_with_caller_rated, json_response, log_event, now_iso, resolve_source_identity, truncate_chars};
use crate::db::{archive_entries_scoped, checkpoint_wal_best_effort};
use crate::state::RuntimeState;

""",
    "server": """use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use crate::handlers;
use crate::state::RuntimeState;
use crate::transport;

""",
}


def specs() -> list[SplitSpec]:
    return [
        SplitSpec(SRC / "handlers/health.rs", SRC / "handlers/health", HEADERS["health"], [
            ("metrics", 15, 224), ("health", 225, 575), ("digest", 576, 797),
            ("savings_build", 798, 1348), ("savings", 1349, 1989), ("stats", 1990, 2019), ("dump", 2020, 2216),
        ], (2217, 2923), [
            "health::{build_health_payload, handle_health, build_readiness_payload, handle_readiness}",
            "digest::{build_digest, handle_digest}", "savings::handle_savings", "stats::handle_stats", "dump::handle_dump",
        ]),
        SplitSpec(SRC / "handlers/store.rs", SRC / "handlers/store", HEADERS["store"], [
            ("types", 22, 231), ("handler", 232, 365), ("core", 366, 797),
            ("policies", 798, 1186), ("insert", 1187, 1520), ("merge", 1521, 1777), ("embedding", 1778, 1793),
        ], (1794, 2625), [
            "handler::handle_store",
            "core::{store_decision, store_decision_with_ttl, store_decision_with_input_embedding, store_decision_with_input_embedding_and_provenance, store_decision_with_input_embedding_and_provenance_retention}",
            "types::{DecisionProvenance, validate_explicit_ttl_seconds}", "embedding::persist_decision_embedding",
        ]),
        SplitSpec(SRC / "db.rs", SRC / "db", HEADERS["db"], [
            ("connection", 10, 172), ("migrations", 173, 780), ("schema", 781, 1129),
            ("team", 1130, 1584), ("maintenance", 1585, 2083),
        ], (2086, 2860), [
            "connection::{open, configure, sqlite_vec_status, SQLITE_BUSY_TIMEOUT_MS, SQLITE_WAL_AUTOCHECKPOINT_PAGES}",
            "migrations::{migration_definitions, latest_schema_user_version, current_schema_user_version, set_schema_user_version, ensure_schema_migrations_table, applied_migration_versions, pending_migration_versions, run_pending_migrations, run_pending_migrations_quiet, initialize_schema}",
            "team::{current_mode, is_team_mode, migration_counts, create_team_mode_tables, upsert_owner_user, migrate_to_team_mode, ensure_default_team_membership, table_exists, migrate_focus_table}",
            "maintenance::{checkpoint_wal_best_effort, delete_expired_entries, ExpiredCleanupCounts, rebuild_fts, reindex_fts, rebuild_fts_if_needed, verify_integrity, quick_check, auto_repair, RepairResult, RepairError, archive_entries_scoped, archive_entries}",
        ]),
        SplitSpec(SRC / "compaction.rs", SRC / "compaction", HEADERS["compaction"], [
            ("types", 16, 119), ("governor", 120, 616), ("events", 617, 1004),
            ("archived", 1005, 1071), ("crystals", 1072, 1135), ("feedback", 1136, 1367), ("helpers", 1368, 1422),
        ], (1425, 2660), [
            "types::*", "governor::{should_run_compaction_governor, run_compaction_governor, run_compaction_governor_startup, fts_segment_row_total, FTS_SEGMENT_ROW_SOFT_LIMIT}",
            "events::{run_compaction, purge_benchmark_artifacts, CompactionResult, BenchmarkPurgeResult}", "helpers::storage_breakdown",
        ]),
        SplitSpec(SRC / "handlers/conductor.rs", SRC / "handlers/conductor", HEADERS["conductor"], [
            ("types", 36, 501), ("locks", 502, 872), ("sessions", 873, 1224), ("tasks", 1225, 1821),
        ], (1822, 1841), [
            "locks::{handle_lock, handle_unlock, handle_locks, handle_post_activity, handle_get_activity}",
            "sessions::{handle_post_message, handle_get_messages, handle_session_start, handle_session_heartbeat, handle_session_end, handle_sessions}",
            "tasks::{handle_create_task, handle_get_tasks, handle_claim_task, handle_complete_task, handle_delete_task, handle_abandon_task, handle_next_task}",
        ]),
        SplitSpec(SRC / "mcp_proxy.rs", SRC / "mcp_proxy", HEADERS["mcp_proxy"], [
            ("core", 52, 802), ("transport", 803, 1553),
        ], (1554, 1775), ["core::*", "transport::*"]),
        SplitSpec(SRC / "compiler.rs", SRC / "compiler", HEADERS["compiler"], [
            ("core", 23, 1338), ("compile", 1339, 1473),
        ], (1474, 1676), ["core::BootResult", "compile::compile"]),
        SplitSpec(SRC / "handlers/mutate.rs", SRC / "handlers/mutate", HEADERS["mutate"], [
            ("types", 18, 205), ("permissions", 206, 717), ("conflicts", 718, 1337),
        ], (1338, 1591), [
            "types::*", "permissions::{list_permissions, grant_permission, revoke_permission}",
            "conflicts::{parse_conflict_id, list_conflicts_payload, forget_keyword_scoped, resolve_decision, resolve_decision_with_metadata}",
        ]),
        SplitSpec(SRC / "server.rs", SRC / "server", HEADERS["server"], [
            ("router", 23, 669), ("routes", 670, 1051),
        ], (1052, 1552), ["router::build_router"]),
    ]


def main() -> None:
    only = sys.argv[1:] if len(sys.argv) > 1 else ["all"]
    for spec in specs():
        if only != ["all"] and spec.dest.name not in only and spec.src.stem not in only:
            continue
        apply(spec)


if __name__ == "__main__":
    main()

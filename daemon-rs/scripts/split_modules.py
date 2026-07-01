#!/usr/bin/env python3
"""Split large Rust modules into subdirectories with tests extracted."""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
SPDX = "// SPDX-License-Identifier: MIT\n"


@dataclass
class SplitSpec:
    src_file: Path
    dest_dir: Path
    segments: list[tuple[str, int, int]]
    imports: str
    test_start: int | None = None
    test_end: int | None = None
    pub_reexports: list[str] | None = None
    test_imports: str = "use super::*;\n"


def read_lines(path: Path) -> list[str]:
    return path.read_text(encoding="utf-8").splitlines(keepends=True)


def extract_range(lines: list[str], start: int, end: int) -> str:
    return "".join(lines[start - 1 : end])


def strip_test_wrapper(content: str) -> str:
    content = re.sub(r"^#\[allow\(clippy::items_after_test_module\)\]\n", "", content)
    content = re.sub(r"^#\[cfg\(test\)\]\n", "", content)
    content = re.sub(r"^mod tests \{\n", "", content)
    stripped = content.rstrip()
    if stripped.endswith("}"):
        content = stripped[:-1] + "\n"
    return content


def write_file(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if not content.endswith("\n"):
        content += "\n"
    path.write_text(content, encoding="utf-8")


def build_mod_rs(
    segments: list[str],
    pub_reexports: list[str] | None,
    has_tests: bool,
) -> str:
    lines = [SPDX]
    for seg in segments:
        lines.append(f"mod {seg};\n")
    if pub_reexports:
        lines.append("\n")
        for item in pub_reexports:
            lines.append(f"pub use {item};\n")
    if has_tests:
        lines.append("\n#[cfg(test)]\nmod tests;\n")
    return "".join(lines)


def add_pub_crate(content: str) -> str:
    lines = content.splitlines(keepends=True)
    out: list[str] = []
    pending_attrs: list[str] = []
    for line in lines:
        stripped = line.lstrip()
        if stripped.startswith("#[") and not stripped.startswith("#[cfg(test)]"):
            pending_attrs.append(line)
            continue
        if stripped.startswith("fn ") and not stripped.startswith("fn test_"):
            out.extend(pending_attrs)
            pending_attrs = []
            out.append(line.replace("fn ", "pub(crate) fn ", 1))
        elif stripped.startswith("async fn "):
            out.extend(pending_attrs)
            pending_attrs = []
            out.append(line.replace("async fn ", "pub(crate) async fn ", 1))
        elif stripped.startswith("enum "):
            out.extend(pending_attrs)
            pending_attrs = []
            out.append(line.replace("enum ", "pub(crate) enum ", 1))
        elif stripped.startswith("struct ") and not stripped.startswith("struct Test"):
            out.extend(pending_attrs)
            pending_attrs = []
            out.append(line.replace("struct ", "pub(crate) struct ", 1))
        elif stripped.startswith("pub fn ") or stripped.startswith("pub async fn ") or stripped.startswith("pub struct ") or stripped.startswith("pub enum ") or stripped.startswith("pub const "):
            out.extend(pending_attrs)
            pending_attrs = []
            out.append(line)
        else:
            out.extend(pending_attrs)
            pending_attrs = []
            out.append(line)
    out.extend(pending_attrs)
    return "".join(out)


def apply_split(spec: SplitSpec, pub_crate: bool = True) -> None:
    lines = read_lines(spec.src_file)
    segment_names = [name for name, _, _ in spec.segments]

    for name, start, end in spec.segments:
        body = extract_range(lines, start, end)
        if pub_crate:
            body = add_pub_crate(body)
        write_file(spec.dest_dir / f"{name}.rs", SPDX + spec.imports + body)

    if spec.test_start is not None and spec.test_end is not None:
        test_body = extract_range(lines, spec.test_start, spec.test_end)
        test_body = strip_test_wrapper(test_body)
        write_file(
            spec.dest_dir / "tests.rs",
            SPDX + spec.test_imports + test_body,
        )

    write_file(
        spec.dest_dir / "mod.rs",
        build_mod_rs(segment_names, spec.pub_reexports, spec.test_start is not None),
    )
    spec.src_file.unlink()
    print(f"Split {spec.src_file.name} -> {spec.dest_dir.name}/")


MCP_IMPORTS = """\
use chrono::{Duration, Utc};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;

use crate::handlers::diary::{write_diary_entry, DiaryRequest};
use crate::handlers::feedback::{
    build_agent_feedback_stats_payload, recommend_recall_k, record_agent_feedback_from_value,
};
use crate::handlers::health::{build_digest, build_health_payload};
use crate::handlers::mutate::{
    forget_keyword_scoped, list_conflicts_payload, parse_conflict_id, resolve_decision,
    resolve_decision_with_metadata, ConflictListOptions, ConflictStatusFilter, ResolutionMetadata,
};
use crate::handlers::recall::{
    execute_recall_policy_explain, execute_semantic_recall, execute_unified_recall,
    parse_recall_policy_mode, resolve_recall_budget_k, unfold_source, RecallContext,
};
use crate::handlers::store::{
    persist_decision_embedding, store_decision_with_input_embedding_and_provenance_retention,
    validate_explicit_ttl_seconds, DecisionProvenance,
};
use crate::handlers::{estimate_tokens, now_iso, SourceIdentity};
use crate::api_types::RetentionClass;
use crate::state::RuntimeState;
use crate::{aging, db, indexer};

"""

HEALTH_IMPORTS = """\
use axum::extract::State;
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

"""

STORE_IMPORTS = """\
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::handlers::{
    ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget, json_response, log_event,
    now_iso, resolve_source_identity, truncate_chars,
};
use crate::api_types::{RetentionClass, StoreRequest};
use crate::budgets::BudgetEndpoint;
use crate::conflict::{
    detect_conflict, jaccard_similarity, ConflictClassification, ConflictResult,
};
use crate::db::checkpoint_wal_best_effort;
use crate::rate_limit::RequestClass;
use crate::state::RuntimeState;

"""

DB_IMPORTS = """\
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

"""

COMPACTION_IMPORTS = """\
use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

"""

HANDLERS_MOD_IMPORTS = """\
use axum::http::{HeaderMap, HeaderValue, StatusCode};
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

"""

MUTATE_IMPORTS = """\
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::handlers::{
    ensure_auth_with_caller_rated, json_response, log_event, now_iso, resolve_source_identity,
    truncate_chars,
};
use crate::db::{archive_entries_scoped, checkpoint_wal_best_effort};
use crate::state::RuntimeState;

"""

SERVER_IMPORTS = """\
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::handlers;
use crate::state::RuntimeState;
use crate::transport;

"""

COMPILER_IMPORTS = """\
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

"""

MCP_PROXY_IMPORTS = """\
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::state::RuntimeState;

"""

CONDUCTOR_IMPORTS = """\
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::handlers::{ensure_auth_with_caller_rated, json_response, now_iso, resolve_source_identity};
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

"""


def all_specs() -> dict[str, SplitSpec]:
    return {
        "health": SplitSpec(
            src_file=SRC / "handlers" / "health.rs",
            dest_dir=SRC / "handlers" / "health",
            imports=HEALTH_IMPORTS,
            segments=[
                ("metrics", 15, 224),
                ("health", 225, 575),
                ("digest", 576, 797),
                ("savings_helpers", 800, 1033),
                ("savings_stats", 1034, 1348),
                ("savings", 1349, 1989),
                ("stats", 1990, 2019),
                ("dump", 2020, 2216),
            ],
            test_start=2217,
            test_end=2923,
            pub_reexports=[
                "health::{build_health_payload, handle_health, build_readiness_payload, handle_readiness}",
                "digest::{build_digest, handle_digest}",
                "savings::handle_savings",
                "stats::handle_stats",
                "dump::handle_dump",
            ],
        ),
        "store": SplitSpec(
            src_file=SRC / "handlers" / "store.rs",
            dest_dir=SRC / "handlers" / "store",
            imports=STORE_IMPORTS,
            segments=[
                ("types", 22, 231),
                ("handler", 232, 365),
                ("core", 366, 797),
                ("policies", 798, 1186),
                ("insert", 1187, 1520),
                ("merge", 1521, 1777),
                ("embedding", 1778, 1793),
            ],
            test_start=1794,
            test_end=2625,
            pub_reexports=[
                "handler::handle_store",
                "core::{store_decision, store_decision_with_ttl, store_decision_with_input_embedding, store_decision_with_input_embedding_and_provenance, store_decision_with_input_embedding_and_provenance_retention}",
                "types::{DecisionProvenance, validate_explicit_ttl_seconds}",
                "embedding::persist_decision_embedding",
            ],
        ),
        "db": SplitSpec(
            src_file=SRC / "db.rs",
            dest_dir=SRC / "db",
            imports=DB_IMPORTS,
            segments=[
                ("connection", 10, 172),
                ("migrations", 173, 783),
                ("schema", 784, 1130),
                ("team", 1131, 1584),
                ("maintenance", 1585, 2083),
            ],
            test_start=2086,
            test_end=2860,
            pub_reexports=[
                "connection::{open, configure, sqlite_vec_status, SQLITE_BUSY_TIMEOUT_MS, SQLITE_WAL_AUTOCHECKPOINT_PAGES}",
                "migrations::{migration_definitions, latest_schema_user_version, current_schema_user_version, set_schema_user_version, ensure_schema_migrations_table, applied_migration_versions, pending_migration_versions, run_pending_migrations, run_pending_migrations_quiet, initialize_schema}",
                "team::{current_mode, is_team_mode, migration_counts, create_team_mode_tables, upsert_owner_user, migrate_to_team_mode, ensure_default_team_membership, table_exists, migrate_focus_table}",
                "maintenance::{checkpoint_wal_best_effort, delete_expired_entries, ExpiredCleanupCounts, rebuild_fts, reindex_fts, rebuild_fts_if_needed, verify_integrity, quick_check, auto_repair, RepairResult, RepairError, archive_entries_scoped, archive_entries}",
            ],
        ),
        "compaction": SplitSpec(
            src_file=SRC / "compaction.rs",
            dest_dir=SRC / "compaction",
            imports=COMPACTION_IMPORTS,
            segments=[
                ("constants", 16, 119),
                ("governor", 120, 616),
                ("main", 617, 1004),
                ("archived", 1005, 1071),
                ("crystals", 1072, 1135),
                ("feedback", 1136, 1367),
                ("helpers", 1368, 1422),
            ],
            test_start=1425,
            test_end=2660,
            pub_reexports=[
                "constants::*",
                "governor::{should_run_compaction_governor, run_compaction_governor, run_compaction_governor_startup, fts_segment_row_total, FTS_SEGMENT_ROW_SOFT_LIMIT}",
                "main::{run_compaction, purge_benchmark_artifacts, CompactionResult, BenchmarkPurgeResult}",
                "helpers::storage_breakdown",
            ],
        ),
    }


def main() -> None:
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    specs = all_specs()
    if which != "all":
        if which not in specs:
            print(f"Unknown module: {which}")
            sys.exit(1)
        specs = {which: specs[which]}
    for spec in specs.values():
        if not spec.src_file.exists():
            print(f"SKIP (missing): {spec.src_file}")
            continue
        apply_split(spec)


if __name__ == "__main__":
    main()

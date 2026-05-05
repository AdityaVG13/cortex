#!/usr/bin/env python3
"""Run the local Cortex R2 boot truncation gate.

This gate compares the current score-adaptive boot packer against the legacy
greedy packer on the same deterministic corpus. It drives the real `/boot`
HTTP path so latency, token usage, and C5 boot-audit metadata are measured from
the daemon rather than from a standalone model of the allocator.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import platform
import shutil
import socket
import sqlite3
import statistics
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


HIGH_SENTINELS = [
    "R2_GT_HIGH_01_ARCHITECTURE",
    "R2_GT_HIGH_02_SECURITY",
    "R2_GT_HIGH_03_ACCESSIBILITY",
    "R2_GT_HIGH_04_GOVERNANCE",
    "R2_GT_HIGH_05_RECALL",
    "R2_GT_HIGH_06_RELEASE",
]

LOW_SENTINELS = [
    "R2_GT_LOW_01_DECOY",
    "R2_GT_LOW_02_DECOY",
    "R2_GT_LOW_03_DECOY",
    "R2_GT_LOW_04_DECOY",
    "R2_GT_LOW_05_DECOY",
    "R2_GT_LOW_06_DECOY",
]

APP_ENV_KEYS = [
    "CORTEX_APP_REQUIRED",
    "CORTEX_DAEMON_OWNER_LOCAL_SPAWN",
    "CORTEX_APP_CLIENT",
]


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def now_tag() -> str:
    return dt.datetime.now().strftime("%Y%m%d-%H%M%S")


def run(args: list[str], cwd: Path, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    return subprocess.run(
        args,
        cwd=str(cwd),
        env=merged_env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )


def git_value(root: Path, *args: str) -> str:
    proc = run(["git", *args], root)
    return proc.stdout.strip()


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def ensure_binary(root: Path, target_dir: Path, skip_build: bool) -> Path:
    exe = "cortex.exe" if os.name == "nt" else "cortex"
    binary = target_dir / "debug" / exe
    if skip_build:
        if binary.exists():
            return binary
        raise RuntimeError(f"--skip-build set but expected binary is missing: {binary}")
    env = {"CARGO_TARGET_DIR": str(target_dir)}
    try:
        proc = run(["rtk", "cargo", "build", "--manifest-path", "daemon-rs/Cargo.toml"], root, env=env)
    except FileNotFoundError:
        proc = None
    if proc is None or proc.returncode != 0:
        proc = run(["cargo", "build", "--manifest-path", "daemon-rs/Cargo.toml"], root, env=env)
    if proc.returncode != 0:
        raise RuntimeError(f"cargo build failed:\n{proc.stdout}")
    if not binary.exists():
        raise RuntimeError(f"expected binary missing after build: {binary}")
    return binary


def http_json(
    method: str,
    url: str,
    *,
    token: str | None = None,
    timeout: float = 15.0,
) -> dict[str, Any]:
    headers = {"X-Cortex-Request": "true", "X-Source-Agent": "r2-boot-truncation-gate"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, headers=headers, method=method)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def wait_for_health(base_url: str, proc: subprocess.Popen[str]) -> None:
    deadline = time.time() + 30
    last_error = ""
    while time.time() < deadline:
        if proc.poll() is not None:
            stderr = proc.stderr.read() if proc.stderr else ""
            raise RuntimeError(f"daemon exited before health: code={proc.returncode}; {stderr}")
        try:
            http_json("GET", f"{base_url}/health", timeout=2)
            return
        except Exception as exc:  # noqa: BLE001 - diagnostic loop
            last_error = str(exc)
            time.sleep(0.2)
    raise RuntimeError(f"daemon did not become healthy: {last_error}")


def wait_for_token(home: Path, proc: subprocess.Popen[str]) -> str:
    token_path = home / "cortex.token"
    deadline = time.time() + 15
    while time.time() < deadline:
        if proc.poll() is not None:
            stderr = proc.stderr.read() if proc.stderr else ""
            raise RuntimeError(f"daemon exited before token: code={proc.returncode}; {stderr}")
        if token_path.exists():
            token = token_path.read_text(encoding="utf-8").strip()
            if token:
                return token
        time.sleep(0.1)
    raise RuntimeError("token file was not written")


def start_daemon(binary: Path, home: Path, port: int, args: argparse.Namespace, packing_mode: str) -> subprocess.Popen[str]:
    env = os.environ.copy()
    for key in APP_ENV_KEYS:
        env.pop(key, None)
    env.update(
        {
            "CORTEX_GLOBAL_LOCK_HOME": str(home / "global-lock"),
            "CORTEX_SINGLE_DAEMON_TEST_BYPASS": "1",
            "CORTEX_BOOT_PACKING_MODE": packing_mode,
            "CORTEX_BOOT_MIN_SOURCE_TOKENS": str(args.min_source_tokens),
            "CORTEX_BOOT_MAX_SOURCE_TOKENS": str(args.max_source_tokens),
            "CORTEX_BOOT_RANK_TOP_N": str(args.rank_top_n),
        }
    )
    return subprocess.Popen(
        [str(binary), "serve", "--home", str(home), "--port", str(port)],
        cwd=str(repo_root()),
        env=env,
        text=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )


def high_decision(sentinel: str, index: int) -> str:
    filler = (
        "This high-priority boot fact carries release-critical context, owner "
        "constraints, regression details, accessibility obligations, and recall "
        "quality evidence that should survive truncation. "
    )
    return f"{sentinel} ground-truth boot fact {index}. " + (filler * 5)


def low_decision(sentinel: str, index: int) -> str:
    return f"{sentinel} short audit decoy {index}; useful but not release-critical."


def seed_fixture(db_path: Path) -> dict[int, dict[str, str]]:
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0)
    old = dt.datetime(2025, 1, 1, 0, 0, 0, tzinfo=dt.timezone.utc)
    id_map: dict[int, dict[str, str]] = {}
    with sqlite3.connect(db_path) as conn:
        conn.execute("DELETE FROM decisions")
        rows: list[tuple[str, str, str, str, float, int, str, str, str]] = []
        for index, sentinel in enumerate(HIGH_SENTINELS, start=1):
            rows.append(
                (
                    high_decision(sentinel, index),
                    f"r2-fixture::ground-truth::{sentinel}",
                    "decision",
                    "durable",
                    0.99,
                    10,
                    now.strftime("%Y-%m-%d %H:%M:%S"),
                    now.strftime("%Y-%m-%d %H:%M:%S"),
                    sentinel,
                )
            )
        for index, sentinel in enumerate(LOW_SENTINELS, start=1):
            rows.append(
                (
                    low_decision(sentinel, index),
                    f"r2-fixture::decoy::{sentinel}",
                    "decision",
                    "operational",
                    0.70,
                    0,
                    old.strftime("%Y-%m-%d %H:%M:%S"),
                    now.strftime("%Y-%m-%d %H:%M:%S"),
                    sentinel,
                )
            )
        for decision, context, row_type, retention_class, score, retrievals, last_accessed, timestamp, sentinel in rows:
            cursor = conn.execute(
                """
                INSERT INTO decisions
                    (decision, context, type, source_agent, status, score, retrievals,
                     last_accessed, created_at, updated_at, retention_class)
                VALUES
                    (?1, ?2, ?3, 'r2-boot-truncation-gate', 'active', ?4, ?5,
                     ?6, ?7, ?7, ?8)
                """,
                (
                    decision,
                    context,
                    row_type,
                    score,
                    retrievals,
                    last_accessed,
                    timestamp,
                    retention_class,
                ),
            )
            row_id = int(cursor.lastrowid)
            id_map[row_id] = {
                "sentinel": sentinel,
                "group": "high" if sentinel in HIGH_SENTINELS else "low",
            }
        conn.commit()
    return id_map


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int(round((pct / 100.0) * (len(ordered) - 1)))))
    return ordered[index]


def prompt_score(prompt: str) -> dict[str, Any]:
    high_found = [sentinel for sentinel in HIGH_SENTINELS if sentinel in prompt]
    low_found = [sentinel for sentinel in LOW_SENTINELS if sentinel in prompt]
    denominator = len(high_found) + len(low_found)
    precision = (len(high_found) / denominator) if denominator else 0.0
    coverage = len(high_found) / len(HIGH_SENTINELS)
    return {
        "high_found": high_found,
        "low_found": low_found,
        "gt_precision": round(precision, 4),
        "gt_coverage": round(coverage, 4),
    }


def capsule_summary(capsules: list[dict[str, Any]], id_map: dict[int, dict[str, str]]) -> dict[str, Any]:
    high_allocations: list[int] = []
    low_allocations: list[int] = []
    high_tokens: list[int] = []
    low_tokens: list[int] = []
    packing_modes: dict[str, int] = {}
    for capsule in capsules:
        source_id = capsule.get("sourceId")
        if source_id is None:
            continue
        info = id_map.get(int(source_id))
        if not info:
            continue
        packing = str(capsule.get("packing", "legacy"))
        packing_modes[packing] = packing_modes.get(packing, 0) + 1
        allocated = capsule.get("allocatedTokens")
        tokens = capsule.get("tokens")
        if info["group"] == "high":
            if isinstance(allocated, int):
                high_allocations.append(allocated)
            if isinstance(tokens, int):
                high_tokens.append(tokens)
        else:
            if isinstance(allocated, int):
                low_allocations.append(allocated)
            if isinstance(tokens, int):
                low_tokens.append(tokens)
    return {
        "packing_modes": dict(sorted(packing_modes.items())),
        "high_allocated_total": sum(high_allocations),
        "high_allocated_avg": round(statistics.mean(high_allocations), 3) if high_allocations else 0.0,
        "low_allocated_total": sum(low_allocations),
        "low_allocated_avg": round(statistics.mean(low_allocations), 3) if low_allocations else 0.0,
        "high_tokens_total": sum(high_tokens),
        "high_tokens_avg": round(statistics.mean(high_tokens), 3) if high_tokens else 0.0,
        "low_tokens_total": sum(low_tokens),
        "low_tokens_avg": round(statistics.mean(low_tokens), 3) if low_tokens else 0.0,
        "high_capsules": len(high_tokens),
        "low_capsules": len(low_tokens),
    }


def audit_summary(db_path: Path) -> dict[str, Any]:
    with sqlite3.connect(db_path) as conn:
        rows = conn.execute(
            """
            SELECT latency_ms, capsules_json
            FROM boot_audits
            ORDER BY id
            """,
        ).fetchall()
    latencies = [float(row[0]) for row in rows]
    last_capsules = json.loads(rows[-1][1]) if rows else []
    return {
        "rows": len(rows),
        "latency_ms": {
            "p50": round(percentile(latencies, 50), 3) if latencies else 0.0,
            "p95": round(percentile(latencies, 95), 3) if latencies else 0.0,
            "max": round(max(latencies), 3) if latencies else 0.0,
        },
        "last_capsules": last_capsules,
    }


def boot_once(base_url: str, token: str, agent: str, budget: int, timeout: float) -> tuple[float, dict[str, Any]]:
    query = urllib.parse.urlencode({"agent": agent, "budget": str(budget), "profile": "full"})
    started = time.perf_counter()
    payload = http_json("GET", f"{base_url}/boot?{query}", token=token, timeout=timeout)
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    return elapsed_ms, payload


def run_mode(binary: Path, args: argparse.Namespace, name: str, packing_mode: str) -> dict[str, Any]:
    root = repo_root()
    temp_root = root / ".tmp" / f"r2-boot-{name}-{now_tag()}-{os.getpid()}"
    home = temp_root / "home"
    home.mkdir(parents=True, exist_ok=True)
    port = reserve_port()
    base_url = f"http://127.0.0.1:{port}"
    proc = start_daemon(binary, home, port, args, packing_mode)
    agent = f"r2-boot-truncation-{name}"
    try:
        wait_for_health(base_url, proc)
        token = wait_for_token(home, proc)
        id_map = seed_fixture(home / "cortex.db")

        for _ in range(args.warmups):
            boot_once(base_url, token, agent, args.budget, args.timeout)

        latencies: list[float] = []
        last_payload: dict[str, Any] = {}
        prompt_scores: list[dict[str, Any]] = []
        for _ in range(args.boots):
            elapsed_ms, payload = boot_once(base_url, token, agent, args.budget, args.timeout)
            latencies.append(elapsed_ms)
            last_payload = payload
            prompt_scores.append(prompt_score(str(payload.get("bootPrompt", ""))))

        audit = audit_summary(home / "cortex.db")
        capsules = last_payload.get("capsules") if isinstance(last_payload.get("capsules"), list) else []
        return {
            "mode": name,
            "packing_mode": packing_mode,
            "requests": {"warmups": args.warmups, "measured": args.boots},
            "latency_ms": {
                "p50_response": round(percentile(latencies, 50), 3),
                "p95_response": round(percentile(latencies, 95), 3),
                "max_response": round(max(latencies), 3) if latencies else 0.0,
            },
            "prompt_score_last": prompt_scores[-1] if prompt_scores else prompt_score(""),
            "prompt_score_min_precision": round(
                min((score["gt_precision"] for score in prompt_scores), default=0.0), 4
            ),
            "token_usage_last": last_payload.get("tokenUsage"),
            "capsule_summary_last": capsule_summary(capsules, id_map),
            "audit": {
                "rows": audit["rows"],
                "latency_ms": audit["latency_ms"],
                "capsule_summary_last": capsule_summary(audit["last_capsules"], id_map),
            },
        }
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
        shutil.rmtree(temp_root, ignore_errors=True)


def write_markdown(path: Path, summary: dict[str, Any]) -> None:
    checks = summary["checks"]
    legacy = summary["modes"]["legacy"]
    adaptive = summary["modes"]["score_adaptive"]
    text = f"""# R2 Boot Truncation Gate

Generated: {summary['generated_at']}

Result: **{'PASS' if summary['passed'] else 'FAIL'}**

This artifact compares the real `/boot` path under the legacy greedy packer and
the score-adaptive packer on the same deterministic fixture.

## Checks

- GT precision delta: {checks['gt_precision_delta']} (required >= {checks['min_gt_precision_gain']})
- p50 audit latency delta: {checks['p50_latency_delta_ms']} ms / {checks['p50_latency_delta_percent']}%
- p50 latency allowed delta: {checks['p50_latency_allowed_delta_ms']} ms
- C5 audit rows: legacy {legacy['audit']['rows']}, score-adaptive {adaptive['audit']['rows']}
- Adaptive allocation: high total {adaptive['capsule_summary_last']['high_tokens_total']} tokens, low total {adaptive['capsule_summary_last']['low_tokens_total']} tokens

## Mode Summary

| Mode | GT precision | GT coverage | p50 response ms | p95 response ms | Prompt tokens |
|------|--------------|-------------|-----------------|-----------------|---------------|
| Legacy greedy | {legacy['prompt_score_last']['gt_precision']} | {legacy['prompt_score_last']['gt_coverage']} | {legacy['latency_ms']['p50_response']} | {legacy['latency_ms']['p95_response']} | {legacy['token_usage_last']['used']} |
| Score-adaptive | {adaptive['prompt_score_last']['gt_precision']} | {adaptive['prompt_score_last']['gt_coverage']} | {adaptive['latency_ms']['p50_response']} | {adaptive['latency_ms']['p95_response']} | {adaptive['token_usage_last']['used']} |
"""
    path.write_text(text, encoding="utf-8")


def compare_modes(modes: dict[str, dict[str, Any]], args: argparse.Namespace) -> dict[str, Any]:
    legacy = modes["legacy"]
    adaptive = modes["score_adaptive"]
    legacy_precision = float(legacy["prompt_score_last"]["gt_precision"])
    adaptive_precision = float(adaptive["prompt_score_last"]["gt_precision"])
    precision_delta = round(adaptive_precision - legacy_precision, 4)
    legacy_p50 = float(legacy["audit"]["latency_ms"]["p50"])
    adaptive_p50 = float(adaptive["audit"]["latency_ms"]["p50"])
    latency_delta = round(adaptive_p50 - legacy_p50, 3)
    latency_delta_percent = round((latency_delta / legacy_p50) * 100.0, 3) if legacy_p50 > 0 else 0.0
    allowed_latency_delta = round(max(abs(legacy_p50) * (args.max_p50_delta_percent / 100.0), args.latency_jitter_ms), 3)
    expected_audit_rows = args.warmups + args.boots
    adaptive_capsules = adaptive["capsule_summary_last"]
    return {
        "gt_precision_delta": precision_delta,
        "min_gt_precision_gain": args.min_gt_precision_gain,
        "gt_precision_pass": precision_delta >= args.min_gt_precision_gain,
        "p50_latency_delta_ms": latency_delta,
        "p50_latency_delta_percent": latency_delta_percent,
        "p50_latency_allowed_delta_ms": allowed_latency_delta,
        "p50_latency_pass": abs(latency_delta) <= allowed_latency_delta,
        "expected_audit_rows_per_mode": expected_audit_rows,
        "audit_rows_pass": all(mode["audit"]["rows"] == expected_audit_rows for mode in modes.values()),
        "adaptive_allocation_pass": (
            adaptive_capsules["high_tokens_total"] > adaptive_capsules["low_tokens_total"]
            and adaptive_capsules["high_capsules"] >= len(HIGH_SENTINELS)
        ),
    }


def parse_args() -> argparse.Namespace:
    root = repo_root()
    parser = argparse.ArgumentParser(description="Run the Cortex R2 boot truncation gate.")
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument("--markdown-output", type=Path, default=None)
    parser.add_argument("--target-dir", type=Path, default=root / "daemon-rs" / "target-codex-r2-gate")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--boots", type=int, default=31)
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--budget", type=int, default=240)
    parser.add_argument("--rank-top-n", type=int, default=12)
    parser.add_argument("--min-source-tokens", type=int, default=24)
    parser.add_argument("--max-source-tokens", type=int, default=120)
    parser.add_argument("--timeout", type=float, default=15.0)
    parser.add_argument("--min-gt-precision-gain", type=float, default=0.02)
    parser.add_argument("--max-p50-delta-percent", type=float, default=5.0)
    parser.add_argument("--latency-jitter-ms", type=float, default=5.0)
    parser.add_argument("--json", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = repo_root()
    output = args.output or root / "benchmarking" / "results" / f"r2-boot-truncation-{now_tag()}.json"
    markdown_output = args.markdown_output or output.with_suffix(".md")
    binary = ensure_binary(root, args.target_dir, args.skip_build)

    modes = {
        "legacy": run_mode(binary, args, "legacy", "legacy"),
        "score_adaptive": run_mode(binary, args, "score-adaptive", "score-adaptive"),
    }
    checks = compare_modes(modes, args)
    passed = all(
        [
            checks["gt_precision_pass"],
            checks["p50_latency_pass"],
            checks["audit_rows_pass"],
            checks["adaptive_allocation_pass"],
        ]
    )
    summary = {
        "schema": "cortex.r2_boot_truncation_gate.v1",
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "git_commit": git_value(root, "rev-parse", "HEAD"),
        "platform": {
            "os": platform.platform(),
            "machine": platform.machine(),
            "python": sys.version.split()[0],
        },
        "config": {
            "boots": args.boots,
            "warmups": args.warmups,
            "budget": args.budget,
            "rank_top_n": args.rank_top_n,
            "min_source_tokens": args.min_source_tokens,
            "max_source_tokens": args.max_source_tokens,
            "packing_modes": ["legacy", "score-adaptive"],
            "latency_gate": f"+/- {args.max_p50_delta_percent}% with {args.latency_jitter_ms}ms jitter floor",
        },
        "fixture": {
            "high_sentinels": HIGH_SENTINELS,
            "low_sentinels": LOW_SENTINELS,
        },
        "modes": modes,
        "checks": checks,
        "passed": passed,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    write_markdown(markdown_output, summary)
    print(json.dumps(summary, indent=2, sort_keys=True) if args.json else f"R2 gate wrote {output}")
    return 0 if passed else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise
    except Exception as exc:  # noqa: BLE001 - command line tool
        print(f"ERROR: {exc}", file=sys.stderr)
        raise SystemExit(1)

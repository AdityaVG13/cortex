#!/usr/bin/env python3
"""Run the local Cortex RQ1 embedding profile gate.

This gate covers the parts of RQ1 that are locally measurable without judge API
credits: BGE backfill throughput and p50 recall latency versus the MiniLM
legacy profile on the existing deterministic recall corpus.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import shutil
import socket
import sqlite3
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


LEGACY_PROFILE = "all-MiniLM-L12-v2"
BGE_PROFILE = "bge-base-en-v1.5"


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


def extract_json_line(output: str) -> dict[str, Any]:
    for line in reversed(output.splitlines()):
        line = line.strip()
        if line.startswith("{") and line.endswith("}"):
            return json.loads(line)
    raise RuntimeError(f"no JSON object found in command output:\n{output}")


def link_or_copy_models(source: Path, target: Path) -> str:
    if target.exists():
        return "existing"
    if not source.exists():
        raise RuntimeError(f"models source does not exist: {source}")
    try:
        os.symlink(source, target, target_is_directory=True)
        return "symlink"
    except OSError:
        pass
    if os.name == "nt":
        proc = run(["cmd", "/c", "mklink", "/J", str(target), str(source)], repo_root())
        if proc.returncode == 0:
            return "junction"
    target.mkdir(parents=True, exist_ok=True)
    for name in [
        "all-MiniLM-L12-v2.onnx",
        "all-MiniLM-L12-v2-tokenizer.json",
        "bge-base-en-v1.5.onnx",
        "bge-base-en-v1.5-tokenizer.json",
        "tokenizer.json",
    ]:
        src = source / name
        if src.exists():
            shutil.copy2(src, target / name)
    return "copy"


def cleanup_temp_root(temp_root: Path, home: Path) -> None:
    models = home / "models"
    try:
        is_link = models.is_symlink()
        is_junction = bool(getattr(models, "is_junction", lambda: False)())
        if models.exists() and (is_link or is_junction):
            models.rmdir()
    except OSError:
        pass
    shutil.rmtree(temp_root, ignore_errors=True)


def seed_backfill_rows(db_path: Path, count: int) -> None:
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).strftime("%Y-%m-%d %H:%M:%S")
    with sqlite3.connect(db_path) as conn:
        conn.execute("DELETE FROM embeddings")
        conn.execute("DELETE FROM memories")
        conn.execute("DELETE FROM decisions")
        for idx in range(count):
            text = (
                f"RQ1 BGE throughput memory {idx}. "
                "This deterministic passage exercises local embedding backfill with enough text "
                "to avoid empty or trivial tokenizer behavior."
            )
            conn.execute(
                """
                INSERT INTO memories
                    (text, source, type, status, score, created_at, updated_at, retention_class)
                VALUES (?1, 'rq1-local-gate', 'note', 'active', 0.8, ?2, ?2, 'operational')
                """,
                (text, now),
            )
            decision = (
                f"RQ1 BGE throughput decision {idx}. "
                "The default embedding profile should backfill this row and preserve recall readiness."
            )
            conn.execute(
                """
                INSERT INTO decisions
                    (decision, context, type, source_agent, status, score, created_at, updated_at, retention_class)
                VALUES (?1, ?2, 'decision', 'rq1-local-gate', 'active', 0.8, ?3, ?3, 'operational')
                """,
                (decision, f"rq1-local-gate::{idx}", now),
            )
        conn.commit()


def run_backfill(binary: Path, args: argparse.Namespace) -> dict[str, Any]:
    root = repo_root()
    temp_root = root / ".tmp" / f"rq1-backfill-{now_tag()}-{os.getpid()}"
    home = temp_root / "home"
    home.mkdir(parents=True, exist_ok=True)
    model_link = link_or_copy_models(Path(args.models_source), home / "models")
    env = {
        "CORTEX_EMBEDDING_MODEL": BGE_PROFILE,
        "CORTEX_GLOBAL_LOCK_HOME": str(home / "global-lock"),
        "CORTEX_SINGLE_DAEMON_TEST_BYPASS": "1",
    }
    try:
        init = run([str(binary), "embeddings", "status", "--home", str(home), "--json"], root, env=env)
        if init.returncode != 0:
            raise RuntimeError(f"embeddings status failed:\n{init.stdout}")
        seed_backfill_rows(home / "cortex.db", args.backfill_rows)
        started = time.perf_counter()
        drain = run(
            [
                str(binary),
                "embeddings",
                "drain",
                "--home",
                str(home),
                "--batch-size",
                str(args.backfill_batch_size),
                "--max-batches",
                str(args.backfill_max_batches),
                "--until-exhausted",
                "--json",
            ],
            root,
            env=env,
        )
        elapsed_seconds = time.perf_counter() - started
        if drain.returncode != 0:
            raise RuntimeError(f"embeddings drain failed:\n{drain.stdout}")
        payload = extract_json_line(drain.stdout)
        computed = int(payload.get("computed_total", 0))
        throughput = (computed / elapsed_seconds) * 3600.0 if elapsed_seconds > 0 else 0.0
        return {
            "profile": BGE_PROFILE,
            "model_link": model_link,
            "rows_seeded": args.backfill_rows * 2,
            "elapsed_seconds": round(elapsed_seconds, 3),
            "throughput_embeddings_per_hour": round(throughput, 2),
            "drain": payload,
        }
    finally:
        cleanup_temp_root(temp_root, home)


def run_recall_probe(profile: str, args: argparse.Namespace) -> dict[str, Any]:
    root = repo_root()
    output_dir = root / ".tmp" / f"rq1-recall-{profile}-{now_tag()}-{os.getpid()}"
    env = {
        "CORTEX_EMBEDDING_MODEL": profile,
        "CORTEX_SINGLE_DAEMON_TEST_BYPASS": "1",
    }
    cmd = [
        sys.executable,
        "benchmarking/scripts/rq2_rerank_gate.py",
        "--mode",
        "off",
        "--limit",
        str(args.recall_limit),
        "--output",
        str(output_dir),
        "--models-source",
        str(args.models_source),
        "--target-dir",
        str(args.rq2_target_dir),
        "--skip-build",
        "--skip-model-download",
        "--json",
    ]
    proc = run(cmd, root, env=env)
    try:
        if proc.returncode != 0:
            raise RuntimeError(f"recall probe failed for {profile}:\n{proc.stdout}")
        payload = json.loads((output_dir / "baseline-off.json").read_text(encoding="utf-8"))
        latency = payload["summary"]
        return {
            "profile": profile,
            "dataset": "rq2-local-deterministic-off-mode",
            "queries": latency["queries"],
            "top1": latency["top1"],
            "top3": latency["top3"],
            "p50_ms": latency["p50_ms"],
            "p95_ms": latency["p95_ms"],
            "max_ms": latency["max_ms"],
        }
    finally:
        shutil.rmtree(output_dir, ignore_errors=True)


def write_markdown(path: Path, summary: dict[str, Any]) -> None:
    checks = summary["checks"]
    legacy = summary["recall_latency"][LEGACY_PROFILE]
    bge = summary["recall_latency"][BGE_PROFILE]
    backfill = summary["backfill"]
    text = f"""# RQ1 Embedding Profile Gate

Generated: {summary['generated_at']}

Result: **{'PASS' if summary['passed'] else 'FAIL'}**

This local gate covers BGE backfill throughput and p50 recall latency versus the
legacy MiniLM profile. It does not replace scored Pure LongMemEval-S.

## Checks

- Backfill throughput: {backfill['throughput_embeddings_per_hour']} emb/hr (required >= {checks['min_backfill_embeddings_per_hour']})
- Backfill rows built: {backfill['drain']['computed_total']} / {backfill['rows_seeded']}
- Recall p50 delta: {checks['recall_p50_delta_ms']} ms (allowed <= {checks['max_recall_p50_regression_ms']} ms)
- LongMemEval-S: {checks['longmemeval_status']}

## Mode Summary

| Profile | p50 ms | p95 ms | top3 |
|---------|--------|--------|------|
| {LEGACY_PROFILE} | {legacy['p50_ms']} | {legacy['p95_ms']} | {legacy['top3']} |
| {BGE_PROFILE} | {bge['p50_ms']} | {bge['p95_ms']} | {bge['top3']} |
"""
    path.write_text(text, encoding="utf-8")


def parse_args() -> argparse.Namespace:
    root = repo_root()
    parser = argparse.ArgumentParser(description="Run the Cortex RQ1 local embedding profile gate.")
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument("--markdown-output", type=Path, default=None)
    parser.add_argument("--target-dir", type=Path, default=root / "daemon-rs" / "target-codex-rq1-gate")
    parser.add_argument("--rq2-target-dir", type=Path, default=root / "daemon-rs" / "target-codex-rq2-gate")
    parser.add_argument("--models-source", type=Path, default=Path.home() / ".cortex" / "models")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--backfill-rows", type=int, default=32, help="Rows per table; total rows are doubled.")
    parser.add_argument("--backfill-batch-size", type=int, default=32)
    parser.add_argument("--backfill-max-batches", type=int, default=10)
    parser.add_argument("--recall-limit", type=int, default=6)
    parser.add_argument("--min-backfill-embeddings-per-hour", type=float, default=500.0)
    parser.add_argument("--max-recall-p50-regression-ms", type=float, default=10.0)
    parser.add_argument("--json", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = repo_root()
    output = args.output or root / "benchmarking" / "results" / f"rq1-embedding-profile-{now_tag()}.json"
    markdown_output = args.markdown_output or output.with_suffix(".md")
    binary = ensure_binary(root, args.target_dir, args.skip_build)

    backfill = run_backfill(binary, args)
    legacy_latency = run_recall_probe(LEGACY_PROFILE, args)
    bge_latency = run_recall_probe(BGE_PROFILE, args)
    p50_delta = round(float(bge_latency["p50_ms"]) - float(legacy_latency["p50_ms"]), 3)
    checks = {
        "min_backfill_embeddings_per_hour": args.min_backfill_embeddings_per_hour,
        "backfill_throughput_pass": backfill["throughput_embeddings_per_hour"] >= args.min_backfill_embeddings_per_hour,
        "backfill_exhausted_pass": bool(backfill["drain"].get("exhausted")) and int(backfill["drain"].get("remaining", {}).get("total", -1)) == 0,
        "max_recall_p50_regression_ms": args.max_recall_p50_regression_ms,
        "recall_p50_delta_ms": p50_delta,
        "recall_p50_pass": p50_delta <= args.max_recall_p50_regression_ms,
        "longmemeval_status": "blocked_no_provider_key",
    }
    passed = all(
        [
            checks["backfill_throughput_pass"],
            checks["backfill_exhausted_pass"],
            checks["recall_p50_pass"],
        ]
    )
    summary = {
        "schema": "cortex.rq1_embedding_profile_gate.v1",
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "git_commit": git_value(root, "rev-parse", "HEAD"),
        "config": {
            "backfill_rows_per_table": args.backfill_rows,
            "recall_limit": args.recall_limit,
            "profiles": [LEGACY_PROFILE, BGE_PROFILE],
        },
        "backfill": backfill,
        "recall_latency": {
            LEGACY_PROFILE: legacy_latency,
            BGE_PROFILE: bge_latency,
        },
        "checks": checks,
        "passed": passed,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    write_markdown(markdown_output, summary)
    print(json.dumps(summary, indent=2, sort_keys=True) if args.json else f"RQ1 gate wrote {output}")
    return 0 if passed else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise
    except Exception as exc:  # noqa: BLE001 - command line tool
        print(f"ERROR: {exc}", file=sys.stderr)
        raise SystemExit(1)

#!/usr/bin/env python3
"""Run the local Cortex RQ2 reranker gate.

This is a local, deterministic gate: it proves model load, off/shadow/primary
telemetry behavior, latency, and a Cortex-owned regression guard. It does not
replace scored LongMemEval-S runs; the generated README calls that out.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


CASES: list[dict[str, Any]] = [
    {
        "id": "capital-france",
        "query": "Which city is the capital of France?",
        "expected": ["paris", "france"],
        "documents": [
            "The answer is Paris. Paris is the capital city of France and the seat of national government.",
            "This decoy repeats the words city, capital, and France, but it says the phrase is only an example from a spelling worksheet.",
            "Berlin is the capital of Germany, not France.",
        ],
    },
    {
        "id": "auth-token",
        "query": "Where is the Cortex API token stored?",
        "expected": ["cortex.token", ".cortex"],
        "documents": [
            "Cortex writes the API token to ~/.cortex/cortex.token and clients read that file for authenticated local requests.",
            "A decoy mentions API token stored procedures, but describes a cloud dashboard rather than the local Cortex token file.",
            "The daemon PID lives in cortex.pid, which is separate from the API token.",
        ],
    },
    {
        "id": "panic-log",
        "query": "Where do daemon panic breadcrumbs go?",
        "expected": ["panic.log", ".cortex"],
        "documents": [
            "Daemon panics write breadcrumbs to ~/.cortex/panic.log with the payload, location, and backtrace.",
            "Handler panics return JSON 500 responses; that is not the panic breadcrumb location.",
            "The README changelog is not where daemon panic breadcrumbs are stored.",
        ],
    },
    {
        "id": "pq8-footprint",
        "query": "What compact embedding format reduced Cortex vector footprint?",
        "expected": ["pq8", "int8"],
        "documents": [
            "PQ8 int8 quantization became the canonical embedding blob format and shrank BGE vectors from 3072 bytes to 774 bytes.",
            "A stale f32 blob is still readable during migration, but it is not the compact target format.",
            "FTS5 optimize reduces index bloat, not the embedding vector encoding itself.",
        ],
    },
    {
        "id": "shadow-mode",
        "query": "What rerank mode observes without changing recall order?",
        "expected": ["shadow", "without changing"],
        "documents": [
            "CORTEX_RERANK_MODE=shadow runs the cross-encoder and reports rerankRoute telemetry without changing user-visible recall order.",
            "CORTEX_RERANK_MODE=primary may reorder the configured top-N recall window.",
            "CORTEX_RERANK_MODE=off skips rerank entirely and reports mode_off telemetry.",
        ],
    },
    {
        "id": "control-center-supervisor",
        "query": "What keeps the app-managed daemon alive after unexpected exits?",
        "expected": ["supervisor", "control center"],
        "documents": [
            "The Control Center supervisor thread restarts the app-managed daemon after unexpected exits while honoring intentional user stops.",
            "The MCP heartbeat tolerance only gives the plugin more recovery time; it is not the component that restarts the daemon.",
            "The setup command configures tools but does not supervise a running daemon.",
        ],
    },
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


def http_json(
    method: str,
    url: str,
    *,
    token: str | None = None,
    payload: dict[str, Any] | None = None,
    timeout: float = 15.0,
) -> dict[str, Any]:
    data = None
    headers = {"X-Cortex-Request": "true", "X-Source-Agent": "rq2-rerank-gate"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def wait_for_health(base_url: str, proc: subprocess.Popen[str]) -> dict[str, Any]:
    deadline = time.time() + 30
    last_error = ""
    while time.time() < deadline:
        if proc.poll() is not None:
            stderr = proc.stderr.read() if proc.stderr else ""
            raise RuntimeError(f"daemon exited before health: code={proc.returncode}; {stderr}")
        try:
            return http_json("GET", f"{base_url}/health", timeout=2)
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
    raise RuntimeError(f"token file was not written: {token_path}")


def ensure_binary(root: Path, target_dir: Path, skip_build: bool) -> Path:
    exe = "cortex.exe" if os.name == "nt" else "cortex"
    binary = target_dir / "debug" / exe
    if binary.exists():
        return binary
    if skip_build:
        raise RuntimeError(f"--skip-build set but expected binary is missing: {binary}")
    env = {"CARGO_TARGET_DIR": str(target_dir)}
    proc = run(["rtk", "cargo", "build", "--manifest-path", "daemon-rs/Cargo.toml"], root, env=env)
    if proc.returncode != 0:
        proc = run(["cargo", "build", "--manifest-path", "daemon-rs/Cargo.toml"], root, env=env)
    if proc.returncode != 0:
        raise RuntimeError(f"cargo build failed:\n{proc.stdout}")
    if not binary.exists():
        raise RuntimeError(f"expected binary missing after build: {binary}")
    return binary


def reranker_model_path(models_source: Path) -> Path:
    return models_source / "rerank" / "ms-marco-MiniLM-L-6-v2" / "model_int8.onnx"


def ensure_models(root: Path, binary: Path, args: argparse.Namespace) -> None:
    required = reranker_model_path(Path(args.models_source))
    if required.exists():
        return
    if args.skip_model_download:
        raise RuntimeError(f"--skip-model-download set but model is missing: {required}")

    proc = run([str(binary), "setup"], root, env={"CARGO_TARGET_DIR": str(args.target_dir)})
    if proc.returncode != 0:
        raise RuntimeError(f"model setup failed:\n{proc.stdout}")
    if not required.exists():
        raise RuntimeError(
            "model setup completed but the reranker asset is still missing: "
            f"{required}. Pass --models-source to the populated Cortex models directory."
        )


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
    copied: list[str] = []
    for name in ["bge-base-en-v1.5.onnx", "bge-base-en-v1.5-tokenizer.json"]:
        src = source / name
        if src.exists():
            shutil.copy2(src, target / name)
            copied.append(name)
    rerank_src = source / "rerank"
    if rerank_src.exists():
        shutil.copytree(rerank_src, target / "rerank", dirs_exist_ok=True)
        copied.append("rerank/")
    return "copy:" + ",".join(copied)


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


def start_daemon(binary: Path, home: Path, mode: str, port: int, top_n: int, alpha: float) -> subprocess.Popen[str]:
    env = os.environ.copy()
    env.update(
        {
            "CORTEX_RERANK_MODE": mode,
            "CORTEX_RERANK_TOP_N": str(top_n),
            "CORTEX_RERANK_FUSION_ALPHA": str(alpha),
            "CORTEX_GLOBAL_LOCK_HOME": str(home / "global-lock"),
            "CORTEX_SINGLE_DAEMON_TEST_BYPASS": "1",
        }
    )
    if mode == "off":
        env["CORTEX_RERANK_ENABLED"] = "0"
    return subprocess.Popen(
        [str(binary), "serve", "--home", str(home), "--port", str(port)],
        cwd=str(repo_root()),
        env=env,
        text=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )


def seed_corpus(base_url: str, token: str) -> None:
    for case in CASES:
        for idx, document in enumerate(case["documents"]):
            http_json(
                "POST",
                f"{base_url}/store",
                token=token,
                payload={
                    "decision": f"RQ2 benchmark {case['id']} document {idx + 1}. {document}",
                    "context": f"rq2-rerank::{case['id']}::{idx + 1}",
                    "retention_class": "ephemeral",
                },
            )


def matches_expected(case: dict[str, Any], item: dict[str, Any]) -> bool:
    haystack = f"{item.get('source', '')} {item.get('excerpt', '')}".lower()
    return all(term in haystack for term in case["expected"])


def run_mode(binary: Path, output_dir: Path, args: argparse.Namespace, mode: str) -> dict[str, Any]:
    temp_root = (repo_root() / "benchmarking" / "runs" / f"tmp-rq2-{mode}-{int(time.time() * 1000)}").resolve()
    home = temp_root / "home"
    home.mkdir(parents=True, exist_ok=True)
    model_link = link_or_copy_models(Path(args.models_source), home / "models")
    port = reserve_port()
    base_url = f"http://127.0.0.1:{port}"
    proc = start_daemon(binary, home, mode, port, args.top_n, args.fusion_alpha)
    try:
        health = wait_for_health(base_url, proc)
        token = wait_for_token(home, proc)
        seed_corpus(base_url, token)
        rows: list[dict[str, Any]] = []
        for case in CASES[: args.limit]:
            query = urllib.parse.urlencode({"q": case["query"], "budget": str(args.recall_budget), "k": str(args.k)})
            started = time.perf_counter()
            payload = http_json("GET", f"{base_url}/recall?{query}", token=token, timeout=30)
            elapsed_ms = round((time.perf_counter() - started) * 1000, 3)
            results = payload.get("results") or []
            top1 = bool(results and matches_expected(case, results[0]))
            top3 = any(matches_expected(case, item) for item in results[:3])
            rows.append(
                {
                    "id": case["id"],
                    "query": case["query"],
                    "latency_ms": elapsed_ms,
                    "top1_hit": top1,
                    "top3_hit": top3,
                    "result_sources": [item.get("source") for item in results],
                    "result_methods": [item.get("method") for item in results],
                    "rerankRoute": payload.get("rerankRoute"),
                    "results": [
                        {
                            "source": item.get("source"),
                            "method": item.get("method"),
                            "relevance": item.get("relevance"),
                            "excerpt": item.get("excerpt"),
                        }
                        for item in results
                    ],
                }
            )
        mode_payload = {
            "mode": mode,
            "health_reranker": (((health.get("vector_search") or {}).get("reranker")) or {}),
            "model_link": model_link,
            "queries": rows,
            "summary": summarize_mode(rows),
        }
        (output_dir / f"{'baseline-off' if mode == 'off' else mode}.json").write_text(
            json.dumps(mode_payload, indent=2),
            encoding="utf-8",
        )
        return mode_payload
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
        cleanup_temp_root(temp_root, home)


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    sorted_values = sorted(values)
    index = int(round((len(sorted_values) - 1) * pct))
    return round(sorted_values[index], 3)


def summarize_mode(rows: list[dict[str, Any]]) -> dict[str, Any]:
    latencies = [float(row["latency_ms"]) for row in rows]
    count = len(rows)
    return {
        "queries": count,
        "top1": round(sum(1 for row in rows if row["top1_hit"]) / count, 4) if count else 0.0,
        "top3": round(sum(1 for row in rows if row["top3_hit"]) / count, 4) if count else 0.0,
        "p50_ms": percentile(latencies, 0.50),
        "p90_ms": percentile(latencies, 0.90),
        "p95_ms": percentile(latencies, 0.95),
        "max_ms": round(max(latencies), 3) if latencies else 0.0,
    }


def compare_results(results: dict[str, dict[str, Any]]) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    baseline = results.get("off", {"summary": {}, "queries": []})
    shadow = results.get("shadow", {"summary": {}, "queries": []})
    primary = results.get("primary", {"summary": {}, "queries": []})
    latency = {
        mode: payload.get("summary", {})
        for mode, payload in results.items()
    }
    quality = {
        mode: {
            "top1": payload.get("summary", {}).get("top1", 0.0),
            "top3": payload.get("summary", {}).get("top3", 0.0),
        }
        for mode, payload in results.items()
    }
    off_top1 = float(baseline.get("summary", {}).get("top1", 0.0))
    primary_top1 = float(primary.get("summary", {}).get("top1", 0.0))
    off_p95 = float(baseline.get("summary", {}).get("p95_ms", 0.0))
    primary_p95 = float(primary.get("summary", {}).get("p95_ms", 0.0))
    shadow_order_matches = compare_order(baseline.get("queries", []), shadow.get("queries", []))
    regression = {
        "owned_dataset": "rq2-local-deterministic",
        "queries": len(baseline.get("queries", [])),
        "top1_delta_primary_vs_off": round(primary_top1 - off_top1, 4),
        "primary_regressed_by_at_least_1pp": (primary_top1 - off_top1) < -0.01,
        "p95_delta_ms_primary_vs_off": round(primary_p95 - off_p95, 3),
        "p95_within_plus_80ms": (primary_p95 - off_p95) <= 80.0,
        "shadow_order_matches_off": shadow_order_matches,
        "longmemeval_status": "not_run_by_this_local_gate",
        "release_posture": "CAUTION",
        "release_posture_reason": (
            "Local model smoke and owned regression artifacts can support shadow/experimental posture, "
            "but scored LongMemEval-S is still required before a public primary rerank claim."
        ),
    }
    return latency, quality, regression


def compare_order(left: list[dict[str, Any]], right: list[dict[str, Any]]) -> bool:
    if len(left) != len(right):
        return False
    for left_row, right_row in zip(left, right):
        if left_row.get("id") != right_row.get("id"):
            return False
        if left_row.get("result_sources") != right_row.get("result_sources"):
            return False
    return True


def write_manifest(output_dir: Path, args: argparse.Namespace, binary: Path, modes: list[str]) -> None:
    model_path = reranker_model_path(Path(args.models_source))
    manifest = {
        "git_commit": git_value(repo_root(), "rev-parse", "HEAD"),
        "git_status_short": git_value(repo_root(), "status", "--short"),
        "os": platform.platform(),
        "machine": platform.machine(),
        "python": sys.version.split()[0],
        "binary": str(binary),
        "model_path": str(model_path),
        "model_size_bytes": model_path.stat().st_size if model_path.exists() else None,
        "env": {
            "CORTEX_RERANK_TOP_N": str(args.top_n),
            "CORTEX_RERANK_FUSION_ALPHA": str(args.fusion_alpha),
            "CARGO_TARGET_DIR": str(args.target_dir),
        },
        "dataset": "rq2-local-deterministic",
        "dataset_count": min(args.limit, len(CASES)),
        "modes": modes,
        "benchmark_command": " ".join(sys.argv),
        "daemon_startup_mode": "isolated serve --home <temp> --port <reserved>",
        "api_credits_or_cloud_judge_available": any(
            os.environ.get(name) for name in ["GEMINI_API_KEY", "GOOGLE_API_KEY", "OPENAI_API_KEY", "GROQ_API_KEY"]
        ),
    }
    (output_dir / "run-manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")


def write_readme(output_dir: Path, regression: dict[str, Any]) -> None:
    text = f"""# RQ2 Rerank Gate

Generated: {dt.datetime.now().isoformat(timespec='seconds')}

This is the local deterministic RQ2 gate. It proves model-load/runtime behavior,
off/shadow/primary telemetry, latency, and a Cortex-owned regression guard.

Release posture: **{regression['release_posture']}**

Reason: {regression['release_posture_reason']}

Important limitation: this run does **not** replace scored Pure LongMemEval-S.
Do not make a public primary rerank quality claim until LongMemEval-S is run
and passes the Phase 2 gate.
"""
    (output_dir / "README.md").write_text(text, encoding="utf-8")


def parse_args() -> argparse.Namespace:
    root = repo_root()
    parser = argparse.ArgumentParser(description="Run the Cortex RQ2 local rerank gate.")
    parser.add_argument("--mode", choices=["off", "shadow", "primary", "all"], default="all")
    parser.add_argument("--limit", type=int, default=len(CASES))
    parser.add_argument("--output", default=None)
    parser.add_argument("--skip-model-download", action="store_true", help="Validate existing assets only.")
    parser.add_argument("--skip-build", action="store_true", help="Use an existing cortex binary.")
    parser.add_argument("--json", action="store_true", help="Print machine-readable summary.")
    parser.add_argument("--dry-run", action="store_true", help="Write manifest intent but do not spawn daemons.")
    parser.add_argument("--top-n", type=int, default=24)
    parser.add_argument("--fusion-alpha", type=float, default=0.65)
    parser.add_argument("--recall-budget", type=int, default=300)
    parser.add_argument("--k", type=int, default=5)
    parser.add_argument("--models-source", default=str(Path.home() / ".cortex" / "models"))
    parser.add_argument("--target-dir", default=str(root / "daemon-rs" / "target-codex-rq2-gate"))
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = repo_root()
    output_dir = Path(args.output) if args.output else root / "benchmarking" / "results" / f"rq2-rerank-{now_tag()}"
    output_dir.mkdir(parents=True, exist_ok=True)

    modes = ["off", "shadow", "primary"] if args.mode == "all" else [args.mode]
    binary = ensure_binary(root, Path(args.target_dir), args.skip_build)
    if not args.dry_run:
        ensure_models(root, binary, args)
    write_manifest(output_dir, args, binary, modes)

    if args.dry_run:
        summary = {"output": str(output_dir), "modes": modes, "dry_run": True}
        (output_dir / "dry-run.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
        print(json.dumps(summary, indent=2) if args.json else f"dry run wrote {output_dir}")
        return 0

    results: dict[str, dict[str, Any]] = {}
    for mode in modes:
        results[mode] = run_mode(binary, output_dir, args, mode)

    latency, quality, regression = compare_results(results)
    (output_dir / "latency-summary.json").write_text(json.dumps(latency, indent=2), encoding="utf-8")
    (output_dir / "quality-summary.json").write_text(json.dumps(quality, indent=2), encoding="utf-8")
    (output_dir / "retriever-regression.json").write_text(json.dumps(regression, indent=2), encoding="utf-8")
    write_readme(output_dir, regression)

    summary = {
        "output": str(output_dir),
        "quality": quality,
        "latency": latency,
        "regression": regression,
    }
    print(json.dumps(summary, indent=2) if args.json else f"RQ2 gate wrote {output_dir}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise
    except Exception as exc:  # noqa: BLE001 - command line tool
        print(f"ERROR: {exc}", file=sys.stderr)
        raise SystemExit(1)

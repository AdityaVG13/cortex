#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from contextlib import contextmanager
from dataclasses import asdict
from datetime import datetime, timezone
from pathlib import Path

import httpx

REPO_ROOT = Path(__file__).resolve().parents[1]
AMB_SRC = REPO_ROOT / "benchmarking" / "tools" / "agent-memory-benchmark" / "src"
ADAPTERS = REPO_ROOT / "benchmarking" / "adapters"
RUNS = REPO_ROOT / "benchmarking" / "runs"


def _configure_paths() -> None:
    for path in (AMB_SRC, ADAPTERS):
        entry = str(path)
        if entry not in sys.path:
            sys.path.insert(0, entry)


def _find_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _find_cortex_bin() -> Path:
    for candidate in (
        os.environ.get("CORTEX_BIN"),
        REPO_ROOT / "target" / "debug" / "cortex",
        REPO_ROOT / "target" / "release" / "cortex",
        Path.home() / ".cortex" / "bin" / "cortex",
    ):
        if candidate and Path(candidate).exists():
            return Path(candidate)
    raise FileNotFoundError("Cortex binary not found. Set CORTEX_BIN or build cortex first.")


def _seed_models(source: Path, target: Path) -> None:
    if not source.exists():
        return
    target.mkdir(parents=True, exist_ok=True)
    for file in source.iterdir():
        if file.is_file() and file.suffix.lower() in {".onnx", ".json"}:
            destination = target / file.name
            if not destination.exists():
                shutil.copy2(file, destination)


def _git_head(root: Path) -> str:
    try:
        return subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", "--short", "HEAD"],
            text=True,
        ).strip()
    except Exception:
        return "unknown"


@contextmanager
def isolated_daemon(run_dir: Path):
    home = run_dir / "daemon-home"
    home.mkdir(parents=True, exist_ok=True)
    port = _find_port()
    base_url = f"http://127.0.0.1:{port}"
    binary = _find_cortex_bin()
    _seed_models(Path.home() / ".cortex" / "models", home / "models")
    stdout = (run_dir / "daemon.stdout.log").open("w", encoding="utf-8")
    stderr = (run_dir / "daemon.stderr.log").open("w", encoding="utf-8")
    proc = subprocess.Popen(
        [str(binary), "serve", "--home", str(home), "--port", str(port), "--bind", "127.0.0.1"],
        stdout=stdout,
        stderr=stderr,
        text=True,
    )
    token_file = home / "cortex.token"
    deadline = time.time() + 30
    while time.time() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(f"benchmark daemon exited early with code {proc.returncode}")
        try:
            healthy = httpx.get(f"{base_url}/health", timeout=2.0).is_success
            token = token_file.read_text(encoding="utf-8").strip() if token_file.exists() else ""
            if healthy and token:
                break
        except httpx.HTTPError:
            pass
        time.sleep(0.1)
    else:
        proc.kill()
        raise TimeoutError("benchmark daemon did not become healthy within 30 seconds")

    namespace = f"longmemeval-{run_dir.name}"
    env = {
        "CORTEX_BASE_URL": base_url,
        "CORTEX_AUTH_TOKEN": token_file.read_text(encoding="utf-8").strip(),
        "CORTEX_TOKEN_FILE": str(token_file),
        "CORTEX_SOURCE_AGENT": f"amb-cortex::{namespace}",
        "CORTEX_RECALL_BUDGET": os.environ.get("CORTEX_RECALL_BUDGET", "200"),
    }
    try:
        yield env
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        stdout.close()
        stderr.close()


def run(args: argparse.Namespace) -> Path:
    _configure_paths()
    from cortex_http_pure_provider import CortexHTTPPureMemoryProvider
    from memory_bench.dataset import get_dataset
    from memory_bench.llm import get_answer_llm
    from memory_bench.memory import REGISTRY
    from memory_bench.modes import get_mode
    from memory_bench.runner import EvalRunner

    REGISTRY["cortex-http-pure"] = CortexHTTPPureMemoryProvider

    run_dir = RUNS / datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    run_dir.mkdir(parents=True, exist_ok=True)

    with isolated_daemon(run_dir) as daemon_env:
        os.environ.update(daemon_env)
        summary = EvalRunner(output_dir=run_dir / "outputs").run(
            dataset=get_dataset("longmemeval"),
            split=args.split,
            memory=CortexHTTPPureMemoryProvider(),
            mode=get_mode(args.mode, llm=get_answer_llm()),
            query_limit=args.query_limit,
            category=args.category,
            oracle=args.oracle,
            run_name="cortex-http-pure",
        )
        (run_dir / "summary.json").write_text(json.dumps(asdict(summary), indent=2), encoding="utf-8")
        manifest = {
            "created_at": datetime.now(timezone.utc).isoformat(),
            "cortex_repo_head": _git_head(REPO_ROOT),
            "dataset": "longmemeval",
            "split": args.split,
            "query_limit": args.query_limit,
            "category": args.category,
            "mode": args.mode,
            "memory_backend": "cortex-http-pure",
            "accuracy": summary.accuracy,
            "correct": summary.correct,
            "total_queries": summary.total_queries,
            "oracle": args.oracle,
        }
        (run_dir / "run-manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    print(
        json.dumps(
            {
                "run_dir": str(run_dir),
                "accuracy": summary.accuracy,
                "correct": summary.correct,
                "total_queries": summary.total_queries,
            },
            indent=2,
        )
    )
    return run_dir


def main() -> None:
    parser = argparse.ArgumentParser(description="Run LongMemEval-S against an isolated Cortex daemon.")
    parser.add_argument("--split", default="s", help="LongMemEval split (default: s)")
    parser.add_argument("--query-limit", type=int, default=20, help="Max queries to score")
    parser.add_argument("--category", default=None, help="Optional question_type filter")
    parser.add_argument("--mode", default="rag", help="AMB response mode (default: rag)")
    parser.add_argument("--oracle", action="store_true", help="Diagnostic ceiling: ingest gold docs only")
    run(parser.parse_args())


if __name__ == "__main__":
    main()

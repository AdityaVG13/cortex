from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import time
from contextlib import AbstractContextManager
from dataclasses import asdict
from datetime import datetime
from pathlib import Path

import httpx


REPO_ROOT = Path(__file__).resolve().parents[1]
AMB_SRC = REPO_ROOT / "benchmarking" / "tools" / "agent-memory-benchmark" / "src"
ADAPTERS_DIR = REPO_ROOT / "benchmarking" / "adapters"
RUNS_ROOT = REPO_ROOT / "benchmarking" / "runs"


def _configure_imports() -> None:
    for path in (str(AMB_SRC), str(ADAPTERS_DIR)):
        if path not in sys.path:
            sys.path.insert(0, path)


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _resolve_cortex_binary() -> Path:
    candidates = [
        os.environ.get("CORTEX_BIN"),
        str(Path.home() / ".cortex" / "bin" / ("cortex.exe" if os.name == "nt" else "cortex")),
        str(REPO_ROOT / "daemon-rs" / "target" / "debug" / ("cortex.exe" if os.name == "nt" else "cortex")),
        str(REPO_ROOT / "daemon-rs" / "target" / "release" / ("cortex.exe" if os.name == "nt" else "cortex")),
    ]
    for candidate in candidates:
        if candidate and Path(candidate).exists():
            return Path(candidate)
    raise FileNotFoundError(
        "Unable to locate a Cortex binary. Set CORTEX_BIN or build/install cortex first."
    )


def _git_head_short(repo_root: Path) -> str | None:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
        )
        return result.stdout.strip() or None
    except Exception:
        return None


def _load_lock_summary() -> dict[str, str]:
    lock_path = REPO_ROOT / "benchmarking" / "benchmarks.lock.json"
    if not lock_path.exists():
        return {}
    payload = json.loads(lock_path.read_text(encoding="utf-8"))
    return {tool["name"]: tool["commit"] for tool in payload.get("tools", [])}


def _write_run_manifest(run_dir: Path, payload: dict) -> None:
    (run_dir / "run-manifest.json").write_text(
        json.dumps(payload, indent=2),
        encoding="utf-8",
    )


class IsolatedCortexDaemon(AbstractContextManager["IsolatedCortexDaemon"]):
    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir
        self.home = run_dir / "daemon-home"
        self.home.mkdir(parents=True, exist_ok=True)
        self.port = _find_free_port()
        self.base_url = f"http://127.0.0.1:{self.port}"
        self.binary = _resolve_cortex_binary()
        self.proc: subprocess.Popen[str] | None = None
        self.token = ""
        self.token_file = self.home / "cortex.token"
        self.stdout_path = run_dir / "daemon.stdout.log"
        self.stderr_path = run_dir / "daemon.stderr.log"

    def __enter__(self) -> "IsolatedCortexDaemon":
        stdout = self.stdout_path.open("w", encoding="utf-8")
        stderr = self.stderr_path.open("w", encoding="utf-8")
        self.proc = subprocess.Popen(
            [str(self.binary), "serve", "--home", str(self.home), "--port", str(self.port)],
            stdout=stdout,
            stderr=stderr,
            text=True,
        )
        self._wait_for_health()
        self.token = self._wait_for_token()
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        if self.proc is not None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=10)

    def export_env(self, namespace: str) -> dict[str, str]:
        return {
            "CORTEX_BASE_URL": self.base_url,
            "CORTEX_AUTH_TOKEN": self.token,
            "CORTEX_TOKEN_FILE": str(self.token_file),
            "CORTEX_SOURCE_AGENT": f"amb-cortex::{namespace}",
            "CORTEX_BENCHMARK_NAMESPACE": namespace,
        }

    def _wait_for_health(self) -> None:
        client = httpx.Client(timeout=2.0)
        deadline = time.time() + 20
        while time.time() < deadline:
            if self.proc is not None and self.proc.poll() is not None:
                raise RuntimeError(f"Benchmark daemon exited early with code {self.proc.returncode}")
            try:
                response = client.get(f"{self.base_url}/health")
                if response.is_success:
                    return
            except httpx.HTTPError:
                pass
            time.sleep(0.1)
        raise TimeoutError("Benchmark daemon did not become healthy within 20 seconds.")

    def _wait_for_token(self) -> str:
        deadline = time.time() + 20
        while time.time() < deadline:
            if self.proc is not None and self.proc.poll() is not None:
                raise RuntimeError(f"Benchmark daemon exited early with code {self.proc.returncode}")
            if self.token_file.exists():
                token = self.token_file.read_text(encoding="utf-8").strip()
                if token:
                    return token
            time.sleep(0.1)
        raise TimeoutError("Benchmark daemon did not write cortex.token within 20 seconds.")


def _register_provider() -> None:
    _configure_imports()
    from cortex_amb_provider import CortexHTTPMemoryProvider
    from memory_bench.memory import REGISTRY

    REGISTRY["cortex-http"] = CortexHTTPMemoryProvider


def _assert_amb_environment() -> None:
    _configure_imports()
    try:
        import memory_bench.memory  # noqa: F401
    except ModuleNotFoundError as exc:
        raise RuntimeError(
            "AMB dependencies are not installed. From "
            "`benchmarking/tools/agent-memory-benchmark`, run `uv sync` or "
            "`uv pip install -e .` before using the AMB-backed `run` command."
        ) from exc


def run_smoke(run_dir: Path) -> None:
    _configure_imports()
    from cortex_http_client import CortexHTTPClient, CortexStoredDocument

    namespace = f"smoke-{run_dir.name}"
    with IsolatedCortexDaemon(run_dir) as daemon:
        os.environ.update(daemon.export_env(namespace))
        _write_run_manifest(
            run_dir,
            {
                "command": "smoke",
                "created_at": datetime.now().isoformat(),
                "cortex_repo_head": _git_head_short(REPO_ROOT),
                "cortex_binary": str(daemon.binary),
                "daemon_mode": "isolated-benchmark",
                "benchmark_tools": _load_lock_summary(),
                "namespace": namespace,
                "legitimacy": {
                    "isolated_daemon": True,
                    "uses_live_app_daemon": False,
                    "oracle_mode": False,
                    "notes": "Smoke test validates Cortex ingest/retrieve only. It does not run AMB judging.",
                },
            },
        )
        client = CortexHTTPClient()
        client.healthcheck()
        client.reset_namespace(namespace)
        client.store_documents(
            [
                CortexStoredDocument(
                    id="d1",
                    content="Cortex uses a Rust daemon with SQLite and ONNX embeddings.",
                    user_id="bench-user",
                ),
                CortexStoredDocument(
                    id="d2",
                    content="LongMemEval evaluates information extraction, reasoning, updates, temporal recall, and abstention.",
                    user_id="bench-user",
                ),
            ]
        )
        docs, raw = client.recall_documents(
            "What does LongMemEval evaluate?",
            k=2,
            user_id="bench-user",
        )
        payload = {
            "retrieved_ids": [doc.id for doc in docs],
            "contexts": [doc.content for doc in docs],
            "raw_result_count": len((raw or {}).get("results") or []),
            "base_url": daemon.base_url,
            "run_dir": str(run_dir),
        }
        client.close()
        print(json.dumps(payload, indent=2))


def run_benchmark(args: argparse.Namespace, run_dir: Path) -> None:
    _assert_amb_environment()
    _register_provider()
    from memory_bench.dataset import get_dataset
    from memory_bench.modes import get_mode
    from memory_bench.runner import EvalRunner
    from memory_bench.memory import get_memory_provider

    namespace = args.run_name or f"{args.dataset}-{args.split}-{run_dir.name}"
    with IsolatedCortexDaemon(run_dir) as daemon:
        os.environ.update(daemon.export_env(namespace))
        _write_run_manifest(
            run_dir,
            {
                "command": "run",
                "created_at": datetime.now().isoformat(),
                "cortex_repo_head": _git_head_short(REPO_ROOT),
                "cortex_binary": str(daemon.binary),
                "daemon_mode": "isolated-benchmark",
                "benchmark_tools": _load_lock_summary(),
                "dataset": args.dataset,
                "split": args.split,
                "mode": args.mode,
                "category": args.category,
                "query_limit": args.query_limit,
                "query_id": args.query_id,
                "doc_limit": args.doc_limit,
                "namespace": namespace,
                "legitimacy": {
                    "isolated_daemon": True,
                    "uses_live_app_daemon": False,
                    "oracle_mode": bool(args.oracle),
                    "notes": (
                        "Normal benchmark runs should keep oracle_mode=false. "
                        "If oracle_mode=true, treat the run as a diagnostic ceiling, not a headline score."
                    ),
                },
            },
        )

        dataset = get_dataset(args.dataset)
        mode = get_mode(args.mode)
        memory = get_memory_provider("cortex-http")
        runner = EvalRunner(output_dir=run_dir / "outputs")

        summary = runner.run(
            dataset=dataset,
            split=args.split,
            memory=memory,
            mode=mode,
            category=args.category,
            query_limit=args.query_limit,
            query_id=args.query_id,
            doc_limit=args.doc_limit,
            oracle=args.oracle,
            skip_ingestion=False,
            skip_ingested=False,
            skip_retrieval=False,
            skip_answer=False,
            only_failed=False,
            show_raw=False,
            run_name=args.run_name or "cortex-http",
            description=args.description,
        )

        summary_path = run_dir / "summary.json"
        summary_path.write_text(json.dumps(asdict(summary), indent=2), encoding="utf-8")
        print(
            json.dumps(
                {
                    "dataset": summary.dataset,
                    "split": summary.split,
                    "memory_provider": summary.memory_provider,
                    "mode": summary.mode,
                    "accuracy": summary.accuracy,
                    "total_queries": summary.total_queries,
                    "run_dir": str(run_dir),
                    "output_json": str((run_dir / "outputs" / summary.dataset / summary.run_name / summary.mode / f"{summary.split}.json")),
                },
                indent=2,
            )
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run AMB against an isolated Cortex benchmark daemon.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    smoke = subparsers.add_parser("smoke", help="Run a retrieval-only smoke test against an isolated Cortex daemon.")
    smoke.set_defaults(func=run_smoke)

    run = subparsers.add_parser("run", help="Run an AMB benchmark against an isolated Cortex daemon.")
    run.add_argument("--dataset", required=True, help="AMB dataset name, e.g. longmemeval, locomo, membench.")
    run.add_argument("--split", required=True, help="AMB split/domain name for the dataset.")
    run.add_argument("--mode", default="rag", help="AMB response mode. Defaults to rag.")
    run.add_argument("--category", default=None, help="Optional AMB category filter.")
    run.add_argument("--query-limit", type=int, default=None, help="Optional query limit for smaller runs.")
    run.add_argument("--query-id", default=None, help="Optional single query id.")
    run.add_argument("--doc-limit", type=int, default=None, help="Optional document limit.")
    run.add_argument("--oracle", action="store_true", help="Use oracle mode when the dataset supports it.")
    run.add_argument("--run-name", default=None, help="Optional AMB run name. Defaults to cortex-http.")
    run.add_argument("--description", default=None, help="Optional run description written into the AMB output.")
    run.set_defaults(func=run_benchmark)

    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    run_dir = RUNS_ROOT / f"amb-{args.command}-{timestamp}"
    run_dir.mkdir(parents=True, exist_ok=True)
    try:
        args.func(args, run_dir) if args.command == "run" else args.func(run_dir)
    except Exception as exc:
        print(f"benchmark runner failed: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc


if __name__ == "__main__":
    main()

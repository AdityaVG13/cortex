"""Isolated Cortex daemon lifecycle for benchmark runs."""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from contextlib import AbstractContextManager
from pathlib import Path
from typing import TextIO

import httpx

from run_amb_config import DEFAULT_MEMORY_BACKEND, REPO_ROOT, SUPPORTED_MEMORY_BACKENDS
from run_amb_runtime import (
    _configure_imports,
    _env_flag_enabled,
    _find_free_port,
    _resolve_cortex_binary,
    _seed_model_assets,
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
        self._stdout: TextIO | None = None
        self._stderr: TextIO | None = None
        self.attached_existing = False

    @property
    def daemon_mode(self) -> str:
        return "app-owned-attached" if self.attached_existing else "isolated-benchmark"

    def _lock_conflict_detected(self) -> bool:
        if not self.stderr_path.exists():
            return False
        try:
            text = self.stderr_path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            return False
        return "another cortex instance holds the lock" in text.lower()

    def _try_attach_existing_daemon(self) -> bool:
        try:
            paths_result = subprocess.run(
                [str(self.binary), "paths", "--json"],
                check=True,
                capture_output=True,
                text=True,
            )
            paths_payload = json.loads(paths_result.stdout)
            token_path = Path(str(paths_payload.get("token", "")))
            port = int(paths_payload.get("port", 7437))
            base_url = os.environ.get("CORTEX_BENCHMARK_BASE_URL", f"http://127.0.0.1:{port}")
            if not token_path.exists():
                return False
            token = token_path.read_text(encoding="utf-8").strip()
            if not token:
                return False
            health_ok = False
            for attempt in range(6):
                try:
                    health = httpx.get(f"{base_url}/health", timeout=5.0)
                    if health.is_success:
                        health_ok = True
                        break
                except httpx.HTTPError:
                    pass
                time.sleep(min(1.5, 0.2 * (attempt + 1)))
            if not health_ok:
                return False
            self.base_url = base_url
            self.token_file = token_path
            self.token = token
            self.attached_existing = True
            return True
        except Exception:
            return False

    def __enter__(self) -> "IsolatedCortexDaemon":
        attach_existing = _env_flag_enabled("CORTEX_BENCHMARK_ATTACH_EXISTING_DAEMON", default=False)
        require_app_daemon = _env_flag_enabled("CORTEX_BENCHMARK_REQUIRE_APP_DAEMON", default=False)
        if require_app_daemon:
            attach_existing = True
        if attach_existing and self._try_attach_existing_daemon():
            return self
        if require_app_daemon:
            raise RuntimeError(
                "App-owned Cortex daemon is required for benchmark runs but no live daemon was reachable. "
                "Open Cortex Control Center first (or set CORTEX_BENCHMARK_REQUIRE_APP_DAEMON=0 for isolated diagnostics)."
            )
        _seed_model_assets(Path.home() / ".cortex" / "models", self.home / "models")
        proc_env = os.environ.copy()
        proc_env.setdefault(
            "CORTEX_RATE_LIMIT_REQUESTS_PER_MIN",
            os.environ.get("CORTEX_BENCHMARK_REQUESTS_PER_MIN", "100000"),
        )
        proc_env.setdefault(
            "CORTEX_RATE_LIMIT_AUTH_FAILS_PER_MIN",
            os.environ.get("CORTEX_BENCHMARK_AUTH_FAILS_PER_MIN", "10000"),
        )
        try:
            self._stdout = self.stdout_path.open("w", encoding="utf-8")
            self._stderr = self.stderr_path.open("w", encoding="utf-8")
            self.proc = subprocess.Popen(
                [
                    str(self.binary),
                    "serve",
                    "--home",
                    str(self.home),
                    "--port",
                    str(self.port),
                    "--bind",
                    "127.0.0.1",
                ],
                stdout=self._stdout,
                stderr=self._stderr,
                text=True,
                env=proc_env,
            )
            self._wait_for_health()
            self.token = self._wait_for_token()
        except Exception as exc:
            if (
                isinstance(exc, RuntimeError)
                and attach_existing
                and self._lock_conflict_detected()
            ):
                self._stop_isolated_process()
                if self._try_attach_existing_daemon():
                    return self
            self._stop_isolated_process()
            raise
        return self

    def _close_log_streams(self) -> None:
        for stream in (self._stdout, self._stderr):
            if stream is not None and not stream.closed:
                stream.close()
        self._stdout = None
        self._stderr = None

    def _stop_isolated_process(self) -> None:
        try:
            if self.proc is not None and not self.attached_existing and self.proc.poll() is None:
                self.proc.terminate()
                try:
                    self.proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
                    self.proc.wait(timeout=10)
        finally:
            if not self.attached_existing:
                self.proc = None
            self._close_log_streams()

    def __exit__(self, exc_type, exc, tb) -> None:
        self._stop_isolated_process()

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
    from cortex_http_base_provider import CortexHTTPBaseMemoryProvider
    from cortex_http_pure_provider import CortexHTTPPureMemoryProvider
    from memory_bench.memory import REGISTRY

    REGISTRY["cortex-http"] = CortexHTTPMemoryProvider
    REGISTRY["cortex-http-base"] = CortexHTTPBaseMemoryProvider
    REGISTRY["cortex-http-pure"] = CortexHTTPPureMemoryProvider


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


def _resolve_memory_backend(args: argparse.Namespace) -> str:
    backend = str(getattr(args, "memory_backend", DEFAULT_MEMORY_BACKEND)).strip().lower()
    if backend not in SUPPORTED_MEMORY_BACKENDS:
        known = ", ".join(SUPPORTED_MEMORY_BACKENDS)
        raise ValueError(
            f"unsupported memory backend '{backend}'. Expected one of: {known}"
        )
    return backend



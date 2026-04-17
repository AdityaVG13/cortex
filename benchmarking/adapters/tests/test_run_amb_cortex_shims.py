from __future__ import annotations

import argparse
import importlib
import json
import os
import sqlite3
import sys
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[3]
BENCHMARKING_DIR = REPO_ROOT / "benchmarking"
if str(BENCHMARKING_DIR) not in sys.path:
    sys.path.insert(0, str(BENCHMARKING_DIR))
ADAPTERS_DIR = BENCHMARKING_DIR / "adapters"
if str(ADAPTERS_DIR) not in sys.path:
    sys.path.insert(0, str(ADAPTERS_DIR))

from run_amb_cortex import (  # noqa: E402
    IsolatedCortexDaemon,
    _apply_dataset_compat_shims,
    _cleanup_benchmark_rows_in_db,
    _configure_imports,
    _configure_llm_environment,
    _execute_single_run,
    _env_flag_enabled,
    _resolve_single_run_timeout_seconds,
    _seed_model_assets,
    build_parser,
)


class _Doc:
    def __init__(self, user_id: str, content: str) -> None:
        self.user_id = user_id
        self.content = content


class _ProviderDoc:
    def __init__(
        self,
        *,
        doc_id: str,
        content: str,
        user_id: str | None = None,
        timestamp: str | None = None,
        context: str | None = None,
    ) -> None:
        self.id = doc_id
        self.content = content
        self.user_id = user_id
        self.timestamp = timestamp
        self.context = context


class _DatasetWithoutUserIds:
    name = "compat-dataset"

    def load_documents(
        self,
        split: str,
        category: str | None = None,
        limit: int | None = None,
        ids: set[str] | None = None,
    ) -> list[_Doc]:
        _ = (split, category, limit, ids)
        return [
            _Doc(user_id="u1", content="alpha"),
            _Doc(user_id="u2", content="beta"),
        ]


class _LongMemStyleDataset:
    name = "longmemeval"

    def build_rag_prompt(
        self,
        query: str,
        context: str,
        task_type: str,
        split: str,
        category: str | None = None,
        meta: dict | None = None,
    ) -> str:
        # Simulate legacy behavior that incorrectly prioritizes raw payload JSON.
        _ = (query, task_type, split, category)
        payload = (meta or {}).get("_raw_response")
        ctx = json.dumps(payload) if payload else context
        return f"PROMPT::{ctx}"


class _MemBenchStyleDataset:
    name = "membench"

    def __init__(self, data_path: Path) -> None:
        self.data_path = data_path

    def _load_trajectories(self, split: str) -> list[dict[str, object]]:
        _ = split
        raise UnicodeDecodeError("cp1252", b"\x9d", 0, 1, "character maps to <undefined>")


def test_load_documents_shim_accepts_user_ids_and_filters_results() -> None:
    dataset = _apply_dataset_compat_shims(_DatasetWithoutUserIds())
    docs = dataset.load_documents("s", user_ids={"u1"})  # type: ignore[attr-defined]
    assert len(docs) == 1
    assert docs[0].user_id == "u1"
    assert docs[0].content == "alpha"


def test_longmemeval_shim_keeps_context_and_appends_compact_metrics() -> None:
    dataset = _apply_dataset_compat_shims(_LongMemStyleDataset())
    prompt = dataset.build_rag_prompt(  # type: ignore[attr-defined]
        "What degree did I graduate with?",
        "Business Administration is the degree.",
        "open",
        "s",
        None,
        {
            "_raw_response": {
                "budget": 300,
                "spent": 210,
                "saved": 90,
                "count": 2,
                "mode": "balanced",
                "tier": "standard",
                "results": [{"excerpt": "truncated"}],
            }
        },
    )
    assert "Business Administration" in prompt
    assert "[retrieval-metrics]" in prompt
    assert "\"budget\": 300" in prompt
    assert "\"count\": 2" in prompt


def test_seed_model_assets_copies_only_supported_files(tmp_path: Path) -> None:
    cache_dir = tmp_path / "cache-models"
    target_dir = tmp_path / "daemon-models"
    cache_dir.mkdir(parents=True, exist_ok=True)
    (cache_dir / "all-MiniLM-L6-v2.onnx").write_bytes(b"onnx")
    (cache_dir / "tokenizer.json").write_text("{}", encoding="utf-8")
    (cache_dir / "notes.txt").write_text("skip", encoding="utf-8")
    (cache_dir / "nested").mkdir()
    (cache_dir / "nested" / "ignored.onnx").write_bytes(b"ignored")

    copied = _seed_model_assets(cache_dir, target_dir)
    assert copied == 2
    assert (target_dir / "all-MiniLM-L6-v2.onnx").exists()
    assert (target_dir / "tokenizer.json").exists()
    assert not (target_dir / "notes.txt").exists()

    copied_again = _seed_model_assets(cache_dir, target_dir)
    assert copied_again == 0


def test_membench_shim_recovers_from_windows_decode_errors(tmp_path: Path) -> None:
    source = tmp_path / "FirstAgentDataLowLevel.json"
    source.write_text(
        json.dumps({"simple": {"roleA": [{"tid": "t1", "message_list": [], "QA": {}}]}}, ensure_ascii=False),
        encoding="utf-8",
    )
    dataset = _apply_dataset_compat_shims(_MemBenchStyleDataset(tmp_path))
    trajectories = dataset._load_trajectories("FirstAgentLowLevel")  # type: ignore[attr-defined]
    assert len(trajectories) == 1
    assert trajectories[0]["tid"] == "t1"
    assert trajectories[0]["_question_type"] == "simple"


def test_configure_llm_environment_normalizes_gemini_keys(monkeypatch) -> None:
    monkeypatch.delenv("OMB_ANSWER_LLM", raising=False)
    monkeypatch.delenv("OMB_JUDGE_LLM", raising=False)
    monkeypatch.delenv("GOOGLE_API_KEY", raising=False)
    monkeypatch.setenv("GEMINI_API_KEY", "test-key")

    provider = _configure_llm_environment()

    assert provider == "gemini"
    assert os.environ.get("GOOGLE_API_KEY") == "test-key"
    assert os.environ.get("GEMINI_API_KEY") is None
    assert os.environ.get("OMB_ANSWER_LLM") == "gemini"
    assert os.environ.get("OMB_JUDGE_LLM") == "gemini"


def test_lock_conflict_detection_reads_stderr(tmp_path: Path) -> None:
    daemon = IsolatedCortexDaemon(tmp_path)
    daemon.stderr_path.write_text(
        "[cortex] FATAL: another cortex instance holds the lock\n",
        encoding="utf-8",
    )
    assert daemon._lock_conflict_detected() is True


def test_daemon_mode_reflects_attach_state(tmp_path: Path) -> None:
    daemon = IsolatedCortexDaemon(tmp_path)
    assert daemon.daemon_mode == "isolated-benchmark"
    daemon.attached_existing = True
    assert daemon.daemon_mode == "app-owned-attached"


def test_env_flag_enabled_parses_common_values(monkeypatch) -> None:
    monkeypatch.delenv("CORTEX_SAMPLE_FLAG", raising=False)
    assert _env_flag_enabled("CORTEX_SAMPLE_FLAG", default=True) is True
    assert _env_flag_enabled("CORTEX_SAMPLE_FLAG", default=False) is False

    monkeypatch.setenv("CORTEX_SAMPLE_FLAG", "off")
    assert _env_flag_enabled("CORTEX_SAMPLE_FLAG", default=True) is False

    monkeypatch.setenv("CORTEX_SAMPLE_FLAG", "YES")
    assert _env_flag_enabled("CORTEX_SAMPLE_FLAG", default=False) is True


def test_daemon_requires_live_app_daemon_when_enforced(tmp_path: Path, monkeypatch) -> None:
    daemon = IsolatedCortexDaemon(tmp_path)
    monkeypatch.setenv("CORTEX_BENCHMARK_REQUIRE_APP_DAEMON", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_ATTACH_EXISTING_DAEMON", "1")
    monkeypatch.setattr(daemon, "_try_attach_existing_daemon", lambda: False)

    with pytest.raises(RuntimeError, match="App-owned Cortex daemon is required"):
        daemon.__enter__()


def test_cleanup_benchmark_rows_in_db_removes_only_matching_source_agent(tmp_path: Path) -> None:
    db_path = tmp_path / "cortex.db"
    conn = sqlite3.connect(db_path)
    conn.execute("CREATE TABLE decisions (id INTEGER PRIMARY KEY, source_agent TEXT, status TEXT)")
    conn.execute(
        "CREATE TABLE embeddings (id INTEGER PRIMARY KEY, target_type TEXT, target_id INTEGER)"
    )
    conn.execute("CREATE TABLE events (id INTEGER PRIMARY KEY, source_agent TEXT)")
    conn.executemany(
        "INSERT INTO decisions (id, source_agent, status) VALUES (?, ?, 'active')",
        [
            (1, "amb-cortex::run-a"),
            (2, "amb-cortex::run-a"),
            (3, "codex"),
        ],
    )
    conn.executemany(
        "INSERT INTO embeddings (target_type, target_id) VALUES (?, ?)",
        [
            ("decision", 1),
            ("decision", 2),
            ("decision", 3),
            ("memory", 99),
        ],
    )
    conn.executemany(
        "INSERT INTO events (source_agent) VALUES (?)",
        [("amb-cortex::run-a",), ("codex",)],
    )
    conn.commit()
    conn.close()

    result = _cleanup_benchmark_rows_in_db(db_path, "amb-cortex::run-a")

    assert result["decisions_deleted"] == 2
    assert result["embeddings_deleted"] == 2
    assert result["events_deleted"] == 1

    check = sqlite3.connect(db_path)
    try:
        assert check.execute("SELECT COUNT(*) FROM decisions").fetchone()[0] == 1
        assert (
            check.execute("SELECT COUNT(*) FROM decisions WHERE source_agent='codex'").fetchone()[0]
            == 1
        )
        assert check.execute("SELECT COUNT(*) FROM embeddings").fetchone()[0] == 2
        assert (
            check.execute(
                "SELECT COUNT(*) FROM embeddings WHERE target_type='decision' AND target_id=3"
            ).fetchone()[0]
            == 1
        )
        assert (
            check.execute("SELECT COUNT(*) FROM events WHERE source_agent='codex'").fetchone()[0]
            == 1
        )
    finally:
        check.close()


def test_cortex_provider_concurrency_defaults_to_one_and_allows_override(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()

    monkeypatch.delenv("CORTEX_BENCHMARK_PROVIDER_CONCURRENCY", raising=False)
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    assert module.CortexHTTPMemoryProvider.concurrency == 1

    monkeypatch.setenv("CORTEX_BENCHMARK_PROVIDER_CONCURRENCY", "3")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    assert module.CortexHTTPMemoryProvider.concurrency == 3


def test_cortex_provider_expands_json_docs_with_fact_extracts(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "2")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    doc = _ProviderDoc(
        doc_id="d1",
        user_id="u1",
        timestamp="2026-04-17T08:00:00Z",
        context="session-alpha",
        content=json.dumps(
            [
                {"role": "user", "content": "I graduated with Business Administration in 2018."},
                {"role": "assistant", "content": "You mentioned your commute is 45 minutes each way."},
            ]
        ),
    )

    expanded = provider._expand_document(doc)
    provider.cleanup()

    assert len(expanded) >= 2
    assert expanded[0].id == "d1"
    assert expanded[1].id == "d1::fact::1"
    assert any("Business Administration" in item.content for item in expanded[1:])
    assert any("45 minutes each way" in item.content for item in expanded[1:])
    assert expanded[1].context.endswith("[fact-extract]")


def test_cortex_provider_extracts_late_fact_sentences(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "1")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    long_intro = " ".join(["I like productivity tools."] * 40)
    doc = _ProviderDoc(
        doc_id="d2",
        user_id="u2",
        timestamp="2026-04-17T08:00:00Z",
        context="session-beta",
        content=json.dumps(
            [
                {
                    "role": "user",
                    "content": f"{long_intro} By the way, I graduated with a degree in Business Administration.",
                }
            ]
        ),
    )

    expanded = provider._expand_document(doc)
    provider.cleanup()

    assert len(expanded) == 2
    assert expanded[1].id == "d2::fact::1"
    assert "Business Administration" in expanded[1].content


def test_cortex_provider_can_disable_fact_extracts(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "0")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    doc = _ProviderDoc(
        doc_id="d1",
        user_id="u1",
        content=json.dumps([{"role": "user", "content": "I packed seven shirts for Costa Rica."}]),
    )

    expanded = provider._expand_document(doc)
    provider.cleanup()

    assert len(expanded) == 1
    assert expanded[0].id == "d1"


def test_cortex_provider_can_disable_full_document_storage(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_STORE_FULL_DOCS", "0")
    monkeypatch.setenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "2")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    doc = _ProviderDoc(
        doc_id="d1",
        user_id="u1",
        content=json.dumps(
            [
                {"role": "user", "content": "I packed seven shirts for Costa Rica."},
                {"role": "user", "content": "I upgraded my home internet to 500 Mbps."},
            ]
        ),
    )

    expanded = provider._expand_document(doc)
    provider.cleanup()

    assert len(expanded) >= 1
    assert all("::fact::" in item.id for item in expanded)


def test_run_parser_defaults_single_run_timeout_to_20_minutes(monkeypatch) -> None:
    monkeypatch.delenv("CORTEX_BENCHMARK_RUN_MAX_RUNTIME_SECONDS", raising=False)

    parser = build_parser()
    args = parser.parse_args(["run", "--dataset", "longmemeval", "--split", "s"])

    assert args.max_runtime_seconds == 1200


def test_run_parser_reads_single_run_timeout_from_env(monkeypatch) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_RUN_MAX_RUNTIME_SECONDS", "900")

    parser = build_parser()
    args = parser.parse_args(["run", "--dataset", "longmemeval", "--split", "s"])

    assert args.max_runtime_seconds == 900


def test_single_run_timeout_validation_enforces_15_to_20_min_window() -> None:
    assert _resolve_single_run_timeout_seconds(900) == 900
    assert _resolve_single_run_timeout_seconds(1200) == 1200

    with pytest.raises(ValueError, match="between 900 and 1200"):
        _resolve_single_run_timeout_seconds(899)
    with pytest.raises(ValueError, match="between 900 and 1200"):
        _resolve_single_run_timeout_seconds(1201)


def test_execute_single_run_uses_validated_timeout(monkeypatch, tmp_path: Path) -> None:
    captured: dict[str, object] = {}

    def _fake_execute(
        *,
        run_args: argparse.Namespace,
        run_dir: Path,
        timeout_seconds: int,
        timeout_label: str,
    ) -> tuple[int, str | None]:
        captured["run_args"] = run_args
        captured["run_dir"] = run_dir
        captured["timeout_seconds"] = timeout_seconds
        captured["timeout_label"] = timeout_label
        return 0, None

    monkeypatch.setattr("run_amb_cortex._execute_benchmark_with_timeout", _fake_execute)
    args = argparse.Namespace(max_runtime_seconds=1000)

    _execute_single_run(args, tmp_path)

    assert captured["run_args"] is args
    assert captured["run_dir"] == tmp_path
    assert captured["timeout_seconds"] == 1000
    assert captured["timeout_label"] == "single run"


def test_execute_single_run_raises_when_timeout_guard_fails(monkeypatch, tmp_path: Path) -> None:
    def _fake_execute(
        *,
        run_args: argparse.Namespace,
        run_dir: Path,
        timeout_seconds: int,
        timeout_label: str,
    ) -> tuple[int, str | None]:
        _ = (run_args, run_dir, timeout_seconds, timeout_label)
        return 124, "single run runtime budget exceeded (900s cap)"

    monkeypatch.setattr("run_amb_cortex._execute_benchmark_with_timeout", _fake_execute)

    with pytest.raises(RuntimeError, match="runtime budget exceeded"):
        _execute_single_run(argparse.Namespace(max_runtime_seconds=900), tmp_path)

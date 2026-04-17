from __future__ import annotations

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

from run_amb_cortex import (  # noqa: E402
    IsolatedCortexDaemon,
    _apply_dataset_compat_shims,
    _cleanup_benchmark_rows_in_db,
    _configure_llm_environment,
    _env_flag_enabled,
    _seed_model_assets,
)


class _Doc:
    def __init__(self, user_id: str, content: str) -> None:
        self.user_id = user_id
        self.content = content


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

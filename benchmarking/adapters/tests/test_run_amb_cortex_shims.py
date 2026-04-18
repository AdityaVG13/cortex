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

import run_amb_cortex

from run_amb_cortex import (  # noqa: E402
    IsolatedCortexDaemon,
    _apply_dataset_compat_shims,
    _build_profile_delta_report,
    _apply_retrieval_profile_defaults,
    _cleanup_benchmark_rows_in_db,
    _configure_imports,
    _configure_llm_environment,
    _context_efficiency_metrics,
    _execute_single_run,
    _env_flag_enabled,
    _resolve_quality_token_target,
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


class _CaptureStoreClient:
    def __init__(self) -> None:
        self.stored_docs: list[object] = []
        self.calls: list[list[str]] = []

    def healthcheck(self) -> dict[str, object]:
        return {"status": "ok"}

    def close(self) -> None:
        return None

    def reset_namespace(self, namespace: str) -> None:
        _ = namespace

    def store_documents(self, documents: list[object]) -> None:
        self.calls.append([str(getattr(doc, "id", "")) for doc in documents])
        self.stored_docs.extend(documents)

    def recall_documents(
        self,
        query: str,
        *,
        k: int = 10,
        user_id: str | None = None,
    ) -> tuple[list[object], dict[str, object]]:
        _ = (query, k, user_id)
        return [], {"results": []}


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
    assert "[answer-format]" in prompt
    assert "Rules:" in prompt
    assert "match the requested time frame" in prompt
    assert "include available city/state/country qualifiers" in prompt
    assert "concrete item phrase" in prompt
    assert "Valentine's Day" not in prompt


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
    monkeypatch.delenv("OMB_ANSWER_MODEL", raising=False)
    monkeypatch.delenv("OMB_JUDGE_MODEL", raising=False)
    monkeypatch.delenv("GOOGLE_API_KEY", raising=False)
    monkeypatch.setenv("GEMINI_API_KEY", "test-key")

    provider = _configure_llm_environment()

    assert provider == "gemini"
    assert os.environ.get("GOOGLE_API_KEY") == "test-key"
    assert os.environ.get("GEMINI_API_KEY") is None
    assert os.environ.get("OMB_ANSWER_LLM") == "gemini"
    assert os.environ.get("OMB_JUDGE_LLM") == "gemini"
    assert os.environ.get("OMB_ANSWER_MODEL") == "gemini-2.5-pro"
    assert os.environ.get("OMB_JUDGE_MODEL") == "gemini-2.5-flash"


def test_configure_llm_environment_preserves_explicit_model_overrides(monkeypatch) -> None:
    monkeypatch.delenv("OMB_ANSWER_LLM", raising=False)
    monkeypatch.delenv("OMB_JUDGE_LLM", raising=False)
    monkeypatch.setenv("OMB_ANSWER_MODEL", "gemini-2.5-flash")
    monkeypatch.setenv("OMB_JUDGE_MODEL", "gemini-2.5-flash-lite")
    monkeypatch.delenv("GOOGLE_API_KEY", raising=False)
    monkeypatch.setenv("GEMINI_API_KEY", "test-key")

    provider = _configure_llm_environment()

    assert provider == "gemini"
    assert os.environ.get("OMB_ANSWER_MODEL") == "gemini-2.5-flash"
    assert os.environ.get("OMB_JUDGE_MODEL") == "gemini-2.5-flash-lite"


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


def test_cleanup_benchmark_rows_in_db_retries_transient_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    db_path = tmp_path / "cortex.db"
    conn = sqlite3.connect(db_path)
    conn.execute("CREATE TABLE decisions (id INTEGER PRIMARY KEY, source_agent TEXT, status TEXT)")
    conn.execute(
        "CREATE TABLE embeddings (id INTEGER PRIMARY KEY, target_type TEXT, target_id INTEGER)"
    )
    conn.execute("CREATE TABLE events (id INTEGER PRIMARY KEY, source_agent TEXT)")
    conn.execute("INSERT INTO decisions (id, source_agent, status) VALUES (1, 'amb-cortex::run-a', 'active')")
    conn.execute("INSERT INTO embeddings (target_type, target_id) VALUES ('decision', 1)")
    conn.execute("INSERT INTO events (source_agent) VALUES ('amb-cortex::run-a')")
    conn.commit()
    conn.close()

    real_connect = sqlite3.connect
    attempts = {"count": 0}

    def flaky_connect(*args, **kwargs):
        attempts["count"] += 1
        if attempts["count"] == 1:
            raise sqlite3.OperationalError("database is locked")
        return real_connect(*args, **kwargs)

    monkeypatch.setattr(run_amb_cortex.sqlite3, "connect", flaky_connect)
    monkeypatch.setattr(run_amb_cortex.time, "sleep", lambda _: None)

    result = _cleanup_benchmark_rows_in_db(db_path, "amb-cortex::run-a")

    assert attempts["count"] >= 2
    assert result["cleanup_retry_attempts"] == 1
    assert result["decisions_deleted"] == 1


def test_cleanup_benchmark_namespace_captures_cleanup_errors(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    db_path = tmp_path / "cortex.db"
    db_path.write_text("", encoding="utf-8")

    monkeypatch.setattr(run_amb_cortex, "_runtime_db_path_from_health", lambda _base_url: db_path)
    monkeypatch.setattr(
        run_amb_cortex,
        "_cleanup_benchmark_rows_in_db",
        lambda _db_path, _source_agent: (_ for _ in ()).throw(
            sqlite3.OperationalError("database is locked")
        ),
    )

    report = run_amb_cortex._cleanup_benchmark_namespace(
        base_url="http://127.0.0.1:7437",
        source_agent="amb-cortex::run-a",
    )

    assert report["cleanup_attempted"] is True
    assert report["cleanup_failed"] is True
    assert "database is locked" in str(report["cleanup_error"])


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
    monkeypatch.setenv("CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS", "1")
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


def test_cortex_provider_default_fact_extracts_exclude_assistant_summaries(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "3")
    monkeypatch.delenv("CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS", raising=False)
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    doc = _ProviderDoc(
        doc_id="d1",
        user_id="u1",
        content=json.dumps(
            [
                {"role": "user", "content": "I upgraded my home internet to 500 Mbps."},
                {"role": "assistant", "content": "You mentioned your commute is 45 minutes each way."},
            ]
        ),
    )

    expanded = provider._expand_document(doc)
    provider.cleanup()

    fact_contents = [item.content for item in expanded if "::fact::" in item.id]
    assert fact_contents
    assert any("500 Mbps" in content for content in fact_contents)
    assert all("45 minutes each way" not in content for content in fact_contents)


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


def test_cortex_provider_extracts_take_verb_location_facts(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "1")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    doc = _ProviderDoc(
        doc_id="d3",
        user_id="u3",
        content=json.dumps(
            [
                {
                    "role": "user",
                    "content": "I take yoga classes at serenity yoga every Tuesday evening.",
                }
            ]
        ),
    )

    expanded = provider._expand_document(doc)
    provider.cleanup()

    assert len(expanded) == 2
    assert expanded[1].id == "d3::fact::1"
    assert "serenity yoga" in expanded[1].content.lower()


def test_cortex_provider_extracts_item_facts_from_gift_statements(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "1")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    doc = _ProviderDoc(
        doc_id="d4",
        user_id="u4",
        content=json.dumps(
            [
                {
                    "role": "user",
                    "content": "My sister's birthday gift was a yellow dress from last weekend.",
                }
            ]
        ),
    )

    expanded = provider._expand_document(doc)
    provider.cleanup()

    assert len(expanded) == 2
    assert expanded[1].id == "d4::fact::1"
    assert "yellow dress" in expanded[1].content.lower()


def test_cortex_provider_extracts_short_user_detail_replies(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "3")
    monkeypatch.setenv("CORTEX_BENCHMARK_STORE_FULL_DOCS", "0")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    doc = _ProviderDoc(
        doc_id="d5",
        user_id="u5",
        content=json.dumps(
            [
                {"role": "assistant", "content": "Where did you redeem the $5 coffee creamer coupon?"},
                {"role": "user", "content": "Target."},
                {"role": "assistant", "content": "What did you buy for your sister's birthday gift?"},
                {"role": "user", "content": "A yellow dress."},
                {"role": "assistant", "content": "When was the fundraiser dinner?"},
                {"role": "user", "content": "February 14th."},
            ]
        ),
    )

    expanded = provider._expand_document(doc)
    provider.cleanup()

    fact_contents = [item.content.lower() for item in expanded if "::fact::" in item.id]
    assert fact_contents
    assert any("target" in content for content in fact_contents)
    assert any("yellow dress" in content for content in fact_contents)
    assert any("february 14th" in content for content in fact_contents)
    assert any("[assistant-question]" in content for content in fact_contents)
    assert any(
        content.index("[user-answer]") < content.index("[assistant-question]")
        for content in fact_contents
        if "[assistant-question]" in content and "[user-answer]" in content
    )


def test_cortex_provider_short_reply_snippets_prioritize_user_answer(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "2")
    monkeypatch.setenv("CORTEX_BENCHMARK_STORE_FULL_DOCS", "0")
    monkeypatch.setenv("CORTEX_BENCHMARK_SHORT_REPLY_QUESTION_MAX_CHARS", "40")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    doc = _ProviderDoc(
        doc_id="d6",
        user_id="u6",
        content=json.dumps(
            [
                {
                    "role": "assistant",
                    "content": (
                        "What speed is your new internet plan after the upgrade, "
                        "and does it help when streaming movies?"
                    ),
                },
                {"role": "user", "content": "I upgraded to 500 Mbps."},
            ]
        ),
    )

    expanded = provider._expand_document(doc)
    provider.cleanup()

    fact_contents = [item.content.lower() for item in expanded if "::fact::" in item.id]
    assert fact_contents
    assert any("[user-answer] i upgraded to 500 mbps" in content for content in fact_contents)
    assert any("[assistant-question]" in content for content in fact_contents)


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


def test_cortex_provider_splits_oversized_store_documents_deterministically(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "0")
    monkeypatch.setenv("CORTEX_BENCHMARK_STORE_FULL_DOCS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_STORE_MAX_CHARS", "120")
    monkeypatch.setenv("CORTEX_BENCHMARK_INGEST_FLUSH_SIZE", "1")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    capture = _CaptureStoreClient()
    provider._http = capture
    oversized = (
        "I attended the University of Melbourne in Australia and stayed near campus. "
        "I also tracked my commute details for each weekday while studying abroad. "
        "These details should be split deterministically for store ingestion."
    )
    doc = _ProviderDoc(
        doc_id="oversized-doc",
        user_id="u-oversized",
        context="beam-session",
        content=oversized,
    )

    provider.ingest([doc])
    provider.cleanup()

    stored = [item for item in capture.stored_docs if str(getattr(item, "id", "")).startswith("oversized-doc")]
    assert len(stored) >= 2
    assert "".join(str(getattr(item, "content", "")) for item in stored) == oversized
    assert all(len(str(getattr(item, "content", ""))) <= 120 for item in stored)
    expected_ids = [f"oversized-doc::part::{idx:02d}" for idx in range(1, len(stored) + 1)]
    assert [str(getattr(item, "id", "")) for item in stored] == expected_ids
    assert all("[store-part " in str(getattr(item, "context", "")) for item in stored)


def test_cortex_provider_splits_oversized_fact_extracts_without_dropping_detail(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_STORE_FULL_DOCS", "0")
    monkeypatch.setenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS", "500")
    monkeypatch.setenv("CORTEX_BENCHMARK_STORE_MAX_CHARS", "90")
    monkeypatch.setenv("CORTEX_BENCHMARK_INGEST_FLUSH_SIZE", "10")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    capture = _CaptureStoreClient()
    provider._http = capture
    doc = _ProviderDoc(
        doc_id="gift-doc",
        user_id="u-gift",
        context="beam-gift",
        content=json.dumps(
            [
                {
                    "role": "user",
                    "content": (
                        "I bought a yellow dress from Nordstrom in Seattle on March 5, 2024 "
                        "for my sister's birthday and matched it with silver accessories."
                    ),
                }
            ]
        ),
    )

    expanded = provider._expand_document(doc)
    assert len(expanded) == 1
    expected_fact = expanded[0].content

    provider.ingest([doc])
    provider.cleanup()

    fact_parts = [item for item in capture.stored_docs if str(getattr(item, "id", "")).startswith("gift-doc::fact::1::part::")]
    assert len(fact_parts) >= 2
    assert "".join(str(getattr(item, "content", "")) for item in fact_parts) == expected_fact
    assert all(len(str(getattr(item, "content", ""))) <= 90 for item in fact_parts)
    assert any("yellow dress" in str(getattr(item, "content", "")).lower() for item in fact_parts)
    assert any("seattle" in str(getattr(item, "content", "")).lower() for item in fact_parts)


def test_cortex_provider_keeps_small_store_documents_unsplit(monkeypatch) -> None:
    module_name = "cortex_amb_provider"
    _configure_imports()
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", "0")
    monkeypatch.setenv("CORTEX_BENCHMARK_STORE_FULL_DOCS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_STORE_MAX_CHARS", "500")
    monkeypatch.setenv("CORTEX_BENCHMARK_INGEST_FLUSH_SIZE", "1")
    sys.modules.pop(module_name, None)
    module = importlib.import_module(module_name)
    provider = module.CortexHTTPMemoryProvider()
    capture = _CaptureStoreClient()
    provider._http = capture
    doc = _ProviderDoc(
        doc_id="small-doc",
        user_id="u-small",
        context="beam-small",
        content="I moved to Denver in 2021.",
    )

    provider.ingest([doc])
    provider.cleanup()

    assert len(capture.stored_docs) == 1
    stored = capture.stored_docs[0]
    assert str(getattr(stored, "id", "")) == "small-doc"
    assert str(getattr(stored, "content", "")) == "I moved to Denver in 2021."


def test_apply_retrieval_profile_defaults_is_non_destructive(monkeypatch) -> None:
    monkeypatch.delenv("CORTEX_BENCHMARK_STORE_FULL_DOCS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_SIBLING_SCORE_MARGIN", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_SHORT_REPLY_QUESTION_MAX_CHARS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_CONTEXT_MAX_CHARS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_QUERY_WINDOW_CHARS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_MAX_QUERY_WINDOWS_PER_TERM", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_USE_RECALL_EXCERPTS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_ANSWER_SOURCE_PENALTY", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", raising=False)
    monkeypatch.setenv("CORTEX_BENCHMARK_CONTEXT_MAX_CHARS", "999")

    effective = _apply_retrieval_profile_defaults("balanced")

    assert effective["CORTEX_BENCHMARK_STORE_FULL_DOCS"] == "0"
    assert effective["CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS"] == "1"
    assert effective["CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC"] == "14"
    assert effective["CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS"] == "800"
    assert effective["CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS"] == "0"
    assert effective["CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS"] == "1"
    assert effective["CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO"] == "0.35"
    assert effective["CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET"] == "96"
    assert effective["CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER"] == "12"
    assert effective["CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN"] == "120"
    assert effective["CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED"] == "2"
    assert effective["CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS"] == "10"
    assert effective["CORTEX_BENCHMARK_DETAIL_SIBLING_SCORE_MARGIN"] == "18"
    assert effective["CORTEX_BENCHMARK_SHORT_REPLY_QUESTION_MAX_CHARS"] == "160"
    assert effective["CORTEX_BENCHMARK_CONTEXT_MAX_CHARS"] == "999"
    assert effective["CORTEX_BENCHMARK_QUERY_WINDOW_CHARS"] == "240"
    assert effective["CORTEX_BENCHMARK_MAX_QUERY_WINDOWS_PER_TERM"] == "3"
    assert effective["CORTEX_BENCHMARK_USE_RECALL_EXCERPTS"] == "1"
    assert effective["CORTEX_BENCHMARK_ANSWER_SOURCE_PENALTY"] == "24"
    assert effective["CORTEX_BENCHMARK_RETRIEVAL_POLICY"] == "high-detail"


def test_apply_efficiency_5pct_profile_uses_detail_recovery_tuning(monkeypatch) -> None:
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_CONTEXT_MAX_CHARS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_QUERY_WINDOW_CHARS", raising=False)
    monkeypatch.delenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", raising=False)

    effective = _apply_retrieval_profile_defaults("efficiency-5pct")

    assert effective["CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER"] == "11"
    assert effective["CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN"] == "110"
    assert effective["CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED"] == "2"
    assert effective["CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS"] == "8"
    assert effective["CORTEX_BENCHMARK_CONTEXT_MAX_CHARS"] == "540"
    assert effective["CORTEX_BENCHMARK_QUERY_WINDOW_CHARS"] == "220"
    assert effective["CORTEX_BENCHMARK_RETRIEVAL_POLICY"] == "high-detail"


def test_context_efficiency_metrics_derives_score_per_token() -> None:
    summary = argparse.Namespace(
        correct=13,
        results=[
            argparse.Namespace(context_tokens=120),
            argparse.Namespace(context_tokens=80),
            {"context_tokens": 100},
        ],
    )

    metrics = _context_efficiency_metrics(summary)

    assert metrics["context_tokens_total"] == 300
    assert metrics["context_tokens_avg"] == 100.0
    assert metrics["score_per_1k_context_tokens"] == 43.3333


def test_recall_summarization_and_efficiency_metrics_track_total_tokens() -> None:
    recall_stats = run_amb_cortex._summarize_recall_metrics(
        [
            {"token_estimate": 120, "recall_call_count": 1},
            {"token_estimate": 180, "recall_call_count": 2},
        ],
        budget=300,
    )
    summary = argparse.Namespace(correct=15)

    efficiency = run_amb_cortex._recall_efficiency_metrics(summary, recall_stats)

    assert recall_stats["queries"] == 2
    assert recall_stats["total_recall_tokens"] == 300
    assert recall_stats["avg_recall_tokens"] == 150.0
    assert recall_stats["recall_calls"] == 3
    assert recall_stats["avg_recall_calls_per_query"] == 1.5
    assert recall_stats["avg_recall_tokens_per_call"] == 100.0
    assert efficiency["recall_tokens_total"] == 300
    assert efficiency["score_per_1k_recall_tokens"] == 50.0


def test_profile_delta_report_compares_effective_limits_cleanly() -> None:
    report = _build_profile_delta_report(
        token_limits={
            "mode": "dynamic",
            "max_recall_tokens": 330,
            "max_avg_recall_tokens": 260.0,
        },
        effective_constraints={
            "max_recall_tokens": 300,
            "max_avg_recall_tokens": 240.0,
        },
        baseline_entry={
            "max_recall_tokens": 310,
            "max_avg_recall_tokens": 250.0,
        },
        recall_stats={
            "max_recall_tokens": 280,
            "avg_recall_tokens": 220.0,
        },
    )

    delta_vs_gate = report["delta_vs_token_gate"]["max_recall_tokens"]
    observed_vs_effective = report["observed_vs_effective"]["max_recall_tokens"]
    assert delta_vs_gate["absolute"] == -30.0
    assert observed_vs_effective["absolute"] == -20.0


def test_quality_token_target_resolver_maps_to_detail_safe_profile() -> None:
    plan = _resolve_quality_token_target(
        target="lean-detail",
        retrieval_profile="token-saver",
        min_accuracy=0.84,
    )

    assert plan["target"] == "lean-detail"
    assert plan["effective_retrieval_profile"] == "efficiency-5pct"
    assert plan["effective_min_accuracy"] == 0.88
    assert plan["applied"] is True


def test_run_parser_defaults_single_run_timeout_to_20_minutes(monkeypatch) -> None:
    monkeypatch.delenv("CORTEX_BENCHMARK_RUN_MAX_RUNTIME_SECONDS", raising=False)

    parser = build_parser()
    args = parser.parse_args(["run", "--dataset", "longmemeval", "--split", "s"])

    assert args.max_runtime_seconds == 1200
    assert args.quality_token_target == "custom"
    assert args.retrieval_profile == "max-quality"


def test_run_parser_reads_single_run_timeout_from_env(monkeypatch) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_RUN_MAX_RUNTIME_SECONDS", "900")

    parser = build_parser()
    args = parser.parse_args(["run", "--dataset", "longmemeval", "--split", "s"])

    assert args.max_runtime_seconds == 900


def test_run_parser_accepts_retrieval_profile_override() -> None:
    parser = build_parser()
    args = parser.parse_args(
        [
            "run",
            "--dataset",
            "longmemeval",
            "--split",
            "s",
            "--retrieval-profile",
            "token-saver",
        ]
    )
    assert args.retrieval_profile == "token-saver"


def test_run_parser_accepts_efficiency_retrieval_profile() -> None:
    parser = build_parser()
    args = parser.parse_args(
        [
            "run",
            "--dataset",
            "longmemeval",
            "--split",
            "s",
            "--retrieval-profile",
            "efficiency-5pct",
        ]
    )
    assert args.retrieval_profile == "efficiency-5pct"


def test_run_parser_accepts_quality_token_target() -> None:
    parser = build_parser()
    args = parser.parse_args(
        [
            "run",
            "--dataset",
            "longmemeval",
            "--split",
            "s",
            "--quality-token-target",
            "balanced-detail",
        ]
    )
    assert args.quality_token_target == "balanced-detail"


def test_run_parser_accepts_efficiency_3pct_retrieval_profile() -> None:
    parser = build_parser()
    args = parser.parse_args(
        [
            "run",
            "--dataset",
            "longmemeval",
            "--split",
            "s",
            "--retrieval-profile",
            "efficiency-3pct",
        ]
    )
    assert args.retrieval_profile == "efficiency-3pct"


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
    args = argparse.Namespace(
        max_runtime_seconds=1000,
        oracle=False,
        query_id=None,
        doc_limit=None,
    )

    _execute_single_run(args, tmp_path)

    assert captured["run_args"] is args
    assert captured["run_dir"] == tmp_path
    assert captured["timeout_seconds"] == 1000
    assert captured["timeout_label"] == "single run"
    preflight = json.loads((tmp_path / "fair-run-preflight.json").read_text(encoding="utf-8"))
    assert preflight["passed"] is True


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
        _execute_single_run(
            argparse.Namespace(max_runtime_seconds=900, oracle=False, query_id=None, doc_limit=None),
            tmp_path,
        )


def test_execute_single_run_rejects_oracle_shortcut_before_execution(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    called = {"value": False}

    def _fake_execute(
        *,
        run_args: argparse.Namespace,
        run_dir: Path,
        timeout_seconds: int,
        timeout_label: str,
    ) -> tuple[int, str | None]:
        _ = (run_args, run_dir, timeout_seconds, timeout_label)
        called["value"] = True
        return 0, None

    monkeypatch.setattr("run_amb_cortex._execute_benchmark_with_timeout", _fake_execute)

    with pytest.raises(ValueError, match="fair-run preflight failed"):
        _execute_single_run(
            argparse.Namespace(max_runtime_seconds=900, oracle=True, query_id=None, doc_limit=None),
            tmp_path,
        )

    assert called["value"] is False
    preflight = json.loads((tmp_path / "fair-run-preflight.json").read_text(encoding="utf-8"))
    assert preflight["passed"] is False
    assert any("oracle=true" in item for item in preflight["violations"])


def test_execute_single_run_rejects_no_enforce_gate_shortcut_before_execution(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    called = {"value": False}

    def _fake_execute(
        *,
        run_args: argparse.Namespace,
        run_dir: Path,
        timeout_seconds: int,
        timeout_label: str,
    ) -> tuple[int, str | None]:
        _ = (run_args, run_dir, timeout_seconds, timeout_label)
        called["value"] = True
        return 0, None

    monkeypatch.setattr("run_amb_cortex._execute_benchmark_with_timeout", _fake_execute)

    with pytest.raises(ValueError, match="fair-run preflight failed"):
        _execute_single_run(
            argparse.Namespace(
                max_runtime_seconds=900,
                oracle=False,
                query_id=None,
                doc_limit=None,
                no_enforce_gate=True,
                allow_missing_recall_metrics=False,
            ),
            tmp_path,
        )

    assert called["value"] is False
    preflight = json.loads((tmp_path / "fair-run-preflight.json").read_text(encoding="utf-8"))
    assert preflight["passed"] is False
    assert any("no_enforce_gate=true" in item for item in preflight["violations"])


def test_execute_single_run_rejects_missing_metrics_shortcut_before_execution(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    called = {"value": False}

    def _fake_execute(
        *,
        run_args: argparse.Namespace,
        run_dir: Path,
        timeout_seconds: int,
        timeout_label: str,
    ) -> tuple[int, str | None]:
        _ = (run_args, run_dir, timeout_seconds, timeout_label)
        called["value"] = True
        return 0, None

    monkeypatch.setattr("run_amb_cortex._execute_benchmark_with_timeout", _fake_execute)

    with pytest.raises(ValueError, match="fair-run preflight failed"):
        _execute_single_run(
            argparse.Namespace(
                max_runtime_seconds=900,
                oracle=False,
                query_id=None,
                doc_limit=None,
                no_enforce_gate=False,
                allow_missing_recall_metrics=True,
            ),
            tmp_path,
        )

    assert called["value"] is False
    preflight = json.loads((tmp_path / "fair-run-preflight.json").read_text(encoding="utf-8"))
    assert preflight["passed"] is False
    assert any("allow_missing_recall_metrics=true" in item for item in preflight["violations"])

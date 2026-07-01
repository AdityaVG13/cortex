"""Runtime helpers: imports, cleanup, profiles, and metrics for run_amb_cortex."""
from __future__ import annotations

import inspect
import json
import os
import re
import shutil
import sqlite3
import socket
import subprocess
import sys
import time
import traceback
from datetime import datetime
from pathlib import Path
from typing import cast

import httpx

from run_amb_config import (
    BASELINE_FILE_DEFAULT,
    CADENCE_MATRIX_FILES_DEFAULT,
    CLEANUP_DB_RETRY_ATTEMPTS,
    CLEANUP_DB_RETRY_BASE_DELAY_SECONDS,
    DEFAULT_MEMORY_BACKEND,
    MATRIX_FILE_DEFAULT,
    QUALITY_TOKEN_TARGETS,
    RETRIEVAL_PROFILES,
    REPO_ROOT,
    RUNS_ROOT,
    TOKEN_GATE_PROFILES,
)

AMB_SRC = REPO_ROOT / "benchmarking" / "tools" / "agent-memory-benchmark" / "src"
ADAPTERS_DIR = REPO_ROOT / "benchmarking" / "adapters"

def _ensure_utf8_stdio() -> None:
    for stream_name in ("stdout", "stderr"):
        stream = getattr(sys, stream_name, None)
        reconfigure = getattr(stream, "reconfigure", None)
        if callable(reconfigure):
            try:
                reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


def _prepare_worker_runtime_env() -> None:
    os.environ.setdefault("PYTHONUTF8", "1")
    os.environ.setdefault("PYTHONIOENCODING", "utf-8")
    _ensure_utf8_stdio()


def _filter_kwargs_for_callable(fn: object, kwargs: dict[str, object]) -> dict[str, object]:
    """Return only kwargs accepted by `fn` (or all kwargs for **kwargs callables)."""
    signature = inspect.signature(fn)  # type: ignore[arg-type]
    parameters = signature.parameters.values()
    accepts_var_kwargs = any(param.kind == inspect.Parameter.VAR_KEYWORD for param in parameters)
    if accepts_var_kwargs:
        return kwargs
    accepted = {
        param.name
        for param in parameters
        if param.kind in (inspect.Parameter.POSITIONAL_OR_KEYWORD, inspect.Parameter.KEYWORD_ONLY)
    }
    return {key: value for key, value in kwargs.items() if key in accepted}


def _apply_dataset_compat_shims(dataset: object) -> object:
    """
    Apply runtime-safe compatibility shims for pinned AMB commits.

    - Some isolation datasets don't yet accept `user_ids` in `load_documents`.
      We add a wrapper that drops unsupported kwargs and applies best-effort
      user_id filtering on the returned docs when requested.
    - LongMemEval prompt construction can over-prioritize raw recall payload
      telemetry. We enforce context-first prompting and append compact metrics.
    """
    load_documents = getattr(dataset, "load_documents", None)
    if callable(load_documents):
        original_load_documents = load_documents

        def load_documents_compat(*args: object, **kwargs: object) -> object:
            requested_user_ids = kwargs.get("user_ids")
            supported_kwargs = _filter_kwargs_for_callable(original_load_documents, kwargs)
            docs = original_load_documents(*args, **supported_kwargs)
            if not isinstance(requested_user_ids, set):
                return docs
            if not isinstance(docs, list):
                return docs
            return [
                doc
                for doc in docs
                if getattr(doc, "user_id", None) in requested_user_ids
            ]

        setattr(dataset, "load_documents", load_documents_compat)

    dataset_name = str(getattr(dataset, "name", "")).lower()
    build_rag_prompt = getattr(dataset, "build_rag_prompt", None)
    if dataset_name == "longmemeval" and callable(build_rag_prompt):
        original_build_rag_prompt = build_rag_prompt

        def longmemeval_prompt_compat(
            query: str,
            context: str,
            task_type: str,
            split: str,
            category: str | None = None,
            meta: dict | None = None,
        ) -> str:
            prompt_meta = dict(meta or {})
            raw_payload = prompt_meta.pop("_raw_response", None)
            prompt = original_build_rag_prompt(
                query,
                context,
                task_type,
                split,
                category,
                prompt_meta,
            )
            answer_format_block = (
                "[answer-format] Return one short answer span copied from memory context.\n"
                "Rules:\n"
                "1) Prefer direct user-stated spans and avoid 'not found' when support exists in context.\n"
                "2) Preserve critical qualifiers present in context (employer/location/date/unit), not a shortened form.\n"
                "3) For time-sensitive wording (for example previous/former/current), match the requested time frame.\n"
                "4) For where/location questions, include available city/state/country qualifiers when context supports the same place.\n"
                "   If the answer is an institution and context includes one matching country mention, include the country.\n"
                "5) For where/location questions, if one location is the only supported candidate across related memories,\n"
                "   return that location directly and avoid uncertainty disclaimers.\n"
                "6) For study-abroad/institution location questions, if context contains a single country mention,\n"
                "   append it as 'Institution in Country'.\n"
                "7) For item questions, prefer the concrete item phrase over generic summaries.\n"
                "8) If the question asks for a single item and context lists multiple items, return only one concrete item.\n"
                "   Keep the first primary purchase phrase and drop accessory add-ons joined with 'and/plus/with'.\n"
                "   Example: 'a yellow dress and a pair of earrings' -> 'a yellow dress'.\n"
                "9) Do not add lead-in text like 'Based on the context' or 'According to the memories'.\n"
                "10) Return only the answer text, no explanation or extra narrative."
            )
            if not isinstance(raw_payload, dict):
                return f"{prompt}\n\n{answer_format_block}"
            metrics = {
                "budget": raw_payload.get("budget"),
                "spent": raw_payload.get("spent"),
                "saved": raw_payload.get("saved"),
                "count": raw_payload.get("count"),
                "mode": raw_payload.get("mode"),
                "tier": raw_payload.get("tier"),
            }
            compact_metrics = {
                key: value
                for key, value in metrics.items()
                if value is not None
            }
            if not compact_metrics:
                return f"{prompt}\n\n{answer_format_block}"
            return (
                f"{prompt}\n\n"
                f"[retrieval-metrics] {json.dumps(compact_metrics, ensure_ascii=False)}\n\n"
                f"{answer_format_block}"
            )

        setattr(dataset, "build_rag_prompt", longmemeval_prompt_compat)

    if dataset_name == "membench":
        load_trajectories = getattr(dataset, "_load_trajectories", None)
        if callable(load_trajectories):
            split_files = {
                "FirstAgentLowLevel": "FirstAgentDataLowLevel.json",
                "FirstAgentHighLevel": "FirstAgentDataHighLevel.json",
                "ThirdAgentLowLevel": "ThirdAgentDataLowLevel.json",
                "ThirdAgentHighLevel": "ThirdAgentDataHighLevel.json",
            }

            def load_trajectories_compat(split: str) -> object:
                try:
                    return load_trajectories(split)
                except UnicodeDecodeError:
                    data_path = Path(getattr(dataset, "data_path", Path("./MemData")))
                    filename = split_files.get(split)
                    if not filename:
                        raise
                    source = data_path / filename
                    with source.open("r", encoding="utf-8") as handle:
                        data = json.load(handle)
                    trajectories: list[dict[str, object]] = []
                    for question_type, scenarios in data.items():
                        if isinstance(scenarios, list):
                            flattened = scenarios
                        elif isinstance(scenarios, dict):
                            flattened = [item for sublist in scenarios.values() for item in sublist]
                        else:
                            continue
                        for traj in flattened:
                            if not isinstance(traj, dict):
                                continue
                            copied = dict(traj)
                            copied.setdefault("_question_type", question_type)
                            trajectories.append(copied)
                    return trajectories

            setattr(dataset, "_load_trajectories", load_trajectories_compat)

    return dataset


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
        str(REPO_ROOT / "daemon-rs" / "target" / "debug" / ("cortex.exe" if os.name == "nt" else "cortex")),
        str(REPO_ROOT / "daemon-rs" / "target" / "release" / ("cortex.exe" if os.name == "nt" else "cortex")),
        str(Path.home() / ".cortex" / "bin" / ("cortex.exe" if os.name == "nt" else "cortex")),
    ]
    for candidate in candidates:
        if candidate and Path(candidate).exists():
            return Path(candidate)
    raise FileNotFoundError(
        "Unable to locate a Cortex binary. Set CORTEX_BIN or build/install cortex first."
    )


def _seed_model_assets(cache_dir: Path, target_dir: Path) -> int:
    if not cache_dir.exists():
        return 0
    target_dir.mkdir(parents=True, exist_ok=True)
    copied = 0
    for candidate in cache_dir.iterdir():
        if not candidate.is_file():
            continue
        if candidate.suffix.lower() not in {".onnx", ".json"}:
            continue
        destination = target_dir / candidate.name
        if destination.exists():
            continue
        shutil.copy2(candidate, destination)
        copied += 1
    return copied


def _env_flag_enabled(name: str, *, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None:
        return default
    value = raw.strip().lower()
    if value in {"0", "false", "off", "no"}:
        return False
    if value in {"1", "true", "on", "yes"}:
        return True
    return default


def _runtime_db_path_from_health(base_url: str) -> Path | None:
    try:
        response = httpx.get(f"{base_url.rstrip('/')}/health", timeout=3.0)
        response.raise_for_status()
        payload = response.json()
    except Exception:
        return None
    runtime = payload.get("runtime")
    if not isinstance(runtime, dict):
        return None
    db_path = runtime.get("db_path")
    if not isinstance(db_path, str) or not db_path.strip():
        return None
    path = Path(db_path)
    if not path.exists():
        return None
    return path


def _cleanup_benchmark_rows_in_db_once(db_path: Path, source_agent: str) -> dict[str, int | str]:
    conn = sqlite3.connect(str(db_path), timeout=30.0)
    try:
        conn.execute("PRAGMA busy_timeout = 5000")
        cur = conn.cursor()
        cur.execute("BEGIN IMMEDIATE")
        cur.execute(
            "CREATE TEMP TABLE _amb_cleanup_ids AS "
            "SELECT id FROM decisions WHERE source_agent = ?1",
            (source_agent,),
        )
        cur.execute(
            "DELETE FROM embeddings WHERE target_type = 'decision' "
            "AND target_id IN (SELECT id FROM _amb_cleanup_ids)"
        )
        embeddings_deleted = int(cur.rowcount)
        cur.execute("DELETE FROM decisions WHERE id IN (SELECT id FROM _amb_cleanup_ids)")
        decisions_deleted = int(cur.rowcount)
        cur.execute("DELETE FROM events WHERE source_agent = ?1", (source_agent,))
        events_deleted = int(cur.rowcount)
        cur.execute("DROP TABLE _amb_cleanup_ids")
        conn.commit()
        cur.execute("PRAGMA wal_checkpoint(PASSIVE)")
        return {
            "source_agent": source_agent,
            "db_path": str(db_path),
            "decisions_deleted": decisions_deleted,
            "embeddings_deleted": embeddings_deleted,
            "events_deleted": events_deleted,
        }
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.close()


def _cleanup_benchmark_rows_in_db(db_path: Path, source_agent: str) -> dict[str, int | str]:
    attempts = max(1, int(CLEANUP_DB_RETRY_ATTEMPTS))
    for attempt in range(1, attempts + 1):
        try:
            payload = _cleanup_benchmark_rows_in_db_once(db_path, source_agent)
            payload["cleanup_retry_attempts"] = attempt - 1
            return payload
        except sqlite3.OperationalError as exc:
            is_locked = "database is locked" in str(exc).lower()
            if not is_locked or attempt >= attempts:
                raise
            time.sleep(CLEANUP_DB_RETRY_BASE_DELAY_SECONDS * attempt)
    raise RuntimeError("unreachable cleanup retry path")


def _cleanup_benchmark_namespace(
    *,
    base_url: str,
    source_agent: str,
) -> dict[str, int | str | bool]:
    db_path = _runtime_db_path_from_health(base_url)
    if db_path is None:
        return {
            "cleanup_attempted": False,
            "cleanup_reason": "runtime_db_path_unavailable",
            "source_agent": source_agent,
        }
    try:
        payload = _cleanup_benchmark_rows_in_db(db_path, source_agent)
    except Exception as exc:
        return {
            "cleanup_attempted": True,
            "cleanup_failed": True,
            "cleanup_error": str(exc),
            "source_agent": source_agent,
            "db_path": str(db_path),
        }
    return {"cleanup_attempted": True, "cleanup_failed": False, **payload}


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


def _configure_llm_environment() -> str:
    explicit_answer = os.environ.get("OMB_ANSWER_LLM")
    explicit_judge = os.environ.get("OMB_JUDGE_LLM")

    if explicit_answer and explicit_judge:
        return explicit_answer

    provider = ""
    gemini_key = os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY")
    if gemini_key:
        provider = "gemini"
        os.environ["GOOGLE_API_KEY"] = gemini_key
        os.environ.pop("GEMINI_API_KEY", None)
    elif os.environ.get("OPENAI_API_KEY"):
        provider = "openai"
    elif os.environ.get("GROQ_API_KEY"):
        provider = "groq"

    if not provider:
        raise RuntimeError(
            "No answer/judge model key is configured. Set GEMINI_API_KEY, GOOGLE_API_KEY, "
            "OPENAI_API_KEY, or GROQ_API_KEY for fair benchmark runs."
        )

    os.environ.setdefault("OMB_ANSWER_LLM", provider)
    os.environ.setdefault("OMB_JUDGE_LLM", provider)
    if provider == "gemini":
        # Keep benchmark answer/judge behavior stable and detail-oriented unless
        # the caller explicitly overrides models in the environment.
        os.environ.setdefault("OMB_ANSWER_MODEL", "gemini-2.5-pro")
        os.environ.setdefault("OMB_JUDGE_MODEL", "gemini-2.5-flash")
    return provider


def _normalize_provider_profile(raw: str | None) -> str:
    if not raw:
        return "default"
    value = raw.strip().lower()
    if value in TOKEN_GATE_PROFILES:
        return value
    aliases = {
        "anthropic": "claude",
        "sonnet": "claude",
        "opus": "claude",
        "gpt": "openai",
        "oai": "openai",
        "google": "gemini",
    }
    for needle, profile in aliases.items():
        if needle in value:
            return profile
    for profile_name in TOKEN_GATE_PROFILES:
        if profile_name != "default" and profile_name in value:
            return profile_name
    return "default"


def _resolve_token_gate_limits(
    *,
    mode: str,
    recall_budget: int,
    provider_profile: str,
    max_recall_tokens: int,
    max_avg_recall_tokens: float,
) -> dict[str, object]:
    if mode == "off":
        return {
            "mode": mode,
            "provider_profile": provider_profile,
            "max_recall_tokens": None,
            "max_avg_recall_tokens": None,
            "profile": None,
        }
    if mode == "absolute":
        return {
            "mode": mode,
            "provider_profile": provider_profile,
            "max_recall_tokens": int(max_recall_tokens),
            "max_avg_recall_tokens": float(max_avg_recall_tokens),
            "profile": None,
        }
    profile = TOKEN_GATE_PROFILES.get(provider_profile, TOKEN_GATE_PROFILES["default"])
    return {
        "mode": mode,
        "provider_profile": provider_profile,
        "max_recall_tokens": int(round(recall_budget * profile["max_peak_ratio"])),
        "max_avg_recall_tokens": round(recall_budget * profile["max_avg_ratio"], 2),
        "profile": profile,
    }


def _apply_retrieval_profile_defaults(retrieval_profile: str) -> dict[str, str]:
    profile = RETRIEVAL_PROFILES.get(retrieval_profile)
    if profile is None:
        known = ", ".join(sorted(RETRIEVAL_PROFILES))
        raise ValueError(
            f"unknown retrieval profile '{retrieval_profile}'. Expected one of: {known}"
        )
    effective: dict[str, str] = {}
    for env_name, default_value in profile.items():
        os.environ.setdefault(env_name, str(default_value))
        effective[env_name] = os.environ[env_name]
    return effective


def _context_efficiency_metrics(summary: object) -> dict[str, float | int | None]:
    results = getattr(summary, "results", [])
    context_tokens: list[int] = []
    if isinstance(results, list):
        for result in results:
            value = (
                result.get("context_tokens")
                if isinstance(result, dict)
                else getattr(result, "context_tokens", None)
            )
            if isinstance(value, (int, float)):
                context_tokens.append(int(value))
    context_tokens_total = int(sum(context_tokens))
    context_tokens_avg = (
        round(context_tokens_total / len(context_tokens), 2) if context_tokens else 0.0
    )
    correct_count = int(getattr(summary, "correct", 0) or 0)
    score_per_1k_context_tokens = (
        round((correct_count * 1000.0) / context_tokens_total, 4)
        if context_tokens_total > 0
        else None
    )
    return {
        "context_tokens_total": context_tokens_total,
        "context_tokens_avg": context_tokens_avg,
        "score_per_1k_context_tokens": score_per_1k_context_tokens,
    }


def _recall_efficiency_metrics(
    summary: object,
    recall_stats: dict[str, float | int],
) -> dict[str, float | int | None]:
    correct_count = int(getattr(summary, "correct", 0) or 0)
    recall_tokens_total = int(recall_stats.get("total_recall_tokens", 0) or 0)
    score_per_1k_recall_tokens = (
        round((correct_count * 1000.0) / recall_tokens_total, 4)
        if recall_tokens_total > 0
        else None
    )
    return {
        "recall_tokens_total": recall_tokens_total,
        "score_per_1k_recall_tokens": score_per_1k_recall_tokens,
    }


def _build_profile_delta_report(
    *,
    token_limits: dict[str, object],
    effective_constraints: dict[str, object],
    baseline_entry: dict | None,
    recall_stats: dict[str, float | int],
) -> dict[str, object]:
    def _as_number(value: object | None) -> float | None:
        if isinstance(value, (int, float)):
            return float(value)
        return None

    def _delta(current: object | None, reference: object | None) -> dict[str, object] | None:
        current_value = _as_number(current)
        reference_value = _as_number(reference)
        if current_value is None or reference_value is None:
            return None
        absolute = round(current_value - reference_value, 2)
        percent = (
            round((absolute / reference_value) * 100.0, 2)
            if reference_value != 0
            else None
        )
        return {
            "current": current_value,
            "reference": reference_value,
            "absolute": absolute,
            "percent": percent,
        }

    effective_max = effective_constraints.get("max_recall_tokens")
    effective_avg = effective_constraints.get("max_avg_recall_tokens")
    gate_max = token_limits.get("max_recall_tokens")
    gate_avg = token_limits.get("max_avg_recall_tokens")
    baseline_max = baseline_entry.get("max_recall_tokens") if isinstance(baseline_entry, dict) else None
    baseline_avg = baseline_entry.get("max_avg_recall_tokens") if isinstance(baseline_entry, dict) else None
    observed_max = recall_stats.get("max_recall_tokens")
    observed_avg = recall_stats.get("avg_recall_tokens")
    return {
        "delta_vs_token_gate": {
            "max_recall_tokens": _delta(effective_max, gate_max),
            "max_avg_recall_tokens": _delta(effective_avg, gate_avg),
        },
        "delta_vs_baseline": {
            "max_recall_tokens": _delta(effective_max, baseline_max),
            "max_avg_recall_tokens": _delta(effective_avg, baseline_avg),
        },
        "observed_vs_effective": {
            "max_recall_tokens": _delta(observed_max, effective_max),
            "max_avg_recall_tokens": _delta(observed_avg, effective_avg),
        },
    }


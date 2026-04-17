from __future__ import annotations

import argparse
import multiprocessing
import inspect
import json
import os
import re
import sqlite3
import shutil
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
BASELINE_FILE_DEFAULT = REPO_ROOT / "benchmarking" / "configs" / "token-gate-baselines.json"
MATRIX_FILE_DEFAULT = REPO_ROOT / "benchmarking" / "configs" / "amb-eval-matrix.stage1.json"
TOKEN_GATE_PROFILES: dict[str, dict[str, float]] = {
    # Tighter ratios for providers that tend to carry heavier prompt wrappers/history overhead.
    "claude": {"max_avg_ratio": 0.72, "max_peak_ratio": 0.90},
    "openai": {"max_avg_ratio": 0.80, "max_peak_ratio": 1.00},
    "codex": {"max_avg_ratio": 0.78, "max_peak_ratio": 0.98},
    "gemini": {"max_avg_ratio": 0.82, "max_peak_ratio": 1.00},
    "groq": {"max_avg_ratio": 0.84, "max_peak_ratio": 1.00},
    "default": {"max_avg_ratio": 0.80, "max_peak_ratio": 1.00},
}
SINGLE_RUN_TIMEOUT_ENV = "CORTEX_BENCHMARK_RUN_MAX_RUNTIME_SECONDS"
SINGLE_RUN_TIMEOUT_MIN_SECONDS = 900
SINGLE_RUN_TIMEOUT_MAX_SECONDS = 1200
SINGLE_RUN_TIMEOUT_DEFAULT_SECONDS = 1200


def _ensure_utf8_stdio() -> None:
    for stream_name in ("stdout", "stderr"):
        stream = getattr(sys, stream_name, None)
        reconfigure = getattr(stream, "reconfigure", None)
        if callable(reconfigure):
            try:
                reconfigure(encoding="utf-8", errors="replace")
            except Exception:
                pass


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
                "[answer-format] Return only the shortest direct answer phrase from memory context. "
                "Do not add explanations, qualifiers, or extra details beyond what was asked. "
                "Do not infer, compute, or normalize values; copy the exact wording from memory "
                "(for example, keep '45 minutes each way' rather than converting it to '90 minutes')."
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


def _cleanup_benchmark_rows_in_db(db_path: Path, source_agent: str) -> dict[str, int | str]:
    conn = sqlite3.connect(str(db_path), timeout=30.0)
    try:
        cur = conn.cursor()
        cur.execute("BEGIN")
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
    payload = _cleanup_benchmark_rows_in_db(db_path, source_agent)
    return {"cleanup_attempted": True, **payload}


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


def _resolve_baseline_path(raw_path: str) -> Path:
    path = Path(raw_path)
    if path.is_absolute():
        return path
    return (REPO_ROOT / path).resolve()


def _resolve_matrix_path(raw_path: str) -> Path:
    path = Path(raw_path)
    if path.is_absolute():
        return path
    return (REPO_ROOT / path).resolve()


def _slug_fragment(value: str) -> str:
    fragment = re.sub(r"[^a-zA-Z0-9._-]+", "-", value.strip().lower()).strip("-")
    return fragment or "case"


def _resolve_single_run_timeout_seconds(raw_timeout: int) -> int:
    timeout_seconds = int(raw_timeout)
    if not SINGLE_RUN_TIMEOUT_MIN_SECONDS <= timeout_seconds <= SINGLE_RUN_TIMEOUT_MAX_SECONDS:
        raise ValueError(
            "single-run max-runtime-seconds must be between "
            f"{SINGLE_RUN_TIMEOUT_MIN_SECONDS} and {SINGLE_RUN_TIMEOUT_MAX_SECONDS} seconds "
            "(15-20 minutes)"
        )
    return timeout_seconds


def _single_run_timeout_default() -> int:
    raw_value = os.environ.get(SINGLE_RUN_TIMEOUT_ENV)
    if raw_value is None:
        return SINGLE_RUN_TIMEOUT_DEFAULT_SECONDS
    try:
        parsed = int(raw_value)
    except ValueError as exc:
        raise ValueError(
            f"{SINGLE_RUN_TIMEOUT_ENV} must be an integer number of seconds "
            f"between {SINGLE_RUN_TIMEOUT_MIN_SECONDS} and {SINGLE_RUN_TIMEOUT_MAX_SECONDS}"
        ) from exc
    return _resolve_single_run_timeout_seconds(parsed)


def _load_matrix_cases(path: Path) -> list[dict[str, object]]:
    if not path.exists():
        raise FileNotFoundError(f"matrix file not found: {path}")
    payload = json.loads(path.read_text(encoding="utf-8-sig"))
    if isinstance(payload, dict):
        raw_cases = payload.get("cases")
        if raw_cases is None:
            raw_cases = payload.get("scenarios")
    elif isinstance(payload, list):
        raw_cases = payload
    else:
        raw_cases = None
    if not isinstance(raw_cases, list) or not raw_cases:
        raise ValueError("matrix file must contain a non-empty 'cases' array")

    cases: list[dict[str, object]] = []
    for index, raw_case in enumerate(raw_cases, start=1):
        if not isinstance(raw_case, dict):
            raise ValueError(f"matrix case #{index} must be an object")
        dataset = raw_case.get("dataset")
        split = raw_case.get("split")
        if not isinstance(dataset, str) or not dataset.strip():
            raise ValueError(f"matrix case #{index} is missing required string field 'dataset'")
        if not isinstance(split, str) or not split.strip():
            raise ValueError(f"matrix case #{index} is missing required string field 'split'")
        normalized: dict[str, object] = {
            "dataset": dataset.strip(),
            "split": split.strip(),
            "id": str(
                raw_case.get("id")
                or f"{index:02d}-{_slug_fragment(dataset)}-{_slug_fragment(split)}"
            ),
        }
        for key in ("mode", "category", "query_id", "run_name", "description"):
            value = raw_case.get(key)
            if value is not None:
                normalized[key] = str(value)
        for key in ("query_limit", "doc_limit", "recall_budget"):
            value = raw_case.get(key)
            if value is None:
                continue
            if not isinstance(value, int):
                raise ValueError(f"matrix case #{index} field '{key}' must be an integer")
            normalized[key] = value
        if "oracle" in raw_case:
            normalized["oracle"] = bool(raw_case.get("oracle"))
        cases.append(normalized)
    return cases


def _build_matrix_run_args(args: argparse.Namespace, case: dict[str, object]) -> argparse.Namespace:
    run_name = case.get("run_name")
    if run_name is None and args.run_name_prefix:
        run_name = f"{args.run_name_prefix}-{case['id']}"
    return argparse.Namespace(
        dataset=str(case["dataset"]),
        split=str(case["split"]),
        mode=str(case.get("mode", args.mode)),
        category=case.get("category", args.category),
        query_limit=case.get("query_limit", args.query_limit),
        query_id=case.get("query_id", args.query_id),
        doc_limit=case.get("doc_limit", args.doc_limit),
        oracle=bool(case.get("oracle", args.oracle)),
        run_name=run_name,
        description=case.get("description", args.description),
        token_gate_mode=args.token_gate_mode,
        provider_profile=args.provider_profile,
        baseline_file=args.baseline_file,
        disable_baseline_gates=args.disable_baseline_gates,
        no_auto_tighten_baseline=args.no_auto_tighten_baseline,
        min_queries_for_baseline_update=args.min_queries_for_baseline_update,
        baseline_token_headroom_pct=args.baseline_token_headroom_pct,
        baseline_accuracy_headroom=args.baseline_accuracy_headroom,
        recall_budget=int(case.get("recall_budget", args.recall_budget)),
        min_accuracy=args.min_accuracy,
        max_recall_tokens=args.max_recall_tokens,
        max_avg_recall_tokens=args.max_avg_recall_tokens,
        allow_missing_recall_metrics=args.allow_missing_recall_metrics,
        no_enforce_gate=args.no_enforce_gate,
    )


def _load_baseline_store(path: Path) -> dict:
    if not path.exists():
        return {
            "version": 1,
            "updated_at": None,
            "profiles": {},
        }
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError(f"baseline file must contain a JSON object: {path}")
    payload.setdefault("version", 1)
    payload.setdefault("updated_at", None)
    payload.setdefault("profiles", {})
    return payload


def _save_baseline_store(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload["updated_at"] = datetime.now().isoformat()
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")


def _scenario_key(args: argparse.Namespace) -> str:
    category = args.category if args.category else "*"
    return f"{args.dataset}::{args.split}::{args.mode}::{category}"


def _get_baseline_entry(store: dict, provider_profile: str, scenario_key: str) -> dict | None:
    profiles = store.get("profiles", {})
    profile_entries = profiles.get(provider_profile, {})
    entry = profile_entries.get(scenario_key)
    if isinstance(entry, dict):
        return entry
    return None


def _derive_effective_constraints(
    *,
    args: argparse.Namespace,
    token_limits: dict[str, object],
    baseline_entry: dict | None,
) -> dict[str, object]:
    min_accuracy = float(args.min_accuracy)
    max_tokens = token_limits.get("max_recall_tokens")
    avg_tokens = token_limits.get("max_avg_recall_tokens")
    baseline_applied = False
    if baseline_entry is not None and not args.disable_baseline_gates:
        baseline_applied = True
        baseline_min_accuracy = baseline_entry.get("min_accuracy")
        if baseline_min_accuracy is not None:
            min_accuracy = max(min_accuracy, float(baseline_min_accuracy))
        baseline_max_tokens = baseline_entry.get("max_recall_tokens")
        if max_tokens is not None and baseline_max_tokens is not None:
            max_tokens = min(float(max_tokens), float(baseline_max_tokens))
        baseline_avg_tokens = baseline_entry.get("max_avg_recall_tokens")
        if avg_tokens is not None and baseline_avg_tokens is not None:
            avg_tokens = min(float(avg_tokens), float(baseline_avg_tokens))
    return {
        "baseline_applied": baseline_applied,
        "min_accuracy": round(min_accuracy, 4),
        "max_recall_tokens": None if max_tokens is None else int(round(float(max_tokens))),
        "max_avg_recall_tokens": None if avg_tokens is None else round(float(avg_tokens), 2),
    }


def _tighten_baseline_entry(
    *,
    store: dict,
    provider_profile: str,
    scenario_key: str,
    accuracy: float,
    recall_stats: dict[str, float | int],
    args: argparse.Namespace,
) -> tuple[dict, bool]:
    profiles = store.setdefault("profiles", {})
    profile_entries = profiles.setdefault(provider_profile, {})
    current = profile_entries.get(scenario_key)
    if not isinstance(current, dict):
        current = {}
    max_tokens_observed = int(recall_stats.get("max_recall_tokens", 0))
    avg_tokens_observed = float(recall_stats.get("avg_recall_tokens", 0.0))
    candidate_min_accuracy = max(0.0, accuracy - args.baseline_accuracy_headroom)
    candidate_max_tokens = max(
        1,
        int(round(max_tokens_observed * (1.0 + args.baseline_token_headroom_pct))),
    )
    candidate_avg_tokens = max(
        1.0,
        round(avg_tokens_observed * (1.0 + args.baseline_token_headroom_pct), 2),
    )
    current_min_accuracy = float(current.get("min_accuracy", 0.0))
    current_max_tokens = current.get("max_recall_tokens")
    current_avg_tokens = current.get("max_avg_recall_tokens")
    new_entry = {
        "min_accuracy": round(max(current_min_accuracy, candidate_min_accuracy), 4),
        "max_recall_tokens": (
            candidate_max_tokens
            if current_max_tokens is None
            else int(min(int(current_max_tokens), candidate_max_tokens))
        ),
        "max_avg_recall_tokens": (
            candidate_avg_tokens
            if current_avg_tokens is None
            else round(min(float(current_avg_tokens), candidate_avg_tokens), 2)
        ),
        "runs": int(current.get("runs", 0)) + 1,
        "last_accuracy": round(accuracy, 4),
        "last_max_recall_tokens": max_tokens_observed,
        "last_avg_recall_tokens": round(avg_tokens_observed, 2),
        "updated_at": datetime.now().isoformat(),
    }
    changed = current != new_entry
    profile_entries[scenario_key] = new_entry
    return new_entry, changed


def _write_run_manifest(run_dir: Path, payload: dict) -> None:
    (run_dir / "run-manifest.json").write_text(
        json.dumps(payload, indent=2),
        encoding="utf-8",
    )


def _read_json_if_exists(path: Path) -> dict[str, object] | None:
    if not path.exists():
        return None
    payload = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(payload, dict):
        return payload
    return None


def _collect_matrix_case_result(
    *,
    case: dict[str, object],
    run_dir: Path,
    exit_code: int,
    error: str | None,
) -> dict[str, object]:
    summary = _read_json_if_exists(run_dir / "summary.json") or {}
    gate = _read_json_if_exists(run_dir / "gate-report.json") or {}
    recall_stats = gate.get("recall_stats")
    if not isinstance(recall_stats, dict):
        recall_stats = {}
    quality_gate = gate.get("quality_gate")
    if not isinstance(quality_gate, dict):
        quality_gate = {}
    result: dict[str, object] = {
        "id": case.get("id"),
        "dataset": case.get("dataset"),
        "split": case.get("split"),
        "exit": exit_code,
        "run_dir": str(run_dir),
        "accuracy": summary.get("accuracy"),
        "correct": summary.get("correct"),
        "total": summary.get("total_queries"),
        "avg_tokens": recall_stats.get("avg_recall_tokens"),
        "max_tokens": recall_stats.get("max_recall_tokens"),
        "over_budget": recall_stats.get("over_budget_count"),
        "quality_gate_passed": quality_gate.get("passed"),
    }
    failures = quality_gate.get("failures")
    if failures is not None:
        result["quality_gate_failures"] = failures
    if error:
        result["error"] = error
    return result


def _run_benchmark_case_worker(run_args: argparse.Namespace, run_dir: str) -> None:
    run_benchmark(run_args, Path(run_dir))


def _execute_benchmark_with_timeout(
    *,
    run_args: argparse.Namespace,
    run_dir: Path,
    timeout_seconds: int,
    timeout_label: str,
) -> tuple[int, str | None]:
    timeout_cap = max(0, int(timeout_seconds))
    if timeout_cap == 0:
        try:
            run_benchmark(run_args, run_dir)
            return 0, None
        except Exception as exc:
            return 1, str(exc)

    process = multiprocessing.Process(
        target=_run_benchmark_case_worker,
        args=(run_args, str(run_dir)),
        daemon=False,
    )
    process.start()
    process.join(timeout=timeout_cap)
    if process.is_alive():
        process.terminate()
        process.join(timeout=5)
        if process.is_alive():
            process.kill()
            process.join(timeout=5)
        return 124, f"{timeout_label} runtime budget exceeded ({timeout_cap}s cap)"
    exit_code = int(process.exitcode or 0)
    if exit_code == 0:
        return 0, None
    return exit_code, f"{timeout_label} exited with code {exit_code}"


def _execute_matrix_case(
    *,
    run_args: argparse.Namespace,
    run_dir: Path,
    timeout_seconds: int,
) -> tuple[int, str | None]:
    return _execute_benchmark_with_timeout(
        run_args=run_args,
        run_dir=run_dir,
        timeout_seconds=timeout_seconds,
        timeout_label="case",
    )


def _execute_single_run(args: argparse.Namespace, run_dir: Path) -> None:
    timeout_seconds = _resolve_single_run_timeout_seconds(args.max_runtime_seconds)
    exit_code, error = _execute_benchmark_with_timeout(
        run_args=args,
        run_dir=run_dir,
        timeout_seconds=timeout_seconds,
        timeout_label="single run",
    )
    if exit_code != 0:
        raise RuntimeError(error or f"single run failed with exit code {exit_code}")


def _load_recall_metrics(path: Path) -> list[dict]:
    metrics: list[dict] = []
    if not path.exists():
        return metrics
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line:
            continue
        metrics.append(json.loads(line))
    return metrics


def _summarize_recall_metrics(metrics: list[dict], budget: int) -> dict[str, float | int]:
    if not metrics:
        return {
            "queries": 0,
            "avg_recall_tokens": 0.0,
            "max_recall_tokens": 0,
            "over_budget_count": 0,
            "budget": budget,
        }
    token_values = [int(item.get("token_estimate", 0)) for item in metrics]
    over_budget = [value for value in token_values if value > budget]
    return {
        "queries": len(metrics),
        "avg_recall_tokens": round(sum(token_values) / len(token_values), 2),
        "max_recall_tokens": max(token_values),
        "over_budget_count": len(over_budget),
        "budget": budget,
    }


def _enforce_quality_gate(
    *,
    accuracy: float,
    recall_stats: dict[str, float | int],
    args: argparse.Namespace,
    token_limits: dict[str, object],
    effective_constraints: dict[str, object],
) -> dict[str, object]:
    failures: list[str] = []
    min_accuracy = float(effective_constraints["min_accuracy"])
    if accuracy < min_accuracy:
        failures.append(
            f"accuracy {accuracy:.4f} is below required floor {min_accuracy:.4f}"
        )
    gate_mode = str(token_limits.get("mode", "dynamic"))
    query_count = int(recall_stats.get("queries", 0))
    if gate_mode != "off" and query_count == 0 and not args.allow_missing_recall_metrics:
        failures.append(
            "no recall token metrics were captured; this run is invalid for token gating"
        )
    max_tokens = float(recall_stats.get("max_recall_tokens", 0))
    avg_tokens = float(recall_stats.get("avg_recall_tokens", 0.0))
    over_budget = int(recall_stats.get("over_budget_count", 0))
    max_limit = effective_constraints.get("max_recall_tokens")
    avg_limit = effective_constraints.get("max_avg_recall_tokens")
    if gate_mode != "off" and query_count > 0:
        if max_limit is not None and max_tokens > float(max_limit):
            failures.append(
                f"max recall tokens {max_tokens:.0f} exceeded limit {float(max_limit):.0f}"
            )
        if avg_limit is not None and avg_tokens > float(avg_limit):
            failures.append(
                f"avg recall tokens {avg_tokens:.2f} exceeded limit {float(avg_limit):.2f}"
            )
        if over_budget > 0:
            failures.append(
                f"{over_budget} recall queries exceeded configured recall budget {args.recall_budget}"
            )
    return {
        "passed": not failures,
        "failures": failures,
    }


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
        attach_existing = _env_flag_enabled("CORTEX_BENCHMARK_ATTACH_EXISTING_DAEMON", default=True)
        require_app_daemon = _env_flag_enabled("CORTEX_BENCHMARK_REQUIRE_APP_DAEMON", default=True)
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
        stdout = self.stdout_path.open("w", encoding="utf-8")
        stderr = self.stderr_path.open("w", encoding="utf-8")
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
            stdout=stdout,
            stderr=stderr,
            text=True,
            env=proc_env,
        )
        try:
            self._wait_for_health()
            self.token = self._wait_for_token()
        except RuntimeError:
            if attach_existing and self._lock_conflict_detected() and self._try_attach_existing_daemon():
                return self
            raise
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        if self.proc is not None and not self.attached_existing and self.proc.poll() is None:
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
    source_agent = f"amb-cortex::{namespace}"
    cleanup_enabled = _env_flag_enabled("CORTEX_BENCHMARK_CLEANUP_ON_EXIT", default=True)
    with IsolatedCortexDaemon(run_dir) as daemon:
        os.environ.update(daemon.export_env(namespace))
        try:
            _write_run_manifest(
                run_dir,
                {
                    "command": "smoke",
                    "created_at": datetime.now().isoformat(),
                    "cortex_repo_head": _git_head_short(REPO_ROOT),
                    "cortex_binary": str(daemon.binary),
                    "daemon_mode": daemon.daemon_mode,
                    "benchmark_tools": _load_lock_summary(),
                    "namespace": namespace,
                    "legitimacy": {
                        "isolated_daemon": not daemon.attached_existing,
                        "uses_live_app_daemon": daemon.attached_existing,
                        "oracle_mode": False,
                        "notes": "Smoke test validates Cortex ingest/retrieve only. It does not run AMB judging.",
                    },
                },
            )
            client = CortexHTTPClient()
            try:
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
                print(json.dumps(payload, indent=2))
            finally:
                client.close()
        finally:
            if daemon.attached_existing and cleanup_enabled:
                cleanup_report = _cleanup_benchmark_namespace(
                    base_url=daemon.base_url,
                    source_agent=source_agent,
                )
                (run_dir / "namespace-cleanup.json").write_text(
                    json.dumps(cleanup_report, indent=2),
                    encoding="utf-8",
                )


def run_benchmark(args: argparse.Namespace, run_dir: Path) -> None:
    _assert_amb_environment()
    _register_provider()
    from memory_bench.dataset import get_dataset
    from memory_bench.llm import get_answer_llm
    from memory_bench.modes import get_mode
    from memory_bench.runner import EvalRunner
    from memory_bench.memory import get_memory_provider

    namespace = args.run_name or f"{args.dataset}-{args.split}-{run_dir.name}"
    source_agent = f"amb-cortex::{namespace}"
    cleanup_enabled = _env_flag_enabled("CORTEX_BENCHMARK_CLEANUP_ON_EXIT", default=True)
    recall_metrics_path = run_dir / "retrieval-metrics.jsonl"
    baseline_path = _resolve_baseline_path(args.baseline_file)
    with IsolatedCortexDaemon(run_dir) as daemon:
        try:
            os.environ.update(daemon.export_env(namespace))
            os.environ["CORTEX_RECALL_BUDGET"] = str(args.recall_budget)
            os.environ["CORTEX_BENCHMARK_METRICS_FILE"] = str(recall_metrics_path)
            llm_provider = _configure_llm_environment()
            provider_profile = _normalize_provider_profile(
                args.provider_profile if args.provider_profile != "auto" else llm_provider
            )
            token_limits = _resolve_token_gate_limits(
                mode=args.token_gate_mode,
                recall_budget=args.recall_budget,
                provider_profile=provider_profile,
                max_recall_tokens=args.max_recall_tokens,
                max_avg_recall_tokens=args.max_avg_recall_tokens,
            )
            scenario_key = _scenario_key(args)
            baseline_store = _load_baseline_store(baseline_path)
            baseline_entry = _get_baseline_entry(
                baseline_store,
                provider_profile=provider_profile,
                scenario_key=scenario_key,
            )
            effective_constraints = _derive_effective_constraints(
                args=args,
                token_limits=token_limits,
                baseline_entry=baseline_entry,
            )
            _write_run_manifest(
                run_dir,
                {
                    "command": "run",
                    "created_at": datetime.now().isoformat(),
                    "cortex_repo_head": _git_head_short(REPO_ROOT),
                    "cortex_binary": str(daemon.binary),
                    "daemon_mode": daemon.daemon_mode,
                    "benchmark_tools": _load_lock_summary(),
                    "dataset": args.dataset,
                    "split": args.split,
                    "mode": args.mode,
                    "category": args.category,
                    "query_limit": args.query_limit,
                    "query_id": args.query_id,
                    "doc_limit": args.doc_limit,
                    "max_runtime_seconds": getattr(args, "max_runtime_seconds", None),
                    "namespace": namespace,
                    "llm_provider": llm_provider,
                    "baseline": {
                        "file": str(baseline_path),
                        "scenario_key": scenario_key,
                        "baseline_applied": effective_constraints["baseline_applied"],
                        "entry": baseline_entry,
                    },
                    "quality_gate": {
                        "enabled": not args.no_enforce_gate,
                        "token_gate_mode": args.token_gate_mode,
                        "provider_profile": provider_profile,
                        "min_accuracy": effective_constraints["min_accuracy"],
                        "recall_budget": args.recall_budget,
                        "max_recall_tokens": effective_constraints["max_recall_tokens"],
                        "max_avg_recall_tokens": effective_constraints["max_avg_recall_tokens"],
                        "allow_missing_recall_metrics": args.allow_missing_recall_metrics,
                    },
                    "legitimacy": {
                        "isolated_daemon": not daemon.attached_existing,
                        "uses_live_app_daemon": daemon.attached_existing,
                        "oracle_mode": bool(args.oracle),
                        "notes": (
                            "Normal benchmark runs should keep oracle_mode=false. "
                            "If oracle_mode=true, treat the run as a diagnostic ceiling, not a headline score."
                        ),
                    },
                },
            )

            dataset = _apply_dataset_compat_shims(get_dataset(args.dataset))
            mode = get_mode(args.mode, llm=get_answer_llm())
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
            recall_metrics = _load_recall_metrics(recall_metrics_path)
            recall_stats = _summarize_recall_metrics(recall_metrics, args.recall_budget)
            gate = _enforce_quality_gate(
                accuracy=float(summary.accuracy),
                recall_stats=recall_stats,
                args=args,
                token_limits=token_limits,
                effective_constraints=effective_constraints,
            )
            baseline_update: dict | None = None
            baseline_updated = False
            can_tighten = (
                not args.no_auto_tighten_baseline
                and not args.no_enforce_gate
                and gate["passed"]
                and args.token_gate_mode != "off"
                and args.query_limit is None
                and args.query_id is None
                and int(recall_stats.get("queries", 0)) >= args.min_queries_for_baseline_update
            )
            if can_tighten:
                baseline_update, baseline_updated = _tighten_baseline_entry(
                    store=baseline_store,
                    provider_profile=provider_profile,
                    scenario_key=scenario_key,
                    accuracy=float(summary.accuracy),
                    recall_stats=recall_stats,
                    args=args,
                )
                if baseline_updated:
                    _save_baseline_store(baseline_path, baseline_store)
            gate_payload = {
                "timestamp": datetime.now().isoformat(),
                "quality_gate": gate,
                "accuracy": float(summary.accuracy),
                "recall_stats": recall_stats,
                "limits": {
                    "token_gate_mode": args.token_gate_mode,
                    "provider_profile": provider_profile,
                    "min_accuracy": effective_constraints["min_accuracy"],
                    "recall_budget": args.recall_budget,
                    "max_recall_tokens": effective_constraints["max_recall_tokens"],
                    "max_avg_recall_tokens": effective_constraints["max_avg_recall_tokens"],
                    "token_gate_profile": token_limits.get("profile"),
                    "allow_missing_recall_metrics": args.allow_missing_recall_metrics,
                },
                "baseline": {
                    "file": str(baseline_path),
                    "scenario_key": scenario_key,
                    "baseline_applied": effective_constraints["baseline_applied"],
                    "entry": baseline_entry,
                    "auto_tighten_enabled": not args.no_auto_tighten_baseline,
                    "min_queries_for_update": args.min_queries_for_baseline_update,
                    "updated": baseline_updated,
                    "updated_entry": baseline_update,
                },
            }
            (run_dir / "gate-report.json").write_text(
                json.dumps(gate_payload, indent=2),
                encoding="utf-8",
            )
            print(
                json.dumps(
                    {
                        "dataset": summary.dataset,
                        "split": summary.split,
                        "memory_provider": summary.memory_provider,
                        "mode": summary.mode,
                        "accuracy": summary.accuracy,
                        "total_queries": summary.total_queries,
                        "recall_stats": recall_stats,
                        "token_gate_mode": args.token_gate_mode,
                        "provider_profile": provider_profile,
                        "token_limits": {
                            "max_recall_tokens": effective_constraints["max_recall_tokens"],
                            "max_avg_recall_tokens": effective_constraints[
                                "max_avg_recall_tokens"
                            ],
                        },
                        "baseline": {
                            "scenario_key": scenario_key,
                            "baseline_applied": effective_constraints["baseline_applied"],
                            "baseline_updated": baseline_updated,
                        },
                        "quality_gate_passed": gate["passed"],
                        "run_dir": str(run_dir),
                        "output_json": str(
                            (
                                run_dir
                                / "outputs"
                                / summary.dataset
                                / summary.run_name
                                / summary.mode
                                / f"{summary.split}.json"
                            )
                        ),
                    },
                    indent=2,
                )
            )
            if not args.no_enforce_gate and not gate["passed"]:
                lines = "\n".join(f"- {failure}" for failure in gate["failures"])
                raise RuntimeError(f"quality gate failed:\n{lines}")
        finally:
            if daemon.attached_existing and cleanup_enabled:
                cleanup_report = _cleanup_benchmark_namespace(
                    base_url=daemon.base_url,
                    source_agent=source_agent,
                )
                (run_dir / "namespace-cleanup.json").write_text(
                    json.dumps(cleanup_report, indent=2),
                    encoding="utf-8",
                )


def run_matrix(args: argparse.Namespace, run_dir: Path) -> None:
    matrix_path = _resolve_matrix_path(args.matrix_file)
    all_cases = _load_matrix_cases(matrix_path)
    start_index = max(1, int(args.start_index))
    if start_index > len(all_cases):
        raise ValueError(
            f"start_index {start_index} exceeds matrix case count {len(all_cases)}"
        )
    cases = all_cases[start_index - 1 :]
    if args.max_cases is not None:
        cases = cases[: max(1, int(args.max_cases))]
    max_runtime_seconds = max(0, int(args.max_runtime_seconds))
    max_case_runtime_seconds = max(0, int(args.max_case_runtime_seconds))
    summary_path = (
        _resolve_matrix_path(args.summary_file)
        if args.summary_file
        else run_dir / "matrix-summary.json"
    )
    _write_run_manifest(
        run_dir,
        {
            "command": "matrix",
            "created_at": datetime.now().isoformat(),
            "cortex_repo_head": _git_head_short(REPO_ROOT),
            "benchmark_tools": _load_lock_summary(),
            "matrix_file": str(matrix_path),
            "summary_file": str(summary_path),
            "dry_run": bool(args.dry_run),
            "continue_on_error": bool(args.continue_on_error),
            "case_count_total": len(all_cases),
            "case_count_selected": len(cases),
            "start_index": start_index,
            "max_cases": args.max_cases,
            "max_runtime_seconds": max_runtime_seconds,
            "max_case_runtime_seconds": max_case_runtime_seconds,
            "defaults": {
                "mode": args.mode,
                "category": args.category,
                "query_limit": args.query_limit,
                "query_id": args.query_id,
                "doc_limit": args.doc_limit,
                "oracle": bool(args.oracle),
                "recall_budget": args.recall_budget,
                "token_gate_mode": args.token_gate_mode,
                "provider_profile": args.provider_profile,
                "baseline_file": args.baseline_file,
            },
        },
    )
    if args.dry_run:
        preview = [
            {
                "id": case["id"],
                "dataset": case["dataset"],
                "split": case["split"],
                "mode": case.get("mode", args.mode),
                "query_limit": case.get("query_limit", args.query_limit),
            }
            for case in cases
        ]
        summary_path.parent.mkdir(parents=True, exist_ok=True)
        summary_path.write_text(json.dumps(preview, indent=2), encoding="utf-8")
        print(
            json.dumps(
                {
                    "dry_run": True,
                    "matrix_file": str(matrix_path),
                    "summary_file": str(summary_path),
                    "cases": preview,
                },
                indent=2,
            )
        )
        return

    results: list[dict[str, object]] = []
    failed_cases = 0
    started_at = time.monotonic()
    for case_offset, case in enumerate(cases):
        elapsed_seconds = time.monotonic() - started_at
        if max_runtime_seconds > 0 and elapsed_seconds >= max_runtime_seconds:
            for skipped_case in cases[case_offset:]:
                results.append(
                    {
                        "id": skipped_case["id"],
                        "dataset": skipped_case["dataset"],
                        "split": skipped_case["split"],
                        "exit": 124,
                        "run_dir": str(run_dir / f"skipped-{_slug_fragment(str(skipped_case['id']))}"),
                        "accuracy": None,
                        "correct": None,
                        "total": None,
                        "avg_tokens": None,
                        "max_tokens": None,
                        "over_budget": None,
                        "quality_gate_passed": None,
                        "error": (
                            "matrix runtime budget exceeded before case start "
                            f"({elapsed_seconds:.1f}s elapsed, cap={max_runtime_seconds}s)"
                        ),
                    }
                )
                failed_cases += 1
            break
        index = start_index + case_offset
        case_slug = _slug_fragment(str(case["id"]))
        case_run_dir = run_dir / f"{index:02d}-{case_slug}"
        case_run_dir.mkdir(parents=True, exist_ok=True)
        run_args = _build_matrix_run_args(args, case)
        exit_code, error_message = _execute_matrix_case(
            run_args=run_args,
            run_dir=case_run_dir,
            timeout_seconds=max_case_runtime_seconds,
        )
        if exit_code != 0:
            failed_cases += 1
        result = _collect_matrix_case_result(
            case=case,
            run_dir=case_run_dir,
            exit_code=exit_code,
            error=error_message,
        )
        results.append(result)
        if exit_code != 0 and not args.continue_on_error:
            break

    summary_path.parent.mkdir(parents=True, exist_ok=True)
    summary_path.write_text(json.dumps(results, indent=2), encoding="utf-8")
    passed_cases = sum(1 for result in results if int(result.get("exit", 1)) == 0)
    print(
        json.dumps(
            {
                "matrix_file": str(matrix_path),
                "summary_file": str(summary_path),
                "cases_total": len(results),
                "cases_selected": len(cases),
                "cases_passed": passed_cases,
                "cases_failed": len(results) - passed_cases,
                "run_dir": str(run_dir),
            },
            indent=2,
        )
    )
    if failed_cases > 0:
        raise RuntimeError(
            f"matrix run failed for {failed_cases} case(s); see {summary_path} for details"
        )


def _add_quality_gate_arguments(target: argparse.ArgumentParser) -> None:
    target.add_argument(
        "--token-gate-mode",
        choices=["dynamic", "absolute", "off"],
        default="dynamic",
        help="Token gate strategy: dynamic (provider-aware), absolute (fixed limits), off (accuracy-only).",
    )
    target.add_argument(
        "--provider-profile",
        default="auto",
        help="Provider profile for dynamic token gates (auto, claude, openai, codex, gemini, groq, default).",
    )
    target.add_argument(
        "--baseline-file",
        default=str(BASELINE_FILE_DEFAULT),
        help="Path to provider/scenario baseline JSON used for non-regression gates and auto-tightening.",
    )
    target.add_argument(
        "--disable-baseline-gates",
        action="store_true",
        help="Ignore saved baseline entries when computing effective gates (diagnostics only).",
    )
    target.add_argument(
        "--no-auto-tighten-baseline",
        action="store_true",
        help="Do not tighten baseline thresholds after passing runs.",
    )
    target.add_argument(
        "--min-queries-for-baseline-update",
        type=int,
        default=20,
        help="Minimum query count required before a run can tighten baseline thresholds.",
    )
    target.add_argument(
        "--baseline-token-headroom-pct",
        type=float,
        default=0.08,
        help="Headroom added above observed token usage when tightening baseline ceilings.",
    )
    target.add_argument(
        "--baseline-accuracy-headroom",
        type=float,
        default=0.02,
        help="Margin subtracted from observed accuracy when tightening baseline floor.",
    )
    target.add_argument(
        "--recall-budget",
        type=int,
        default=300,
        help="Recall token budget sent to Cortex for each retrieval query.",
    )
    target.add_argument(
        "--min-accuracy",
        type=float,
        default=0.90,
        help="Minimum acceptable benchmark accuracy.",
    )
    target.add_argument(
        "--max-recall-tokens",
        type=int,
        default=300,
        help="Maximum allowed recall tokens for any single query.",
    )
    target.add_argument(
        "--max-avg-recall-tokens",
        type=float,
        default=300.0,
        help="Maximum allowed average recall tokens across benchmark queries.",
    )
    target.add_argument(
        "--allow-missing-recall-metrics",
        action="store_true",
        help="Permit runs with missing recall token telemetry (not recommended).",
    )
    target.add_argument(
        "--no-enforce-gate",
        action="store_true",
        help="Skip failing the run when quality gates are violated (diagnostics only).",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run AMB against an isolated Cortex benchmark daemon.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    smoke = subparsers.add_parser("smoke", help="Run a retrieval-only smoke test against an isolated Cortex daemon.")
    smoke.set_defaults(func=run_smoke)

    run = subparsers.add_parser("run", help="Run one AMB benchmark scenario against an isolated Cortex daemon.")
    run.add_argument("--dataset", required=True, help="AMB dataset name, e.g. longmemeval, locomo, membench.")
    run.add_argument("--split", required=True, help="AMB split/domain name for the dataset.")
    run.add_argument("--mode", default="rag", help="AMB response mode. Defaults to rag.")
    run.add_argument("--category", default=None, help="Optional AMB category filter.")
    run.add_argument("--query-limit", type=int, default=None, help="Optional query limit for smaller runs.")
    run.add_argument("--query-id", default=None, help="Optional single query id.")
    run.add_argument("--doc-limit", type=int, default=None, help="Optional document limit.")
    run.add_argument("--oracle", action="store_true", help="Use oracle mode when the dataset supports it.")
    run.add_argument(
        "--max-runtime-seconds",
        type=int,
        default=_single_run_timeout_default(),
        help=(
            "Hard runtime cap for a single run (seconds). "
            "Must be between 900 and 1200 (15-20 minutes)."
        ),
    )
    run.add_argument("--run-name", default=None, help="Optional AMB run name. Defaults to cortex-http.")
    run.add_argument("--description", default=None, help="Optional run description written into the AMB output.")
    _add_quality_gate_arguments(run)
    run.set_defaults(func=run_benchmark)

    matrix = subparsers.add_parser(
        "matrix",
        help="Run a multi-dataset AMB evaluation matrix against isolated Cortex daemons.",
    )
    matrix.add_argument(
        "--matrix-file",
        default=str(MATRIX_FILE_DEFAULT),
        help="Path to JSON matrix spec with cases/scenarios.",
    )
    matrix.add_argument(
        "--summary-file",
        default=None,
        help="Optional output path for matrix summary JSON (defaults to run_dir/matrix-summary.json).",
    )
    matrix.add_argument(
        "--start-index",
        type=int,
        default=1,
        help="1-based case index to start from within the matrix file.",
    )
    matrix.add_argument(
        "--max-cases",
        type=int,
        default=None,
        help="Optional max number of cases to execute from start-index.",
    )
    matrix.add_argument(
        "--max-runtime-seconds",
        type=int,
        default=int(os.environ.get("CORTEX_BENCHMARK_MATRIX_MAX_RUNTIME_SECONDS", "1200")),
        help="Hard runtime cap for a matrix invocation (defaults to 1200 seconds / 20 minutes).",
    )
    matrix.add_argument(
        "--max-case-runtime-seconds",
        type=int,
        default=int(os.environ.get("CORTEX_BENCHMARK_MATRIX_MAX_CASE_RUNTIME_SECONDS", "900")),
        help="Hard runtime cap per matrix case (defaults to 900 seconds / 15 minutes; set 0 to disable).",
    )
    matrix.add_argument(
        "--run-name-prefix",
        default="matrix",
        help="Prefix used for per-case run names when a case does not define run_name.",
    )
    matrix.add_argument("--mode", default="rag", help="Default AMB response mode for cases missing mode.")
    matrix.add_argument("--category", default=None, help="Default AMB category for cases missing category.")
    matrix.add_argument("--query-limit", type=int, default=None, help="Default query limit for cases missing query_limit.")
    matrix.add_argument("--query-id", default=None, help="Default query id for cases missing query_id.")
    matrix.add_argument("--doc-limit", type=int, default=None, help="Default doc limit for cases missing doc_limit.")
    matrix.add_argument("--oracle", action="store_true", help="Default oracle mode for cases missing oracle.")
    matrix.add_argument("--description", default=None, help="Default run description for cases missing description.")
    matrix.add_argument(
        "--continue-on-error",
        action="store_true",
        help="Continue remaining matrix cases after an individual case fails.",
    )
    matrix.add_argument(
        "--dry-run",
        action="store_true",
        help="Validate and expand matrix cases without executing AMB runs.",
    )
    _add_quality_gate_arguments(matrix)
    matrix.set_defaults(func=run_matrix)

    return parser


def main() -> None:
    _ensure_utf8_stdio()
    parser = build_parser()
    args = parser.parse_args()
    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    run_dir = RUNS_ROOT / f"amb-{args.command}-{timestamp}"
    run_dir.mkdir(parents=True, exist_ok=True)
    try:
        if args.command == "smoke":
            args.func(run_dir)
        elif args.command == "run":
            _execute_single_run(args, run_dir)
        else:
            args.func(args, run_dir)
    except Exception as exc:
        print(f"benchmark runner failed: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc


if __name__ == "__main__":
    main()

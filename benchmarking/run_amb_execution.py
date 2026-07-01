"""Benchmark execution, matrix runs, cadence, and quality gates."""
from __future__ import annotations

import argparse
import json
import multiprocessing
import os
import subprocess
import sys
import time
import traceback
from dataclasses import asdict
from datetime import datetime
from pathlib import Path
from typing import cast

import httpx

from run_amb_config import (
    CASE_ERROR_FILENAME,
    DEFAULT_MEMORY_BACKEND,
    FAIR_RUN_PREFLIGHT_FILENAME,
    MATRIX_CASE_TIMEOUT_MAX_SECONDS,
    MATRIX_TIMEOUT_MAX_SECONDS,
    QUALITY_TOKEN_TARGETS,
    REPO_ROOT,
    RUNS_ROOT,
    SUPPORTED_MEMORY_BACKENDS,
)
from run_amb_config import _resolve_quality_token_target
from run_amb_daemon import (
    IsolatedCortexDaemon,
    _assert_amb_environment,
    _register_provider,
    _resolve_memory_backend,
)
from run_amb_preflight import (
    _apply_matrix_execution_profile,
    _build_matrix_preflight,
    _build_matrix_run_args,
    _build_single_run_preflight,
    _derive_effective_constraints,
    _format_preflight_failures,
    _get_baseline_entry,
    _load_baseline_store,
    _load_matrix_cases,
    _load_matrix_spec,
    _resolve_baseline_path,
    _resolve_matrix_dataset_prereq_skips,
    _resolve_matrix_path,
    _resolve_matrix_timeout_seconds,
    _save_baseline_store,
    _scenario_key,
    _tighten_baseline_entry,
    _validate_matrix_fairness,
    _write_fair_run_preflight,
)
from run_amb_runtime import (
    _apply_dataset_compat_shims,
    _apply_retrieval_profile_defaults,
    _build_profile_delta_report,
    _cleanup_benchmark_namespace,
    _configure_imports,
    _configure_llm_environment,
    _context_efficiency_metrics,
    _env_flag_enabled,
    _filter_kwargs_for_callable,
    _git_head_short,
    _load_lock_summary,
    _normalize_provider_profile,
    _prepare_worker_runtime_env,
    _recall_efficiency_metrics,
    _resolve_token_gate_limits,
)

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
    summary_path = run_dir / "summary.json"
    gate_path = run_dir / "gate-report.json"
    worker_error_path = run_dir / CASE_ERROR_FILENAME
    summary = _read_json_if_exists(summary_path) or {}
    gate = _read_json_if_exists(gate_path) or {}
    worker_error = _read_json_if_exists(worker_error_path) or {}
    recall_stats = gate.get("recall_stats")
    if not isinstance(recall_stats, dict):
        recall_stats = {}
    quality_gate = gate.get("quality_gate")
    if not isinstance(quality_gate, dict):
        quality_gate = {}
    recall_efficiency = gate.get("recall_efficiency")
    if not isinstance(recall_efficiency, dict):
        recall_efficiency = {}
    tradeoff = gate.get("tradeoff")
    if not isinstance(tradeoff, dict):
        tradeoff = {}
    profile_delta = tradeoff.get("profile_delta")
    if not isinstance(profile_delta, dict):
        profile_delta = {}
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
        "score_per_1k_recall_tokens": gate.get(
            "score_per_1k_recall_tokens",
            recall_efficiency.get("score_per_1k_recall_tokens"),
        ),
        "quality_token_target": tradeoff.get("quality_token_target"),
        "retrieval_profile_effective": tradeoff.get("effective_retrieval_profile"),
        "quality_gate_passed": quality_gate.get("passed"),
    }
    missing_artifacts: list[str] = []
    if not summary_path.exists():
        missing_artifacts.append(summary_path.name)
    if not gate_path.exists():
        missing_artifacts.append(gate_path.name)
    if missing_artifacts:
        result["missing_artifacts"] = missing_artifacts
    delta_vs_token_gate = profile_delta.get("delta_vs_token_gate")
    if isinstance(delta_vs_token_gate, dict):
        result["profile_delta_vs_token_gate"] = delta_vs_token_gate
    failures = quality_gate.get("failures")
    if failures is not None:
        result["quality_gate_failures"] = failures
    worker_error_message = worker_error.get("error")
    if isinstance(worker_error_message, str) and worker_error_message.strip():
        result["worker_error"] = worker_error_message.strip()
    worker_error_type = worker_error.get("type")
    if isinstance(worker_error_type, str) and worker_error_type.strip():
        result["worker_error_type"] = worker_error_type.strip()
    worker_traceback = worker_error.get("traceback")
    if isinstance(worker_traceback, str) and worker_traceback.strip():
        trace_lines = [line for line in worker_traceback.strip().splitlines() if line.strip()]
        if trace_lines:
            result["worker_traceback_tail"] = "\n".join(trace_lines[-8:])
    if error:
        result["error"] = error
    return result


def _run_benchmark_case_worker(
    run_args: argparse.Namespace,
    run_dir: str,
    error_path: str,
) -> None:
    _prepare_worker_runtime_env()
    try:
        run_benchmark(run_args, Path(run_dir))
    except Exception as exc:
        payload = {
            "type": exc.__class__.__name__,
            "error": str(exc),
            "traceback": traceback.format_exc(),
        }
        try:
            Path(error_path).write_text(
                json.dumps(payload, indent=2),
                encoding="utf-8",
            )
        except Exception:
            pass
        raise


def _execute_benchmark_with_timeout(
    *,
    run_args: argparse.Namespace,
    run_dir: Path,
    timeout_seconds: int,
    timeout_label: str,
) -> tuple[int, str | None]:
    _prepare_worker_runtime_env()
    error_payload_path = run_dir / CASE_ERROR_FILENAME
    if error_payload_path.exists():
        try:
            error_payload_path.unlink()
        except OSError:
            pass
    timeout_cap = max(0, int(timeout_seconds))
    if timeout_cap == 0:
        try:
            run_benchmark(run_args, run_dir)
            return 0, None
        except Exception as exc:
            payload = {
                "type": exc.__class__.__name__,
                "error": str(exc),
                "traceback": traceback.format_exc(),
            }
            try:
                error_payload_path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
            except OSError:
                pass
            return 1, str(exc)

    process = multiprocessing.Process(
        target=_run_benchmark_case_worker,
        args=(run_args, str(run_dir), str(error_payload_path)),
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
        if error_payload_path.exists():
            try:
                error_payload_path.unlink()
            except OSError:
                pass
        return 0, None
    worker_error_payload = _read_json_if_exists(error_payload_path)
    if isinstance(worker_error_payload, dict):
        worker_error_type = worker_error_payload.get("type")
        worker_error_message = worker_error_payload.get("error")
        if isinstance(worker_error_type, str) and isinstance(worker_error_message, str):
            return (
                exit_code,
                (
                    f"{timeout_label} failed with {worker_error_type}: {worker_error_message} "
                    f"(see {error_payload_path.name})"
                ),
            )
        if isinstance(worker_error_message, str):
            return (
                exit_code,
                f"{timeout_label} failed: {worker_error_message} (see {error_payload_path.name})",
            )
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
    preflight, timeout_seconds = _build_single_run_preflight(args)
    _write_fair_run_preflight(run_dir, preflight)
    if not bool(preflight.get("passed", False)):
        raw_violations = preflight.get("violations")
        failures = [str(item) for item in raw_violations] if isinstance(raw_violations, list) else []
        if not failures:
            failures = ["single-run fair-run preflight failed with unknown violation"]
        raise ValueError(_format_preflight_failures("single-run fair-run preflight failed", failures))
    if timeout_seconds is None:
        raise ValueError("single-run fair-run preflight did not produce a valid timeout cap")
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
            "total_recall_tokens": 0,
            "over_budget_count": 0,
            "budget": budget,
            "recall_calls": 0,
            "avg_recall_calls_per_query": 0.0,
            "avg_recall_tokens_per_call": 0.0,
        }
    token_values = [int(item.get("token_estimate", 0)) for item in metrics]
    total_recall_tokens = sum(token_values)
    over_budget = [value for value in token_values if value > budget]
    recall_calls = [max(1, int(item.get("recall_call_count", 1))) for item in metrics]
    total_calls = sum(recall_calls)
    return {
        "queries": len(metrics),
        "avg_recall_tokens": round(total_recall_tokens / len(token_values), 2),
        "max_recall_tokens": max(token_values),
        "total_recall_tokens": total_recall_tokens,
        "over_budget_count": len(over_budget),
        "budget": budget,
        "recall_calls": total_calls,
        "avg_recall_calls_per_query": round(total_calls / len(metrics), 3),
        "avg_recall_tokens_per_call": round(total_recall_tokens / total_calls, 2) if total_calls > 0 else 0.0,
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

    memory_backend = _resolve_memory_backend(args)
    default_run_name = memory_backend
    namespace = args.run_name or f"{memory_backend}-{args.dataset}-{args.split}-{run_dir.name}"
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
            quality_target_plan = _resolve_quality_token_target(
                target=args.quality_token_target,
                retrieval_profile=args.retrieval_profile,
                min_accuracy=float(args.min_accuracy),
            )
            effective_retrieval_profile = str(
                quality_target_plan["effective_retrieval_profile"]
            )
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
            retrieval_profile_env = _apply_retrieval_profile_defaults(
                effective_retrieval_profile
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
                min_accuracy_override=float(
                    quality_target_plan["effective_min_accuracy"]
                ),
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
                    "memory_backend": memory_backend,
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
                        "quality_token_target": quality_target_plan["target"],
                        "retrieval_profile_requested": args.retrieval_profile,
                        "retrieval_profile_effective": effective_retrieval_profile,
                        "provider_profile": provider_profile,
                        "min_accuracy_requested": float(args.min_accuracy),
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
            memory = get_memory_provider(memory_backend)
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
                run_name=args.run_name or default_run_name,
                description=args.description,
            )

            summary_path = run_dir / "summary.json"
            summary_path.write_text(json.dumps(asdict(summary), indent=2), encoding="utf-8")
            recall_metrics = _load_recall_metrics(recall_metrics_path)
            recall_stats = _summarize_recall_metrics(recall_metrics, args.recall_budget)
            efficiency = _context_efficiency_metrics(summary)
            recall_efficiency = _recall_efficiency_metrics(summary, recall_stats)
            profile_delta = _build_profile_delta_report(
                token_limits=token_limits,
                effective_constraints=effective_constraints,
                baseline_entry=baseline_entry,
                recall_stats=recall_stats,
            )
            tradeoff = {
                "quality_token_target": quality_target_plan["target"],
                "quality_token_target_applied": quality_target_plan["applied"],
                "quality_token_target_description": quality_target_plan["description"],
                "requested_retrieval_profile": quality_target_plan["requested_retrieval_profile"],
                "effective_retrieval_profile": quality_target_plan["effective_retrieval_profile"],
                "requested_min_accuracy": quality_target_plan["requested_min_accuracy"],
                "effective_min_accuracy": quality_target_plan["effective_min_accuracy"],
                "score_per_1k_recall_tokens": recall_efficiency.get("score_per_1k_recall_tokens"),
                "profile_delta": profile_delta,
            }
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
                **recall_efficiency,
                **efficiency,
                "efficiency": efficiency,
                "recall_efficiency": recall_efficiency,
                "tradeoff": tradeoff,
                "limits": {
                    "token_gate_mode": args.token_gate_mode,
                    "quality_token_target": quality_target_plan["target"],
                    "retrieval_profile_requested": args.retrieval_profile,
                    "retrieval_profile": effective_retrieval_profile,
                    "provider_profile": provider_profile,
                    "min_accuracy_requested": float(args.min_accuracy),
                    "min_accuracy": effective_constraints["min_accuracy"],
                    "recall_budget": args.recall_budget,
                    "max_recall_tokens": effective_constraints["max_recall_tokens"],
                    "max_avg_recall_tokens": effective_constraints["max_avg_recall_tokens"],
                    "token_gate_profile": token_limits.get("profile"),
                    "allow_missing_recall_metrics": args.allow_missing_recall_metrics,
                },
                "retrieval_profile_env": retrieval_profile_env,
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
                        **recall_efficiency,
                        **efficiency,
                        "efficiency": efficiency,
                        "recall_efficiency": recall_efficiency,
                        "tradeoff": tradeoff,
                        "token_gate_mode": args.token_gate_mode,
                        "quality_token_target": quality_target_plan["target"],
                        "retrieval_profile_requested": args.retrieval_profile,
                        "retrieval_profile": effective_retrieval_profile,
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
    all_cases, execution_profile = _load_matrix_spec(matrix_path)
    requested_shortcuts = {
        "oracle": bool(getattr(args, "oracle", False)),
        "no_enforce_gate": bool(getattr(args, "no_enforce_gate", False)),
        "allow_missing_recall_metrics": bool(getattr(args, "allow_missing_recall_metrics", False)),
    }
    _apply_matrix_execution_profile(args, execution_profile)
    start_index = max(1, int(args.start_index))
    if start_index > len(all_cases):
        raise ValueError(
            f"start_index {start_index} exceeds matrix case count {len(all_cases)}"
        )
    selected_cases = all_cases[start_index - 1 :]
    if args.max_cases is not None:
        selected_cases = selected_cases[: max(1, int(args.max_cases))]
    runnable_cases, skipped_cases = _resolve_matrix_dataset_prereq_skips(selected_cases)
    max_runtime_seconds = _resolve_matrix_timeout_seconds(
        int(args.max_runtime_seconds),
        max_timeout=MATRIX_TIMEOUT_MAX_SECONDS,
        field_name="max-runtime-seconds",
    )
    max_case_runtime_seconds = _resolve_matrix_timeout_seconds(
        int(args.max_case_runtime_seconds),
        max_timeout=MATRIX_CASE_TIMEOUT_MAX_SECONDS,
        field_name="max-case-runtime-seconds",
    )
    default_memory_backend = _resolve_memory_backend(args)
    preflight = _build_matrix_preflight(
        args=args,
        cases=all_cases,
        selected_cases=selected_cases,
        runnable_cases=runnable_cases,
        skipped_cases=skipped_cases,
        start_index=start_index,
        max_runtime_seconds=max_runtime_seconds,
        max_case_runtime_seconds=max_case_runtime_seconds,
        requested_shortcuts=requested_shortcuts,
    )
    _write_fair_run_preflight(run_dir, preflight)
    if not bool(preflight.get("passed", False)):
        raw_violations = preflight.get("violations")
        failures = [str(item) for item in raw_violations] if isinstance(raw_violations, list) else []
        if not failures:
            failures = ["matrix fair-run preflight failed with unknown violation"]
        raise ValueError(_format_preflight_failures("matrix fair-run preflight failed", failures))
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
            "case_count_selected": len(selected_cases),
            "case_count_runnable": len(runnable_cases),
            "case_count_skipped_prereq": len(skipped_cases),
            "start_index": start_index,
            "max_cases": args.max_cases,
            "max_runtime_seconds": max_runtime_seconds,
            "max_case_runtime_seconds": max_case_runtime_seconds,
            "execution_profile": execution_profile,
            "defaults": {
                "mode": args.mode,
                "category": args.category,
                "query_limit": args.query_limit,
                "query_id": args.query_id,
                "doc_limit": args.doc_limit,
                "oracle": bool(args.oracle),
                "recall_budget": args.recall_budget,
                "quality_token_target": args.quality_token_target,
                "retrieval_profile": args.retrieval_profile,
                "memory_backend": default_memory_backend,
                "token_gate_mode": args.token_gate_mode,
                "provider_profile": args.provider_profile,
                "baseline_file": args.baseline_file,
            },
        },
    )
    if args.dry_run:
        skipped_lookup = {
            str(item.get("id", "")): item for item in skipped_cases
        }
        preview = [
            {
                "id": case["id"],
                "dataset": case["dataset"],
                "split": case["split"],
                "mode": case.get("mode", args.mode),
                "memory_backend": str(case.get("memory_backend", default_memory_backend)),
                "query_limit": case.get("query_limit", args.query_limit),
                "status": "skipped-prereq"
                if str(case["id"]) in skipped_lookup
                else "ready",
                "skip_reason": "; ".join(
                    cast(
                        list[str],
                        skipped_lookup.get(str(case["id"]), {}).get("violations", []),
                    )
                )
                if str(case["id"]) in skipped_lookup
                else None,
            }
            for case in selected_cases
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
    for skipped in skipped_cases:
        case_slug = _slug_fragment(str(skipped["id"]))
        results.append(
            {
                "id": skipped["id"],
                "dataset": skipped["dataset"],
                "split": skipped["split"],
                "exit": 0,
                "run_dir": str(run_dir / f"skipped-{case_slug}"),
                "accuracy": None,
                "correct": None,
                "total": None,
                "avg_tokens": None,
                "max_tokens": None,
                "over_budget": None,
                "quality_gate_passed": None,
                "error": "; ".join(cast(list[str], skipped.get("violations", []))),
                "skipped": True,
            }
        )
    started_at = time.monotonic()
    for case_offset, case in enumerate(runnable_cases):
        elapsed_seconds = time.monotonic() - started_at
        if max_runtime_seconds > 0 and elapsed_seconds >= max_runtime_seconds:
            for skipped_case in runnable_cases[case_offset:]:
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
    executed_results = [result for result in results if not bool(result.get("skipped", False))]
    passed_cases = sum(1 for result in executed_results if int(result.get("exit", 1)) == 0)
    failed_case_count = sum(1 for result in executed_results if int(result.get("exit", 1)) != 0)
    skipped_case_count = sum(1 for result in results if bool(result.get("skipped", False)))
    print(
        json.dumps(
            {
                "matrix_file": str(matrix_path),
                "summary_file": str(summary_path),
                "cases_total": len(results),
                "cases_selected": len(selected_cases),
                "cases_runnable": len(runnable_cases),
                "cases_skipped_prereq": skipped_case_count,
                "cases_passed": passed_cases,
                "cases_failed": failed_case_count,
                "run_dir": str(run_dir),
            },
            indent=2,
        )
    )
    if failed_cases > 0:
        raise RuntimeError(
            f"matrix run failed for {failed_cases} case(s); see {summary_path} for details"
        )


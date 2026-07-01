"""CLI parser and entry helpers for run_amb_cortex."""
from __future__ import annotations

import argparse
import json
import os
import time
from datetime import datetime
from pathlib import Path

from run_amb_config import (
    BASELINE_FILE_DEFAULT,
    CADENCE_MATRIX_FILES_DEFAULT,
    DEFAULT_MEMORY_BACKEND,
    MATRIX_FILE_DEFAULT,
    QUALITY_TOKEN_TARGETS,
    REPO_ROOT,
    RETRIEVAL_PROFILES,
    RUNS_ROOT,
    SUPPORTED_MEMORY_BACKENDS,
)
from run_amb_execution import run_benchmark, run_matrix, run_smoke
from run_amb_preflight import (
    _resolve_matrix_path,
    _single_run_timeout_default,
    _slug_fragment,
)

def _build_cadence_matrix_args(
    args: argparse.Namespace,
    matrix_file: Path,
    summary_file: Path,
) -> argparse.Namespace:
    return argparse.Namespace(
        matrix_file=str(matrix_file),
        summary_file=str(summary_file),
        start_index=max(1, int(getattr(args, "start_index", 1))),
        max_cases=getattr(args, "max_cases", None),
        max_runtime_seconds=int(getattr(args, "max_runtime_seconds", 1200)),
        max_case_runtime_seconds=int(getattr(args, "max_case_runtime_seconds", 900)),
        run_name_prefix=str(getattr(args, "run_name_prefix", "matrix")),
        mode=str(getattr(args, "mode", "rag")),
        category=getattr(args, "category", None),
        memory_backend=str(getattr(args, "memory_backend", DEFAULT_MEMORY_BACKEND)),
        query_limit=getattr(args, "query_limit", None),
        query_id=getattr(args, "query_id", None),
        doc_limit=getattr(args, "doc_limit", None),
        oracle=bool(getattr(args, "oracle", False)),
        description=getattr(args, "description", None),
        continue_on_error=bool(getattr(args, "continue_on_error", False)),
        dry_run=bool(getattr(args, "dry_run", False)),
        token_gate_mode=str(getattr(args, "token_gate_mode", "dynamic")),
        provider_profile=str(getattr(args, "provider_profile", "auto")),
        baseline_file=str(getattr(args, "baseline_file", BASELINE_FILE_DEFAULT)),
        disable_baseline_gates=bool(getattr(args, "disable_baseline_gates", False)),
        no_auto_tighten_baseline=bool(getattr(args, "no_auto_tighten_baseline", False)),
        min_queries_for_baseline_update=int(
            getattr(args, "min_queries_for_baseline_update", 20)
        ),
        baseline_token_headroom_pct=float(
            getattr(args, "baseline_token_headroom_pct", 0.08)
        ),
        baseline_accuracy_headroom=float(
            getattr(args, "baseline_accuracy_headroom", 0.02)
        ),
        recall_budget=int(getattr(args, "recall_budget", 300)),
        quality_token_target=str(getattr(args, "quality_token_target", "custom")),
        retrieval_profile=str(getattr(args, "retrieval_profile", "max-quality")),
        min_accuracy=float(getattr(args, "min_accuracy", 0.90)),
        max_recall_tokens=int(getattr(args, "max_recall_tokens", 300)),
        max_avg_recall_tokens=float(getattr(args, "max_avg_recall_tokens", 300.0)),
        allow_missing_recall_metrics=bool(
            getattr(args, "allow_missing_recall_metrics", False)
        ),
        no_enforce_gate=bool(getattr(args, "no_enforce_gate", False)),
    )


def run_cadence(args: argparse.Namespace, run_dir: Path) -> None:
    raw_matrix_files = [
        str(item).strip()
        for item in list(getattr(args, "matrix_files", []))
        if str(item).strip()
    ]
    if not raw_matrix_files:
        raw_matrix_files = [str(path) for path in CADENCE_MATRIX_FILES_DEFAULT]
    matrix_paths = [_resolve_matrix_path(path) for path in raw_matrix_files]
    max_matrices = getattr(args, "max_matrices", None)
    if max_matrices is not None:
        matrix_paths = matrix_paths[: max(1, int(max_matrices))]

    if not matrix_paths:
        raise ValueError("cadence run requires at least one matrix file")

    continue_on_error = bool(getattr(args, "continue_on_error", False))
    cadence_results: list[dict[str, object]] = []
    failed_matrices = 0

    for index, matrix_path in enumerate(matrix_paths, start=1):
        matrix_slug = _slug_fragment(matrix_path.stem)
        matrix_run_dir = run_dir / f"{index:02d}-{matrix_slug}"
        matrix_run_dir.mkdir(parents=True, exist_ok=True)
        summary_path = matrix_run_dir / "matrix-summary.json"
        matrix_args = _build_cadence_matrix_args(args, matrix_path, summary_path)

        started_at = time.monotonic()
        status = "passed"
        error: str | None = None
        try:
            run_matrix(matrix_args, matrix_run_dir)
        except Exception as exc:
            status = "failed"
            error = str(exc)
            failed_matrices += 1

        cadence_results.append(
            {
                "index": index,
                "matrix_file": str(matrix_path),
                "matrix_run_dir": str(matrix_run_dir),
                "summary_file": str(summary_path),
                "status": status,
                "error": error,
                "duration_seconds": round(time.monotonic() - started_at, 2),
            }
        )

        if status == "failed" and not continue_on_error:
            break

    cadence_summary = {
        "command": "cadence",
        "created_at": datetime.now().isoformat(),
        "run_dir": str(run_dir),
        "matrix_count_requested": len(raw_matrix_files),
        "matrix_count_executed": len(cadence_results),
        "matrix_count_failed": failed_matrices,
        "continue_on_error": continue_on_error,
        "results": cadence_results,
    }
    summary_path = run_dir / "cadence-summary.json"
    summary_path.write_text(json.dumps(cadence_summary, indent=2), encoding="utf-8")
    print(json.dumps(cadence_summary, indent=2))

    if failed_matrices > 0:
        raise RuntimeError(
            f"cadence run failed for {failed_matrices} matrix invocation(s); see {summary_path}"
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
        "--quality-token-target",
        choices=sorted(QUALITY_TOKEN_TARGETS),
        default="custom",
        help=(
            "High-level quality-vs-token preset: custom, detail-first, balanced-detail, lean-detail. "
            "Non-custom targets map to detail-safe retrieval profiles and minimum accuracy floors."
        ),
    )
    target.add_argument(
        "--retrieval-profile",
        choices=sorted(RETRIEVAL_PROFILES),
        default="max-quality",
        help=(
            "Retrieval shaping profile applied as non-destructive env defaults: "
            "max-quality (detail-safe), balanced, efficiency-3pct, efficiency-5pct, token-saver."
        ),
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
    run.add_argument(
        "--memory-backend",
        choices=SUPPORTED_MEMORY_BACKENDS,
        default=os.environ.get("CORTEX_BENCHMARK_MEMORY_BACKEND", DEFAULT_MEMORY_BACKEND),
        help=(
            "Benchmark memory backend. "
            "'cortex-http' uses the tuned adapter client; "
            "'cortex-http-base' uses direct HTTP store/recall without helper client logic."
        ),
    )
    run.add_argument(
        "--run-name",
        default=None,
        help="Optional AMB run name. Defaults to the selected memory backend.",
    )
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
        help="Hard runtime cap per matrix case (defaults to 900 seconds / 15 minutes).",
    )
    matrix.add_argument(
        "--run-name-prefix",
        default="matrix",
        help="Prefix used for per-case run names when a case does not define run_name.",
    )
    matrix.add_argument("--mode", default="rag", help="Default AMB response mode for cases missing mode.")
    matrix.add_argument("--category", default=None, help="Default AMB category for cases missing category.")
    matrix.add_argument(
        "--memory-backend",
        choices=SUPPORTED_MEMORY_BACKENDS,
        default=os.environ.get("CORTEX_BENCHMARK_MEMORY_BACKEND", DEFAULT_MEMORY_BACKEND),
        help="Default memory backend for matrix cases missing memory_backend.",
    )
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

    cadence = subparsers.add_parser(
        "cadence",
        help="Run the broader benchmark matrix cadence sequence for post-gate verification.",
    )
    cadence.add_argument(
        "--matrix-files",
        nargs="+",
        default=[str(path) for path in CADENCE_MATRIX_FILES_DEFAULT],
        help=(
            "Ordered matrix specs for cadence execution. "
            "Defaults to stage1-q5 + practical non-longmem + fast non-longmem expansion."
        ),
    )
    cadence.add_argument(
        "--max-matrices",
        type=int,
        default=None,
        help="Optional cap on how many matrix files from --matrix-files to execute.",
    )
    cadence.add_argument(
        "--start-index",
        type=int,
        default=1,
        help="1-based case index applied to each matrix run.",
    )
    cadence.add_argument(
        "--max-cases",
        type=int,
        default=None,
        help="Optional max cases per matrix run.",
    )
    cadence.add_argument(
        "--max-runtime-seconds",
        type=int,
        default=int(os.environ.get("CORTEX_BENCHMARK_MATRIX_MAX_RUNTIME_SECONDS", "1200")),
        help="Hard runtime cap per matrix invocation.",
    )
    cadence.add_argument(
        "--max-case-runtime-seconds",
        type=int,
        default=int(os.environ.get("CORTEX_BENCHMARK_MATRIX_MAX_CASE_RUNTIME_SECONDS", "900")),
        help="Hard runtime cap per matrix case.",
    )
    cadence.add_argument(
        "--run-name-prefix",
        default="matrix",
        help="Prefix used for per-case run names when a matrix case omits run_name.",
    )
    cadence.add_argument("--mode", default="rag", help="Default AMB response mode for matrix cases.")
    cadence.add_argument("--category", default=None, help="Default AMB category for matrix cases.")
    cadence.add_argument(
        "--memory-backend",
        choices=SUPPORTED_MEMORY_BACKENDS,
        default=os.environ.get("CORTEX_BENCHMARK_MEMORY_BACKEND", DEFAULT_MEMORY_BACKEND),
        help="Default memory backend for matrix cases.",
    )
    cadence.add_argument("--query-limit", type=int, default=None, help="Default query limit for matrix cases.")
    cadence.add_argument("--query-id", default=None, help="Default query id for matrix cases.")
    cadence.add_argument("--doc-limit", type=int, default=None, help="Default doc limit for matrix cases.")
    cadence.add_argument("--oracle", action="store_true", help="Default oracle mode for matrix cases.")
    cadence.add_argument("--description", default=None, help="Default run description for matrix cases.")
    cadence.add_argument(
        "--continue-on-error",
        action="store_true",
        help="Continue remaining cadence matrix files after a matrix-level failure.",
    )
    cadence.add_argument(
        "--dry-run",
        action="store_true",
        help="Validate and expand matrix cases for each cadence matrix without executing AMB runs.",
    )
    _add_quality_gate_arguments(cadence)
    cadence.set_defaults(func=run_cadence)

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

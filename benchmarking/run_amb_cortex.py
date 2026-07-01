"""Run AMB against an isolated Cortex benchmark daemon."""
from __future__ import annotations

import sqlite3
import subprocess
import time

from run_amb_cli import build_parser, main, run_cadence
from run_amb_config import (
    BASELINE_FILE_DEFAULT,
    CADENCE_MATRIX_FILES_DEFAULT,
    CASE_ERROR_FILENAME,
    DEFAULT_MEMORY_BACKEND,
    FAIR_RUN_PREFLIGHT_FILENAME,
    MATRIX_FILE_DEFAULT,
    MEMBENCH_DEFAULT_DATA_PATH,
    QUALITY_TOKEN_TARGETS,
    RETRIEVAL_PROFILES,
    REPO_ROOT,
    RUNS_ROOT,
    SUPPORTED_MEMORY_BACKENDS,
    TOKEN_GATE_PROFILES,
    _resolve_quality_token_target,
)
from run_amb_daemon import IsolatedCortexDaemon, _resolve_memory_backend
from run_amb_execution import (
    _collect_matrix_case_result,
    _execute_benchmark_with_timeout,
    _execute_matrix_case,
    _execute_single_run,
    _summarize_recall_metrics,
    run_benchmark,
    run_matrix,
    run_smoke,
)
from run_amb_preflight import (
    _build_matrix_preflight,
    _build_matrix_run_args,
    _build_single_run_preflight,
    _load_matrix_cases,
    _load_matrix_spec,
    _resolve_single_run_timeout_seconds,
)
from run_amb_runtime import (
    _apply_dataset_compat_shims,
    _apply_retrieval_profile_defaults,
    _build_profile_delta_report,
    _cleanup_benchmark_namespace,
    _cleanup_benchmark_rows_in_db,
    _configure_imports,
    _configure_llm_environment,
    _context_efficiency_metrics,
    _env_flag_enabled,
    _recall_efficiency_metrics,
    _resolve_cortex_binary,
    _runtime_db_path_from_health,
    _seed_model_assets,
)

__all__ = [
    "BASELINE_FILE_DEFAULT",
    "CADENCE_MATRIX_FILES_DEFAULT",
    "CASE_ERROR_FILENAME",
    "DEFAULT_MEMORY_BACKEND",
    "FAIR_RUN_PREFLIGHT_FILENAME",
    "IsolatedCortexDaemon",
    "MATRIX_FILE_DEFAULT",
    "MEMBENCH_DEFAULT_DATA_PATH",
    "QUALITY_TOKEN_TARGETS",
    "REPO_ROOT",
    "RETRIEVAL_PROFILES",
    "RUNS_ROOT",
    "SUPPORTED_MEMORY_BACKENDS",
    "TOKEN_GATE_PROFILES",
    "_apply_dataset_compat_shims",
    "_apply_retrieval_profile_defaults",
    "_build_matrix_preflight",
    "_build_matrix_run_args",
    "_build_profile_delta_report",
    "_build_single_run_preflight",
    "_cleanup_benchmark_namespace",
    "_cleanup_benchmark_rows_in_db",
    "_collect_matrix_case_result",
    "_configure_imports",
    "_configure_llm_environment",
    "_context_efficiency_metrics",
    "_env_flag_enabled",
    "_execute_benchmark_with_timeout",
    "_execute_matrix_case",
    "_execute_single_run",
    "_load_matrix_cases",
    "_load_matrix_spec",
    "_recall_efficiency_metrics",
    "_resolve_cortex_binary",
    "_resolve_memory_backend",
    "_resolve_quality_token_target",
    "_resolve_single_run_timeout_seconds",
    "_runtime_db_path_from_health",
    "_seed_model_assets",
    "_summarize_recall_metrics",
    "build_parser",
    "main",
    "run_benchmark",
    "run_cadence",
    "run_matrix",
    "run_smoke",
    "sqlite3",
    "subprocess",
    "time",
]

if __name__ == "__main__":
    main()

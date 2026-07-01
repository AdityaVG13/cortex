#!/usr/bin/env python3
"""Split large benchmarking modules into subpackages (~1000 LOC max)."""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ADAPTERS = ROOT / "adapters"
RECALL_TUNING = ADAPTERS / "recall_tuning"
BENCHMARKING = ROOT


def read_lines(path: Path) -> list[str]:
    return path.read_text(encoding="utf-8").splitlines(keepends=True)


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if not content.endswith("\n"):
        content += "\n"
    path.write_text(content, encoding="utf-8")


def slice_lines(lines: list[str], start: int, end: int) -> str:
    return "".join(lines[start - 1 : end])


def mixin_module(header: str, class_name: str, body: str) -> str:
    return header + f"\n\nclass {class_name}:\n" + body


def class_module(header: str, class_name: str, bases: str, body: str) -> str:
    return header + f"\n\nclass {class_name}({bases}):\n" + body


def split_cortex_http_client() -> None:
    src = ADAPTERS / "cortex_http_client.py"
    lines = read_lines(src)

    write(
        RECALL_TUNING / "slugify.py",
        '''"""Shared slugify helper for benchmark namespace keys."""
from __future__ import annotations

import re


def slugify(value: str) -> str:
    slug = re.sub(r"[^a-zA-Z0-9._-]+", "-", value.strip().lower()).strip("-")
    return slug or "default"
''',
    )

    write(
        RECALL_TUNING / "client_patterns.py",
        '''"""Query/detail regex patterns for the tuned cortex-http client adapter."""
from __future__ import annotations

import re

'''
        + slice_lines(lines, 16, 244),
    )

    write(
        RECALL_TUNING / "__init__.py",
        '''"""Shared recall tuning helpers for cortex HTTP benchmark adapters."""
from __future__ import annotations

from recall_tuning.slugify import slugify

__all__ = ["slugify"]
''',
    )

    write(
        ADAPTERS / "cortex_http_client_recall.py",
        mixin_module(
            '''"""Recall orchestration mixin for CortexHTTPClient."""
from __future__ import annotations

import json
import re
from pathlib import Path
from typing import cast

from cortex_http_types import RecallResponse
from recall_tuning.client_patterns import *  # noqa: F403''',
            "CortexHTTPClientRecallMixin",
            slice_lines(lines, 696, 1374),
        ),
    )

    write(
        ADAPTERS / "cortex_http_client_scoring.py",
        mixin_module(
            '''"""Query scoring and reranking mixin for CortexHTTPClient."""
from __future__ import annotations

import re

from recall_tuning.client_patterns import *  # noqa: F403''',
            "CortexHTTPClientScoringMixin",
            slice_lines(lines, 1376, 2286),
        ),
    )

    core = (
        '''"""Tuned Cortex HTTP client for AMB benchmark runs."""
from __future__ import annotations

import json
import os
import re
import time
from dataclasses import dataclass
from hashlib import sha1
from pathlib import Path
from typing import cast

import httpx

from cortex_http_client_recall import CortexHTTPClientRecallMixin
from cortex_http_client_scoring import CortexHTTPClientScoringMixin
from cortex_http_types import HealthResponse, RecallResponse
from recall_tuning.client_patterns import *  # noqa: F403
from recall_tuning.slugify import slugify


@dataclass
class CortexStoredDocument:
    id: str
    content: str
    user_id: str | None = None
    timestamp: str | None = None
    context: str | None = None


'''
        + class_module(
            "",
            "CortexHTTPClient",
            "CortexHTTPClientRecallMixin, CortexHTTPClientScoringMixin",
            slice_lines(lines, 261, 694) + slice_lines(lines, 2288, 2327),
        ).lstrip()
    )
    write(src, core)


def split_cortex_http_base_provider() -> None:
    src = ADAPTERS / "cortex_http_base_provider.py"
    lines = read_lines(src)

    write(
        RECALL_TUNING / "base_patterns.py",
        '''"""Query/detail regex patterns for the cortex-http-base provider adapter."""
from __future__ import annotations

import re

'''
        + slice_lines(lines, 16, 248),
    )

    write(
        ADAPTERS / "cortex_http_base_ingest.py",
        mixin_module(
            '''"""Document ingest mixin for CortexHTTPBaseMemoryProvider."""
from __future__ import annotations

import json
import re
from hashlib import sha1

from memory_bench.models import Document
from recall_tuning.base_patterns import *  # noqa: F403''',
            "CortexHTTPBaseIngestMixin",
            slice_lines(lines, 365, 394) + slice_lines(lines, 605, 1017),
        ),
    )

    write(
        ADAPTERS / "cortex_http_base_recall.py",
        mixin_module(
            '''"""Retrieve and rerank mixin for CortexHTTPBaseMemoryProvider."""
from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import cast

from cortex_http_types import RecallResponse
from memory_bench.models import Document
from recall_tuning.base_patterns import *  # noqa: F403''',
            "CortexHTTPBaseRecallMixin",
            slice_lines(lines, 395, 528) + slice_lines(lines, 1018, 1878),
        ),
    )

    provider = (
        '''"""Direct Cortex HTTP memory provider for AMB (base adapter)."""
from __future__ import annotations

import os
import re
from hashlib import sha1
from pathlib import Path
from typing import cast

import httpx
from cortex_http_base_ingest import CortexHTTPBaseIngestMixin
from cortex_http_base_recall import CortexHTTPBaseRecallMixin
from cortex_http_types import HealthResponse, RecallResponse
from memory_bench.memory.base import MemoryProvider
from recall_tuning.slugify import slugify


'''
        + class_module(
            "",
            "CortexHTTPBaseMemoryProvider",
            "CortexHTTPBaseIngestMixin, CortexHTTPBaseRecallMixin, MemoryProvider",
            slice_lines(lines, 257, 364)
            + slice_lines(lines, 529, 604)
            + slice_lines(lines, 1880, 1911),
        ).lstrip()
    )
    write(src, provider.replace("_slugify(", "slugify("))


def split_run_amb_cortex() -> None:
    src = BENCHMARKING / "run_amb_cortex.py"
    lines = read_lines(src)

    write(
        BENCHMARKING / "run_amb_config.py",
        '''"""Configuration constants and retrieval profiles for run_amb_cortex."""
from __future__ import annotations

from pathlib import Path

'''
        + slice_lines(lines, 25, 236),
    )

    write(
        BENCHMARKING / "run_amb_runtime.py",
        '''"""Runtime helpers: imports, cleanup, profiles, and metrics for run_amb_cortex."""
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

'''
        + slice_lines(lines, 239, 788),
    )

    write(
        BENCHMARKING / "run_amb_preflight.py",
        '''"""Matrix loading, fairness preflight, and baseline helpers."""
from __future__ import annotations

import argparse
import json
import os
from datetime import datetime
from pathlib import Path

from run_amb_config import (
    BASELINE_FILE_DEFAULT,
    CASE_ERROR_FILENAME,
    FAIR_RUN_PREFLIGHT_FILENAME,
    MATRIX_CASE_TIMEOUT_MAX_SECONDS,
    MATRIX_FILE_DEFAULT,
    MATRIX_TIMEOUT_MAX_SECONDS,
    MEMBENCH_DEFAULT_DATA_PATH,
    MEMBENCH_REQUIRED_FILES,
    REPO_ROOT,
    SINGLE_RUN_TIMEOUT_ENV,
    SINGLE_RUN_TIMEOUT_MAX_SECONDS,
    SINGLE_RUN_TIMEOUT_MIN_SECONDS,
)
from run_amb_runtime import (
    _git_head_short,
    _load_lock_summary,
    _membench_missing_files,
    _resolve_membench_data_path,
    _resolve_single_run_timeout_seconds,
    _slug_fragment,
)

'''
        + slice_lines(lines, 790, 1477),
    )

    write(
        BENCHMARKING / "run_amb_daemon.py",
        '''"""Isolated Cortex daemon lifecycle for benchmark runs."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from contextlib import AbstractContextManager
from pathlib import Path
from typing import TextIO

import httpx

from run_amb_config import REPO_ROOT
from run_amb_runtime import (
    _env_flag_enabled,
    _find_free_port,
    _resolve_cortex_binary,
    _seed_model_assets,
)

'''
        + slice_lines(lines, 1790, 2007),
    )

    write(
        BENCHMARKING / "run_amb_execution.py",
        '''"""Benchmark execution, matrix runs, cadence, and quality gates."""
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
    QUALITY_TOKEN_TARGETS,
    REPO_ROOT,
    RUNS_ROOT,
    SUPPORTED_MEMORY_BACKENDS,
)
from run_amb_daemon import IsolatedCortexDaemon, _register_provider, _resolve_memory_backend
from run_amb_preflight import (
    _build_matrix_preflight,
    _build_matrix_run_args,
    _build_single_run_preflight,
    _derive_effective_constraints,
    _get_baseline_entry,
    _load_baseline_store,
    _load_matrix_cases,
    _load_matrix_spec,
    _save_baseline_store,
    _scenario_key,
    _tighten_baseline_entry,
    _validate_matrix_fairness,
)
from run_amb_runtime import (
    _apply_dataset_compat_shims,
    _apply_retrieval_profile_defaults,
    _assert_amb_environment,
    _configure_imports,
    _configure_llm_environment,
    _context_efficiency_metrics,
    _env_flag_enabled,
    _filter_kwargs_for_callable,
    _prepare_worker_runtime_env,
    _recall_efficiency_metrics,
    _resolve_quality_token_target,
    _resolve_token_gate_limits,
)

'''
        + slice_lines(lines, 1478, 1788)
        + slice_lines(lines, 2008, 2597),
    )

    write(
        BENCHMARKING / "run_amb_cli.py",
        '''"""CLI parser and entry helpers for run_amb_cortex."""
from __future__ import annotations

import argparse
import os
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
from run_amb_runtime import _single_run_timeout_default

'''
        + slice_lines(lines, 2599, 2719)
        + slice_lines(lines, 2721, 3030),
    )

    shim = '''"""Run AMB against an isolated Cortex benchmark daemon."""
from __future__ import annotations

from run_amb_cli import build_parser, main
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
)
from run_amb_daemon import IsolatedCortexDaemon, _resolve_memory_backend
from run_amb_execution import (
    _execute_single_run,
    run_benchmark,
    run_matrix,
    run_smoke,
)
from run_amb_cli import run_cadence
from run_amb_preflight import (
    _build_matrix_preflight,
    _build_matrix_run_args,
    _build_single_run_preflight,
    _collect_matrix_case_result,
    _execute_matrix_case,
    _load_matrix_cases,
    _load_matrix_spec,
)
from run_amb_runtime import (
    _apply_dataset_compat_shims,
    _apply_retrieval_profile_defaults,
    _build_profile_delta_report,
    _cleanup_benchmark_rows_in_db,
    _configure_imports,
    _configure_llm_environment,
    _context_efficiency_metrics,
    _env_flag_enabled,
    _resolve_memory_backend,
    _resolve_quality_token_target,
    _resolve_single_run_timeout_seconds,
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
    "_cleanup_benchmark_rows_in_db",
    "_collect_matrix_case_result",
    "_configure_imports",
    "_configure_llm_environment",
    "_context_efficiency_metrics",
    "_env_flag_enabled",
    "_execute_matrix_case",
    "_execute_single_run",
    "_load_matrix_cases",
    "_load_matrix_spec",
    "_resolve_memory_backend",
    "_resolve_quality_token_target",
    "_resolve_single_run_timeout_seconds",
    "_seed_model_assets",
    "build_parser",
    "main",
    "run_benchmark",
    "run_cadence",
    "run_matrix",
    "run_smoke",
]

if __name__ == "__main__":
    main()
'''
    write(src, shim)


def main() -> None:
    split_cortex_http_client()
    split_cortex_http_base_provider()
    split_run_amb_cortex()
    print("benchmarking module split complete")


if __name__ == "__main__":
    main()

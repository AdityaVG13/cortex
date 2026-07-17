"""Configuration constants and retrieval profiles for run_amb_cortex."""
from __future__ import annotations

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
AMB_SRC = REPO_ROOT / "benchmarking" / "tools" / "agent-memory-benchmark" / "src"
ADAPTERS_DIR = REPO_ROOT / "benchmarking" / "adapters"
RUNS_ROOT = REPO_ROOT / "benchmarking" / "runs"
BASELINE_FILE_DEFAULT = REPO_ROOT / "benchmarking" / "configs" / "token-gate-baselines.json"
MATRIX_FILE_DEFAULT = REPO_ROOT / "benchmarking" / "configs" / "amb-eval-matrix.stage1.json"
CADENCE_MATRIX_FILES_DEFAULT: tuple[Path, ...] = (
    REPO_ROOT / "benchmarking" / "configs" / "amb-eval-matrix.stage1.q5.json",
    REPO_ROOT / "benchmarking" / "configs" / "amb-eval-matrix.nonlongmem.practical.json",
    REPO_ROOT / "benchmarking" / "configs" / "amb-eval-matrix.nonlongmem.expansion.fast.json",
)
DEFAULT_MEMORY_BACKEND = "cortex-http"
SUPPORTED_MEMORY_BACKENDS = (
    "cortex-http",
    "cortex-http-base",
    "cortex-http-pure",
)
TOKEN_GATE_PROFILES: dict[str, dict[str, float]] = {
    # Tighter ratios for providers that tend to carry heavier prompt wrappers/history overhead.
    "claude": {"max_avg_ratio": 0.72, "max_peak_ratio": 0.90},
    "openai": {"max_avg_ratio": 0.80, "max_peak_ratio": 1.00},
    "codex": {"max_avg_ratio": 0.78, "max_peak_ratio": 0.98},
    "gemini": {"max_avg_ratio": 0.82, "max_peak_ratio": 1.00},
    "groq": {"max_avg_ratio": 0.84, "max_peak_ratio": 1.00},
    "default": {"max_avg_ratio": 0.80, "max_peak_ratio": 1.00},
}
RETRIEVAL_PROFILES: dict[str, dict[str, str]] = {
    # Default profile favors answer completeness and non-lossy user-fact shaping.
    "max-quality": {
        "CORTEX_BENCHMARK_STORE_FULL_DOCS": "0",
        "CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS": "1",
        "CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC": "18",
        "CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS": "900",
        "CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS": "0",
        "CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS": "1",
        "CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO": "0.45",
        "CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET": "128",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER": "14",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN": "140",
        "CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED": "2",
        "CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS": "10",
        "CORTEX_BENCHMARK_DETAIL_SIBLING_SCORE_MARGIN": "18",
        "CORTEX_BENCHMARK_SHORT_REPLY_QUESTION_MAX_CHARS": "160",
        "CORTEX_BENCHMARK_CONTEXT_MAX_CHARS": "760",
        "CORTEX_BENCHMARK_QUERY_WINDOW_CHARS": "280",
        "CORTEX_BENCHMARK_MAX_QUERY_WINDOWS_PER_TERM": "3",
        "CORTEX_BENCHMARK_USE_RECALL_EXCERPTS": "1",
        "CORTEX_BENCHMARK_ANSWER_SOURCE_PENALTY": "26",
        "CORTEX_BENCHMARK_RETRIEVAL_POLICY": "high-detail",
    },
    "balanced": {
        "CORTEX_BENCHMARK_STORE_FULL_DOCS": "0",
        "CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS": "1",
        "CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC": "14",
        "CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS": "800",
        "CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS": "0",
        "CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS": "1",
        "CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO": "0.35",
        "CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET": "96",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER": "12",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN": "120",
        "CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED": "2",
        "CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS": "10",
        "CORTEX_BENCHMARK_DETAIL_SIBLING_SCORE_MARGIN": "18",
        "CORTEX_BENCHMARK_SHORT_REPLY_QUESTION_MAX_CHARS": "160",
        "CORTEX_BENCHMARK_CONTEXT_MAX_CHARS": "620",
        "CORTEX_BENCHMARK_QUERY_WINDOW_CHARS": "240",
        "CORTEX_BENCHMARK_MAX_QUERY_WINDOWS_PER_TERM": "3",
        "CORTEX_BENCHMARK_USE_RECALL_EXCERPTS": "1",
        "CORTEX_BENCHMARK_ANSWER_SOURCE_PENALTY": "24",
        "CORTEX_BENCHMARK_RETRIEVAL_POLICY": "high-detail",
    },
    # Lower-token mode targeting roughly <=3% quality loss versus balanced.
    "efficiency-3pct": {
        "CORTEX_BENCHMARK_STORE_FULL_DOCS": "0",
        "CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS": "1",
        "CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC": "12",
        "CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS": "740",
        "CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS": "0",
        "CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS": "1",
        "CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO": "0.32",
        "CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET": "92",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER": "11",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN": "110",
        "CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED": "2",
        "CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS": "8",
        "CORTEX_BENCHMARK_DETAIL_SIBLING_SCORE_MARGIN": "18",
        "CORTEX_BENCHMARK_SHORT_REPLY_QUESTION_MAX_CHARS": "150",
        "CORTEX_BENCHMARK_CONTEXT_MAX_CHARS": "560",
        "CORTEX_BENCHMARK_QUERY_WINDOW_CHARS": "220",
        "CORTEX_BENCHMARK_MAX_QUERY_WINDOWS_PER_TERM": "3",
        "CORTEX_BENCHMARK_USE_RECALL_EXCERPTS": "1",
        "CORTEX_BENCHMARK_ANSWER_SOURCE_PENALTY": "22",
        "CORTEX_BENCHMARK_RETRIEVAL_POLICY": "high-detail",
    },
    # Explicit low-loss efficiency profile intended for ~3-5% accuracy tradeoff windows.
    "efficiency-5pct": {
        "CORTEX_BENCHMARK_STORE_FULL_DOCS": "0",
        "CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS": "1",
        "CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC": "10",
        "CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS": "680",
        "CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS": "0",
        "CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS": "1",
        "CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO": "0.30",
        "CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET": "88",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER": "11",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN": "110",
        "CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED": "2",
        "CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS": "8",
        "CORTEX_BENCHMARK_DETAIL_SIBLING_SCORE_MARGIN": "18",
        "CORTEX_BENCHMARK_SHORT_REPLY_QUESTION_MAX_CHARS": "140",
        "CORTEX_BENCHMARK_CONTEXT_MAX_CHARS": "540",
        "CORTEX_BENCHMARK_QUERY_WINDOW_CHARS": "220",
        "CORTEX_BENCHMARK_MAX_QUERY_WINDOWS_PER_TERM": "3",
        "CORTEX_BENCHMARK_USE_RECALL_EXCERPTS": "1",
        "CORTEX_BENCHMARK_ANSWER_SOURCE_PENALTY": "20",
        "CORTEX_BENCHMARK_RETRIEVAL_POLICY": "high-detail",
    },
    "token-saver": {
        "CORTEX_BENCHMARK_STORE_FULL_DOCS": "0",
        "CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS": "1",
        "CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC": "8",
        "CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS": "520",
        "CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS": "0",
        "CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS": "0",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER": "8",
        "CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN": "80",
        "CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED": "1",
        "CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS": "4",
        "CORTEX_BENCHMARK_DETAIL_SIBLING_SCORE_MARGIN": "18",
        "CORTEX_BENCHMARK_SHORT_REPLY_QUESTION_MAX_CHARS": "120",
        "CORTEX_BENCHMARK_CONTEXT_MAX_CHARS": "420",
        "CORTEX_BENCHMARK_QUERY_WINDOW_CHARS": "160",
        "CORTEX_BENCHMARK_MAX_QUERY_WINDOWS_PER_TERM": "2",
        "CORTEX_BENCHMARK_USE_RECALL_EXCERPTS": "1",
        "CORTEX_BENCHMARK_ANSWER_SOURCE_PENALTY": "18",
        "CORTEX_BENCHMARK_RETRIEVAL_POLICY": "standard",
    },
}
QUALITY_TOKEN_TARGETS: dict[str, dict[str, object]] = {
    "custom": {
        "retrieval_profile": None,
        "min_accuracy_floor": None,
        "description": "Use explicit retrieval/profile gate settings as provided.",
    },
    "detail-first": {
        "retrieval_profile": "max-quality",
        "min_accuracy_floor": 0.92,
        "description": "Prioritize exact detail recall with stricter quality floor.",
    },
    "balanced-detail": {
        "retrieval_profile": "balanced",
        "min_accuracy_floor": 0.90,
        "description": "Balance recall-token efficiency while keeping detail-safe retrieval shaping.",
    },
    "lean-detail": {
        "retrieval_profile": "efficiency-5pct",
        "min_accuracy_floor": 0.88,
        "description": "Lower token cost with detail-preserving retrieval shaping and guarded quality floor.",
    },
}
SINGLE_RUN_TIMEOUT_ENV = "CORTEX_BENCHMARK_RUN_MAX_RUNTIME_SECONDS"
SINGLE_RUN_TIMEOUT_MIN_SECONDS = 900
SINGLE_RUN_TIMEOUT_MAX_SECONDS = 1200
SINGLE_RUN_TIMEOUT_DEFAULT_SECONDS = 1200
MATRIX_TIMEOUT_MAX_SECONDS = 1200
MATRIX_CASE_TIMEOUT_MAX_SECONDS = 900
FAIR_RUN_PREFLIGHT_FILENAME = "fair-run-preflight.json"
CLEANUP_DB_RETRY_ATTEMPTS = 4
CLEANUP_DB_RETRY_BASE_DELAY_SECONDS = 0.25
CASE_ERROR_FILENAME = "case-error.json"
MEMBENCH_REQUIRED_FILES = (
    "FirstAgentDataLowLevel.json",
    "FirstAgentDataHighLevel.json",
    "ThirdAgentDataLowLevel.json",
    "ThirdAgentDataHighLevel.json",
)
MEMBENCH_DEFAULT_DATA_PATH = Path("./MemData")


def _resolve_quality_token_target(
    *,
    target: str,
    retrieval_profile: str,
    min_accuracy: float,
) -> dict[str, object]:
    normalized_target = str(target or "custom").strip().lower()
    if normalized_target not in QUALITY_TOKEN_TARGETS:
        known = ", ".join(sorted(QUALITY_TOKEN_TARGETS))
        raise ValueError(
            f"unknown quality/token target '{target}'. Expected one of: {known}"
        )
    plan = QUALITY_TOKEN_TARGETS[normalized_target]
    selected_retrieval_profile = str(retrieval_profile)
    selected_min_accuracy = float(min_accuracy)
    target_profile = plan.get("retrieval_profile")
    target_min_accuracy = plan.get("min_accuracy_floor")
    if isinstance(target_profile, str):
        selected_retrieval_profile = target_profile
    if isinstance(target_min_accuracy, (int, float)):
        selected_min_accuracy = max(selected_min_accuracy, float(target_min_accuracy))
    return {
        "target": normalized_target,
        "description": str(plan.get("description", "")),
        "requested_retrieval_profile": str(retrieval_profile),
        "effective_retrieval_profile": selected_retrieval_profile,
        "requested_min_accuracy": round(float(min_accuracy), 4),
        "effective_min_accuracy": round(selected_min_accuracy, 4),
        "target_retrieval_profile": target_profile,
        "target_min_accuracy_floor": target_min_accuracy,
        "applied": normalized_target != "custom",
    }

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[3]
BENCHMARKING_DIR = REPO_ROOT / "benchmarking"
if str(BENCHMARKING_DIR) not in sys.path:
    sys.path.insert(0, str(BENCHMARKING_DIR))

from run_amb_cortex import (  # noqa: E402
    _build_matrix_run_args,
    _collect_matrix_case_result,
    _execute_matrix_case,
    _load_matrix_cases,
    run_matrix,
)

_PRACTICAL_NON_LONGMEM_DATASETS = {
    "locomo",
    "lifebench",
    "membench",
    "personamem",
    "memsim",
    "beam",
}


def _base_args() -> argparse.Namespace:
    return argparse.Namespace(
        mode="rag",
        category=None,
        query_limit=None,
        query_id=None,
        doc_limit=None,
        oracle=False,
        description=None,
        run_name_prefix="matrix",
        token_gate_mode="dynamic",
        provider_profile="auto",
        baseline_file="benchmarking/configs/token-gate-baselines.json",
        disable_baseline_gates=False,
        no_auto_tighten_baseline=False,
        min_queries_for_baseline_update=20,
        baseline_token_headroom_pct=0.08,
        baseline_accuracy_headroom=0.02,
        recall_budget=300,
        quality_token_target="custom",
        retrieval_profile="max-quality",
        min_accuracy=0.9,
        max_recall_tokens=300,
        max_avg_recall_tokens=300.0,
        allow_missing_recall_metrics=False,
        no_enforce_gate=False,
        start_index=1,
        max_cases=None,
        max_runtime_seconds=1200,
        max_case_runtime_seconds=900,
        dry_run=False,
        continue_on_error=False,
        matrix_file=None,
        summary_file=None,
    )


def test_load_matrix_cases_accepts_cases_object(tmp_path: Path) -> None:
    matrix_path = tmp_path / "matrix.json"
    matrix_path.write_text(
        json.dumps(
            {
                "cases": [
                    {
                        "dataset": "longmemeval",
                        "split": "s",
                        "query_limit": 20,
                        "retrieval_profile": "balanced",
                    },
                    {"dataset": "membench", "split": "FirstAgentLowLevel"},
                ]
            }
        ),
        encoding="utf-8",
    )
    cases = _load_matrix_cases(matrix_path)

    assert len(cases) == 2
    assert cases[0]["dataset"] == "longmemeval"
    assert cases[0]["split"] == "s"
    assert cases[0]["query_limit"] == 20
    assert cases[0]["retrieval_profile"] == "balanced"
    assert str(cases[0]["id"]).startswith("01-")
    assert cases[1]["dataset"] == "membench"
    assert str(cases[1]["id"]).startswith("02-")


def test_load_matrix_cases_rejects_invalid_schema(tmp_path: Path) -> None:
    matrix_path = tmp_path / "invalid.json"
    matrix_path.write_text(json.dumps({"cases": [{"split": "s"}]}), encoding="utf-8")

    with pytest.raises(ValueError, match="dataset"):
        _load_matrix_cases(matrix_path)


def test_build_matrix_run_args_applies_case_overrides() -> None:
    args = _base_args()
    case = {
        "id": "locomo-locomo10",
        "dataset": "locomo",
        "split": "locomo10",
        "mode": "rag",
        "query_limit": 12,
        "oracle": True,
        "recall_budget": 420,
        "quality_token_target": "lean-detail",
        "retrieval_profile": "token-saver",
    }
    run_args = _build_matrix_run_args(args, case)

    assert run_args.dataset == "locomo"
    assert run_args.split == "locomo10"
    assert run_args.query_limit == 12
    assert run_args.oracle is True
    assert run_args.recall_budget == 420
    assert run_args.quality_token_target == "lean-detail"
    assert run_args.retrieval_profile == "token-saver"
    assert run_args.run_name == "matrix-locomo-locomo10"


def test_build_matrix_run_args_uses_base_retrieval_profile_by_default() -> None:
    args = _base_args()
    args.retrieval_profile = "balanced"
    case = {
        "id": "longmemeval-s",
        "dataset": "longmemeval",
        "split": "s",
    }

    run_args = _build_matrix_run_args(args, case)

    assert run_args.retrieval_profile == "balanced"


def test_collect_matrix_case_result_reads_summary_and_gate(tmp_path: Path) -> None:
    run_dir = tmp_path / "run"
    run_dir.mkdir(parents=True, exist_ok=True)
    (run_dir / "summary.json").write_text(
        json.dumps({"accuracy": 0.55, "correct": 11, "total_queries": 20}),
        encoding="utf-8",
    )
    (run_dir / "gate-report.json").write_text(
        json.dumps(
            {
                "quality_gate": {"passed": False, "failures": ["accuracy below floor"]},
                "recall_stats": {
                    "avg_recall_tokens": 226.7,
                    "max_recall_tokens": 326,
                    "over_budget_count": 3,
                },
                "score_per_1k_recall_tokens": 41.2844,
                "tradeoff": {
                    "quality_token_target": "balanced-detail",
                    "effective_retrieval_profile": "balanced",
                    "profile_delta": {
                        "delta_vs_token_gate": {
                            "max_recall_tokens": {"absolute": -14.0, "percent": -4.29}
                        }
                    },
                },
            }
        ),
        encoding="utf-8",
    )
    result = _collect_matrix_case_result(
        case={"id": "longmemeval-s", "dataset": "longmemeval", "split": "s"},
        run_dir=run_dir,
        exit_code=1,
        error="quality gate failed",
    )

    assert result["dataset"] == "longmemeval"
    assert result["split"] == "s"
    assert result["accuracy"] == 0.55
    assert result["correct"] == 11
    assert result["total"] == 20
    assert result["avg_tokens"] == 226.7
    assert result["max_tokens"] == 326
    assert result["over_budget"] == 3
    assert result["score_per_1k_recall_tokens"] == 41.2844
    assert result["quality_token_target"] == "balanced-detail"
    assert result["retrieval_profile_effective"] == "balanced"
    assert result["profile_delta_vs_token_gate"]["max_recall_tokens"]["absolute"] == -14.0
    assert result["quality_gate_passed"] is False
    assert result["error"] == "quality gate failed"


def test_run_matrix_dry_run_honors_start_index_and_max_cases(tmp_path: Path) -> None:
    matrix_path = tmp_path / "matrix.json"
    summary_path = tmp_path / "summary.json"
    run_dir = tmp_path / "run"
    run_dir.mkdir(parents=True, exist_ok=True)
    matrix_path.write_text(
        json.dumps(
            {
                "cases": [
                    {"id": "case-a", "dataset": "longmemeval", "split": "s"},
                    {"id": "case-b", "dataset": "locomo", "split": "locomo10"},
                    {"id": "case-c", "dataset": "beam", "split": "100k"},
                ]
            }
        ),
        encoding="utf-8",
    )
    args = _base_args()
    args.matrix_file = str(matrix_path)
    args.summary_file = str(summary_path)
    args.dry_run = True
    args.start_index = 2
    args.max_cases = 1

    run_matrix(args, run_dir)

    preview = json.loads(summary_path.read_text(encoding="utf-8"))
    assert len(preview) == 1
    assert preview[0]["id"] == "case-b"
    assert preview[0]["dataset"] == "locomo"
    preflight = json.loads((run_dir / "fair-run-preflight.json").read_text(encoding="utf-8"))
    assert preflight["passed"] is True


def test_execute_matrix_case_returns_error_for_run_benchmark_exception(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_dir = tmp_path / "run"
    run_dir.mkdir(parents=True, exist_ok=True)

    def _boom(_: argparse.Namespace, __: Path) -> None:
        raise RuntimeError("intentional-case-failure")

    monkeypatch.setattr("run_amb_cortex.run_benchmark", _boom)

    exit_code, error = _execute_matrix_case(
        run_args=_base_args(),
        run_dir=run_dir,
        timeout_seconds=0,
    )
    assert exit_code == 1
    assert error == "intentional-case-failure"


def _assert_strict_non_longmem_matrix(cases: list[dict[str, object]], *, query_limit: int) -> None:
    assert cases
    datasets = {str(case["dataset"]) for case in cases}
    assert datasets == _PRACTICAL_NON_LONGMEM_DATASETS
    assert all(str(case["dataset"]) != "longmemeval" for case in cases)
    assert all(int(case.get("query_limit", -1)) == query_limit for case in cases)
    assert all(bool(case.get("oracle", False)) is False for case in cases)
    assert all("query_id" not in case for case in cases)
    assert all("doc_limit" not in case for case in cases)


def test_strict_nonlongmem_q5_matrix_is_fair_and_no_longmemeval() -> None:
    matrix_path = BENCHMARKING_DIR / "configs" / "amb-eval-matrix.nonlongmem.q5.json"
    cases = _load_matrix_cases(matrix_path)

    _assert_strict_non_longmem_matrix(cases, query_limit=5)


def test_strict_nonlongmem_q10_matrix_is_fair_and_no_longmemeval() -> None:
    matrix_path = BENCHMARKING_DIR / "configs" / "amb-eval-matrix.nonlongmem.q10.json"
    cases = _load_matrix_cases(matrix_path)

    _assert_strict_non_longmem_matrix(cases, query_limit=10)


def test_strict_nonlongmem_q5_matrix_execution_profile_is_strict() -> None:
    matrix_path = BENCHMARKING_DIR / "configs" / "amb-eval-matrix.nonlongmem.q5.json"
    payload = json.loads(matrix_path.read_text(encoding="utf-8"))
    profile = payload["execution_profile"]

    assert profile["max_runtime_seconds"] == 1200
    assert profile["max_case_runtime_seconds"] == 900
    assert profile["oracle"] is False
    assert profile["no_enforce_gate"] is False
    assert profile["allow_missing_recall_metrics"] is False


def test_run_matrix_applies_execution_profile_and_clears_oracle_default(tmp_path: Path) -> None:
    matrix_path = tmp_path / "matrix-profile.json"
    summary_path = tmp_path / "summary.json"
    run_dir = tmp_path / "run"
    run_dir.mkdir(parents=True, exist_ok=True)
    matrix_path.write_text(
        json.dumps(
            {
                "execution_profile": {
                    "max_runtime_seconds": 1200,
                    "max_case_runtime_seconds": 900,
                    "oracle": False,
                },
                "cases": [
                    {"id": "case-a", "dataset": "locomo", "split": "locomo10", "query_limit": 5}
                ],
            }
        ),
        encoding="utf-8",
    )
    args = _base_args()
    args.matrix_file = str(matrix_path)
    args.summary_file = str(summary_path)
    args.max_runtime_seconds = 9999
    args.max_case_runtime_seconds = 9999
    args.oracle = True
    args.dry_run = True

    run_matrix(args, run_dir)

    assert args.max_runtime_seconds == 1200
    assert args.max_case_runtime_seconds == 900
    assert args.oracle is False
    preview = json.loads(summary_path.read_text(encoding="utf-8"))
    assert len(preview) == 1
    assert preview[0]["dataset"] == "locomo"


def test_run_matrix_rejects_oracle_case_even_in_dry_run(tmp_path: Path) -> None:
    matrix_path = tmp_path / "matrix-oracle.json"
    run_dir = tmp_path / "run"
    run_dir.mkdir(parents=True, exist_ok=True)
    matrix_path.write_text(
        json.dumps(
            {
                "cases": [
                    {"id": "case-a", "dataset": "locomo", "split": "locomo10", "query_limit": 5, "oracle": True}
                ]
            }
        ),
        encoding="utf-8",
    )
    args = _base_args()
    args.matrix_file = str(matrix_path)
    args.dry_run = True

    with pytest.raises(ValueError, match="oracle"):
        run_matrix(args, run_dir)


def test_run_matrix_rejects_default_query_id_even_in_dry_run(tmp_path: Path) -> None:
    matrix_path = tmp_path / "matrix-query-id.json"
    run_dir = tmp_path / "run"
    run_dir.mkdir(parents=True, exist_ok=True)
    matrix_path.write_text(
        json.dumps({"cases": [{"id": "case-a", "dataset": "locomo", "split": "locomo10"}]}),
        encoding="utf-8",
    )
    args = _base_args()
    args.matrix_file = str(matrix_path)
    args.query_id = "q-1"
    args.dry_run = True

    with pytest.raises(ValueError, match="query_id"):
        run_matrix(args, run_dir)


def test_run_matrix_rejects_default_doc_limit_even_in_dry_run(tmp_path: Path) -> None:
    matrix_path = tmp_path / "matrix-doc-limit.json"
    run_dir = tmp_path / "run"
    run_dir.mkdir(parents=True, exist_ok=True)
    matrix_path.write_text(
        json.dumps({"cases": [{"id": "case-a", "dataset": "locomo", "split": "locomo10"}]}),
        encoding="utf-8",
    )
    args = _base_args()
    args.matrix_file = str(matrix_path)
    args.doc_limit = 1
    args.dry_run = True

    with pytest.raises(ValueError, match="doc_limit"):
        run_matrix(args, run_dir)


def test_run_matrix_rejects_matrix_runtime_cap_above_ceiling(tmp_path: Path) -> None:
    matrix_path = tmp_path / "matrix-timeout.json"
    run_dir = tmp_path / "run"
    run_dir.mkdir(parents=True, exist_ok=True)
    matrix_path.write_text(
        json.dumps({"cases": [{"id": "case-a", "dataset": "locomo", "split": "locomo10"}]}),
        encoding="utf-8",
    )
    args = _base_args()
    args.matrix_file = str(matrix_path)
    args.max_runtime_seconds = 1201
    args.dry_run = True

    with pytest.raises(ValueError, match="max-runtime-seconds"):
        run_matrix(args, run_dir)


def test_practical_nonlongmem_matrix_is_fair_and_timeout_friendly() -> None:
    matrix_path = BENCHMARKING_DIR / "configs" / "amb-eval-matrix.nonlongmem.practical.json"
    cases = _load_matrix_cases(matrix_path)

    assert cases
    datasets = {str(case["dataset"]) for case in cases}
    assert datasets == _PRACTICAL_NON_LONGMEM_DATASETS
    assert all(str(case["dataset"]) != "longmemeval" for case in cases)
    assert all(bool(case.get("oracle", False)) is False for case in cases)
    assert all("query_id" not in case for case in cases)
    assert all("doc_limit" not in case for case in cases)
    assert all(1 <= int(case.get("query_limit", 0)) <= 6 for case in cases)


def test_practical_nonlongmem_matrix_execution_profile_is_strict() -> None:
    matrix_path = BENCHMARKING_DIR / "configs" / "amb-eval-matrix.nonlongmem.practical.json"
    payload = json.loads(matrix_path.read_text(encoding="utf-8"))
    profile = payload["execution_profile"]

    assert profile["max_runtime_seconds"] == 1200
    assert profile["max_case_runtime_seconds"] == 900
    assert profile["token_gate_mode"] == "dynamic"
    assert profile["provider_profile"] == "gemini"
    assert profile["recall_budget"] == 300
    assert profile["min_accuracy"] == 0.9
    assert profile["max_recall_tokens"] == 300
    assert profile["max_avg_recall_tokens"] == 246.0
    assert profile["allow_missing_recall_metrics"] is False
    assert profile["no_enforce_gate"] is False
    assert profile["oracle"] is False

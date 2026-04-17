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
    _load_matrix_cases,
)


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
        min_accuracy=0.9,
        max_recall_tokens=300,
        max_avg_recall_tokens=300.0,
        allow_missing_recall_metrics=False,
        no_enforce_gate=False,
    )


def test_load_matrix_cases_accepts_cases_object(tmp_path: Path) -> None:
    matrix_path = tmp_path / "matrix.json"
    matrix_path.write_text(
        json.dumps(
            {
                "cases": [
                    {"dataset": "longmemeval", "split": "s", "query_limit": 20},
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
    }
    run_args = _build_matrix_run_args(args, case)

    assert run_args.dataset == "locomo"
    assert run_args.split == "locomo10"
    assert run_args.query_limit == 12
    assert run_args.oracle is True
    assert run_args.recall_budget == 420
    assert run_args.run_name == "matrix-locomo-locomo10"


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
    assert result["quality_gate_passed"] is False
    assert result["error"] == "quality gate failed"

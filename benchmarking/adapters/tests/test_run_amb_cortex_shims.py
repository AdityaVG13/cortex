from __future__ import annotations

import json
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
BENCHMARKING_DIR = REPO_ROOT / "benchmarking"
if str(BENCHMARKING_DIR) not in sys.path:
    sys.path.insert(0, str(BENCHMARKING_DIR))

from run_amb_cortex import _apply_dataset_compat_shims  # noqa: E402


class _Doc:
    def __init__(self, user_id: str, content: str) -> None:
        self.user_id = user_id
        self.content = content


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


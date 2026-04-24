"""
Cortex Pure HTTP Memory Provider.

Zero helpers. Single `POST /recall` per query. Daemon results returned
verbatim in daemon rank order. This adapter measures core daemon quality
only -- no intent detection, no query expansion, no post-hoc reranking,
no related-memory injection, no detail-bonus scoring.

Protected by purity gates in scripts/purity-gates/. Do not add tuning
logic to this file. Open a new adapter if helpers are needed.
"""
from __future__ import annotations

import os
from typing import Any

import httpx
from memory_bench.memory.base import MemoryProvider
from memory_bench.models import Document

CORTEX_URL = os.environ.get("CORTEX_URL", "http://127.0.0.1:7437")
CORTEX_TOKEN = os.environ.get("CORTEX_API_KEY")
DEFAULT_BUDGET_TOKENS = 200
DEFAULT_K = 10


class CortexHTTPPureMemoryProvider(MemoryProvider):
    """Pure-HTTP passthrough adapter. Zero tuning. Canonical measurement floor."""

    name = "cortex-http-pure"

    def __init__(self, *, url: str | None = None, token: str | None = None) -> None:
        self._url = url or CORTEX_URL
        self._token = token or CORTEX_TOKEN
        self._client: httpx.Client | None = None

    def initialize(self) -> None:
        self._forbid_helper_env_vars()
        self._client = httpx.Client(
            base_url=self._url,
            headers=self._auth_headers(),
            timeout=30.0,
        )

    def cleanup(self) -> None:
        if self._client is not None:
            self._client.close()
            self._client = None

    def prepare(self, store_dir, unit_ids=None, reset=True) -> None:
        """No-op prepare. Daemon is externally managed; caller ensures isolation."""
        return None

    def store(self, text: str, *, metadata: dict[str, Any] | None = None) -> None:
        """Single POST /store. No enrichment. No classifier hints."""
        assert self._client is not None
        payload: dict[str, Any] = {"text": text, "source": "benchmark"}
        if metadata:
            context = metadata.get("context")
            if context:
                payload["context"] = context
        response = self._client.post("/store", json=payload)
        response.raise_for_status()

    def recall(self, query: str, *, k: int = DEFAULT_K) -> list[Document]:
        """Single POST /recall. Daemon ranking returned verbatim."""
        assert self._client is not None
        payload = {
            "query": query,
            "budget_tokens": DEFAULT_BUDGET_TOKENS,
            "k": k,
        }
        response = self._client.post("/recall", json=payload)
        response.raise_for_status()
        data = response.json()
        return [self._to_document(hit) for hit in data.get("hits", [])]

    def _auth_headers(self) -> dict[str, str]:
        headers = {"Content-Type": "application/json", "X-Cortex-Request": "true"}
        if self._token:
            headers["Authorization"] = f"Bearer {self._token}"
        return headers

    @staticmethod
    def _to_document(hit: dict[str, Any]) -> Document:
        return Document(
            id=str(hit.get("id", "")),
            text=hit.get("text", "") or hit.get("excerpt", ""),
            score=float(hit.get("score", 0.0)),
            metadata=hit.get("metadata") or {},
        )

    @staticmethod
    def _forbid_helper_env_vars() -> None:
        """Fail fast if any known helper env var is set during a pure run.

        Pure mode measures core daemon quality; helper env vars inject
        adapter-side tuning that by definition lives outside the daemon.
        Enforcing this at initialize() time catches accidental leakage
        into the canonical baseline.
        """
        forbidden_prefixes = (
            "CORTEX_BENCHMARK_",
            "CORTEX_HELPER_",
            "CORTEX_RERANK_",
            "CORTEX_EXPAND_",
            "CORTEX_LONGMEMEVAL_",
        )
        violations = [
            key
            for key in os.environ
            if any(key.startswith(prefix) for prefix in forbidden_prefixes)
        ]
        if violations:
            raise RuntimeError(
                "cortex-http-pure detected helper env vars: "
                f"{', '.join(violations)}. Pure runs forbid all tuning flags. "
                "Unset them or use the cortex-http (tuned) adapter instead."
            )

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import httpx
from memory_bench.memory.base import MemoryProvider
from memory_bench.models import Document


class CortexHTTPPureMemoryProvider(MemoryProvider):
    name = "cortex-http-pure"
    description = "Pure Cortex HTTP passthrough. No adapter-side tuning."
    kind = "local"
    provider = "cortex"
    variant = "http-pure"
    concurrency = 1

    def __init__(self) -> None:
        base = (os.environ.get("CORTEX_BASE_URL") or "").strip()
        if not base:
            raise RuntimeError("CORTEX_BASE_URL is required")
        self._base = base.rstrip("/")
        self._budget = max(1, int(os.environ.get("CORTEX_RECALL_BUDGET", "200")))
        self._agent = os.environ.get("CORTEX_SOURCE_AGENT", "amb-cortex::pure")
        self._client: httpx.Client | None = None

    def initialize(self) -> None:
        self._client = httpx.Client(timeout=30.0)

    def cleanup(self) -> None:
        if self._client is not None:
            self._client.close()
            self._client = None

    def prepare(self, store_dir: Path, unit_ids: set[str] | None = None, reset: bool = True) -> None:
        return None

    def ingest(self, documents: list[Document]) -> None:
        assert self._client is not None
        for doc in documents:
            payload: dict[str, Any] = {
                "decision": doc.content,
                "context": doc.context or doc.id,
                "type": "decision",
                "confidence": 1.0,
            }
            response = self._client.post(
                f"{self._base}/store",
                json=payload,
                headers=self._headers(),
            )
            response.raise_for_status()

    def retrieve(
        self,
        query: str,
        k: int = 10,
        user_id: str | None = None,
        query_timestamp: str | None = None,
    ) -> tuple[list[Document], dict | None]:
        _ = (user_id, query_timestamp)
        assert self._client is not None
        params = {"q": query, "k": str(max(1, k)), "budget": str(self._budget)}
        response = self._client.get(
            f"{self._base}/recall",
            params=params,
            headers=self._headers(),
        )
        response.raise_for_status()
        payload = response.json()
        documents: list[Document] = []
        for item in payload.get("results") or []:
            if not isinstance(item, dict):
                continue
            excerpt = str(item.get("excerpt") or item.get("text") or "")
            source = str(item.get("source") or f"recall-{len(documents)}")
            documents.append(
                Document(
                    id=source,
                    content=excerpt,
                    user_id=user_id,
                )
            )
        return documents[:k], payload

    def _headers(self) -> dict[str, str]:
        headers = {"X-Cortex-Request": "true", "X-Source-Agent": self._agent}
        token = (os.environ.get("CORTEX_AUTH_TOKEN") or "").strip()
        if not token:
            token_file = Path(os.environ.get("CORTEX_TOKEN_FILE", ""))
            if token_file.exists():
                token = token_file.read_text(encoding="utf-8").strip()
        if token:
            headers["Authorization"] = f"Bearer {token}"
        return headers

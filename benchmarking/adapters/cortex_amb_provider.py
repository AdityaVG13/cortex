from __future__ import annotations

import os
import re
from pathlib import Path

from cortex_http_client import CortexHTTPClient, CortexStoredDocument
from cortex_http_types import RecallResponse
from memory_bench.memory.base import MemoryProvider
from memory_bench.models import Document


class CortexHTTPMemoryProvider(MemoryProvider):
    name = "cortex-http"
    description = (
        "Cortex HTTP provider for AMB. Intended for isolated benchmark daemons so "
        "benchmark corpora do not mix with real user memory."
    )
    kind = "local"
    provider = "cortex"
    variant = "http"
    # Keep benchmark request pressure deterministic and fair against local daemon
    # limits; callers can opt up for stress runs.
    concurrency = max(1, int(os.environ.get("CORTEX_BENCHMARK_PROVIDER_CONCURRENCY", "1")))

    def __init__(self) -> None:
        self._http = CortexHTTPClient()
        self._store_dir: Path | None = None
        self._pending_docs: list[CortexStoredDocument] = []
        self._flush_batch_size = max(1, int(os.environ.get("CORTEX_BENCHMARK_INGEST_FLUSH_SIZE", "200")))

    def initialize(self) -> None:
        self._http.healthcheck()

    def cleanup(self) -> None:
        self._flush_pending(force=True)
        self._http.close()

    def prepare(self, store_dir: Path, unit_ids: set[str] | None = None, reset: bool = True) -> None:
        self._store_dir = store_dir
        if reset:
            if "CORTEX_BENCHMARK_NAMESPACE" not in os.environ:
                self._http.reset_namespace(str(store_dir))
            else:
                self._http.reset_namespace(os.environ["CORTEX_BENCHMARK_NAMESPACE"])

    def ingest(self, documents: list[Document]) -> None:
        self._pending_docs.extend(
            CortexStoredDocument(
                id=document.id,
                content=document.content,
                user_id=document.user_id,
                timestamp=document.timestamp,
                context=document.context,
            )
            for document in documents
        )
        self._flush_pending(force=False)

    def _flush_pending(self, force: bool) -> None:
        if not self._pending_docs:
            return
        if force:
            while self._pending_docs:
                chunk = self._pending_docs[: self._flush_batch_size]
                del self._pending_docs[: self._flush_batch_size]
                self._http.store_documents(chunk)
            return
        while len(self._pending_docs) >= self._flush_batch_size:
            chunk = self._pending_docs[: self._flush_batch_size]
            del self._pending_docs[: self._flush_batch_size]
            self._http.store_documents(chunk)

    def retrieve(
        self,
        query: str,
        k: int = 10,
        user_id: str | None = None,
        query_timestamp: str | None = None,
    ) -> tuple[list[Document], RecallResponse]:
        self._flush_pending(force=True)
        stored_docs, payload = self._http.recall_documents(query, k=k, user_id=user_id)
        documents = [
            Document(
                id=document.id,
                content=document.content,
                user_id=document.user_id,
                timestamp=document.timestamp,
                context=document.context,
            )
            for document in stored_docs
        ]
        return documents, payload

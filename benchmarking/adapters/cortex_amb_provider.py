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
    concurrency = 4

    def __init__(self) -> None:
        self._http = CortexHTTPClient()
        self._store_dir: Path | None = None

    def initialize(self) -> None:
        self._http.healthcheck()

    def cleanup(self) -> None:
        self._http.close()

    def prepare(self, store_dir: Path, unit_ids: set[str] | None = None, reset: bool = True) -> None:
        self._store_dir = store_dir
        if reset:
            if "CORTEX_BENCHMARK_NAMESPACE" not in os.environ:
                self._http.reset_namespace(str(store_dir))
            else:
                self._http.reset_namespace(os.environ["CORTEX_BENCHMARK_NAMESPACE"])

    def ingest(self, documents: list[Document]) -> None:
        self._http.store_documents(
            [
                CortexStoredDocument(
                    id=document.id,
                    content=document.content,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=document.context,
                )
                for document in documents
            ]
        )

    def retrieve(
        self,
        query: str,
        k: int = 10,
        user_id: str | None = None,
        query_timestamp: str | None = None,
    ) -> tuple[list[Document], RecallResponse]:
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

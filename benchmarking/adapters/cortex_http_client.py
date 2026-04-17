from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass
from pathlib import Path
from typing import cast

import httpx

from cortex_http_types import HealthResponse, RecallResponse


def slugify(value: str) -> str:
    slug = re.sub(r"[^a-zA-Z0-9._-]+", "-", value.strip().lower()).strip("-")
    return slug or "default"


@dataclass
class CortexStoredDocument:
    id: str
    content: str
    user_id: str | None = None
    timestamp: str | None = None
    context: str | None = None


class CortexHTTPClient:
    def __init__(self) -> None:
        self.base_url = os.environ.get("CORTEX_BASE_URL", "http://127.0.0.1:7437").rstrip("/")
        self.token = self._resolve_token()
        self.timeout = float(os.environ.get("CORTEX_TIMEOUT_SECONDS", "30"))
        # Keep benchmark runs honest by defaulting retrieval context budget to 300 tokens.
        self.budget = int(os.environ.get("CORTEX_RECALL_BUDGET", "300"))
        self.source_agent = os.environ.get("CORTEX_SOURCE_AGENT", "amb-cortex")
        self.entry_type = os.environ.get("CORTEX_STORE_TYPE", "benchmark")
        self.namespace = slugify(os.environ.get("CORTEX_BENCHMARK_NAMESPACE", "amb"))
        self.metrics_file = os.environ.get("CORTEX_BENCHMARK_METRICS_FILE")
        self.client = httpx.Client(timeout=self.timeout)
        self.docs_by_context: dict[str, CortexStoredDocument] = {}

    def close(self) -> None:
        self.client.close()

    def healthcheck(self) -> HealthResponse:
        return cast(HealthResponse, self.request("GET", "/health", auth_required=False))

    def reset_namespace(self, namespace: str) -> None:
        self.namespace = slugify(namespace)
        self.docs_by_context.clear()

    def store_documents(self, documents: list[CortexStoredDocument]) -> None:
        for document in documents:
            context_key = self.context_key(document.id, document.user_id)
            self.docs_by_context[context_key] = document
            self.request(
                "POST",
                "/store",
                json={
                    "decision": self.serialize_document(document),
                    "context": context_key,
                    "type": self.entry_type,
                    "confidence": 1.0,
                },
            )

    def recall_documents(
        self,
        query: str,
        *,
        k: int = 10,
        user_id: str | None = None,
    ) -> tuple[list[CortexStoredDocument], RecallResponse]:
        raw_k = max(k, 10)
        if user_id is not None:
            raw_k = max(raw_k * 5, 25)
        params = {"q": query, "k": str(raw_k), "budget": str(self.budget)}
        # Keep benchmark runs isolated on shared app daemons.
        if self.namespace:
            params["source_prefix"] = f"amb::{self.namespace}::"
        payload = cast(
            RecallResponse,
            self.request(
                "GET",
                "/recall",
                params=params,
            ),
        )
        self._record_recall_metrics(query, payload)
        documents: list[CortexStoredDocument] = []
        seen_ids: set[str] = set()
        for result in payload.get("results") or []:
            source = result.get("source", "")
            document = self.docs_by_context.get(source)
            if document is None:
                excerpt = result.get("excerpt", "")
                if not excerpt:
                    continue
                document = CortexStoredDocument(
                    id=source or f"recall-{len(documents)}",
                    content=excerpt,
                    user_id=user_id,
                )
            if user_id is not None and document.user_id != user_id:
                continue
            if document.id in seen_ids:
                continue
            seen_ids.add(document.id)
            documents.append(document)
            if len(documents) >= k:
                break
        return documents, payload

    def _record_recall_metrics(self, query: str, payload: RecallResponse) -> None:
        if not self.metrics_file:
            return
        path = Path(self.metrics_file)
        path.parent.mkdir(parents=True, exist_ok=True)
        results = payload.get("results") or []
        token_estimate = 0
        if isinstance(results, list):
            token_estimate = sum(
                int(item.get("tokens", 0))
                for item in results
                if isinstance(item, dict)
            )
        record = {
            "query": query,
            "budget": self.budget,
            "result_count": len(results) if isinstance(results, list) else 0,
            "token_estimate": token_estimate,
        }
        with path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, ensure_ascii=True))
            handle.write("\n")

    def request(
        self,
        method: str,
        path: str,
        *,
        auth_required: bool = True,
        **kwargs: object,
    ) -> dict[str, object]:
        response = self.client.request(
            method,
            f"{self.base_url}{path}",
            headers=self.headers(auth_required=auth_required),
            **kwargs,
        )
        response.raise_for_status()
        if not response.content:
            return {}
        return response.json()

    def headers(self, *, auth_required: bool = True) -> dict[str, str]:
        headers = {
            "X-Cortex-Request": "true",
            "X-Source-Agent": self.source_agent,
        }
        if auth_required:
            headers["Authorization"] = f"Bearer {self.token}"
        return headers

    def context_key(self, doc_id: str, user_id: str | None) -> str:
        if user_id:
            return f"amb::{self.namespace}::user::{user_id}::doc::{doc_id}"
        return f"amb::{self.namespace}::doc::{doc_id}"

    def serialize_document(self, document: CortexStoredDocument) -> str:
        parts: list[str] = []
        if document.timestamp:
            parts.append(f"[timestamp] {document.timestamp}")
        if document.user_id:
            parts.append(f"[user] {document.user_id}")
        if document.context:
            parts.append(f"[context] {document.context}")
        parts.append(document.content)
        return "\n".join(part for part in parts if part)

    def _resolve_token(self) -> str:
        env_token = os.environ.get("CORTEX_AUTH_TOKEN")
        if env_token:
            return env_token.strip()

        token_file = os.environ.get("CORTEX_TOKEN_FILE")
        if token_file and Path(token_file).exists():
            token = Path(token_file).read_text(encoding="utf-8").strip()
            if token:
                return token

        default_token = Path.home() / ".cortex" / "cortex.token"
        if default_token.exists():
            token = default_token.read_text(encoding="utf-8").strip()
            if token:
                return token

        raise RuntimeError(
            "Unable to resolve Cortex auth token. Set CORTEX_AUTH_TOKEN or CORTEX_TOKEN_FILE."
        )

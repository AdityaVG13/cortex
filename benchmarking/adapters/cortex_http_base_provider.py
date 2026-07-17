"""Direct Cortex HTTP memory provider for AMB (base adapter)."""
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


class CortexHTTPBaseMemoryProvider(CortexHTTPBaseIngestMixin, CortexHTTPBaseRecallMixin, MemoryProvider):
    name = "cortex-http-base"
    description = (
        "Direct Cortex HTTP provider for AMB. Uses raw /store and /recall calls "
        "without helper-client multi-call query variants."
    )
    kind = "local"
    provider = "cortex"
    variant = "http-base"
    concurrency = max(1, int(os.environ.get("CORTEX_BENCHMARK_PROVIDER_CONCURRENCY", "1")))

    def __init__(self) -> None:
        base_url = (os.environ.get("CORTEX_BASE_URL") or "").strip()
        if not base_url:
            raise RuntimeError("CORTEX_BASE_URL is required for cortex-http-base provider")
        self.base_url = base_url.rstrip("/")
        self.timeout = float(os.environ.get("CORTEX_BENCHMARK_HTTP_TIMEOUT", "15.0"))
        self.max_retries = max(0, int(os.environ.get("CORTEX_BENCHMARK_HTTP_RETRIES", "2")))
        self.entry_type = os.environ.get("CORTEX_ENTRY_TYPE", "decision")
        self.source_agent = os.environ.get(
            "CORTEX_SOURCE_AGENT",
            "amb-cortex::provider-base",
        )
        self.namespace = slugify(os.environ.get("CORTEX_BENCHMARK_NAMESPACE", "default"))
        self.budget = max(1, int(os.environ.get("CORTEX_RECALL_BUDGET", "300")))
        self.metrics_file = os.environ.get("CORTEX_BENCHMARK_METRICS_FILE", "")
        self.recall_fanout_multiplier = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "6")),
        )
        self.recall_fanout_min = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "60")),
        )
        self.detail_siblings_per_seed = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_BASE_DETAIL_SIBLINGS_PER_SEED", "2")),
        )
        self.detail_max_added_siblings = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_BASE_DETAIL_MAX_ADDED_SIBLINGS", "10")),
        )
        self.detail_sibling_score_margin = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_BASE_DETAIL_SIBLING_SCORE_MARGIN", "16")),
        )
        self.enable_fact_extracts = os.environ.get(
            "CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS",
            "1",
        ).strip().lower() not in {"0", "false", "no"}
        self.store_full_docs = os.environ.get(
            "CORTEX_BENCHMARK_STORE_FULL_DOCS",
            "1",
        ).strip().lower() not in {"0", "false", "no"}
        requested_fact_extracts = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "12")),
        )
        base_fact_extract_cap = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_BASE_MAX_FACT_EXTRACTS_PER_DOC", "12")),
        )
        self.max_fact_extracts_per_doc = min(requested_fact_extracts, base_fact_extract_cap)
        self.fact_extract_max_chars = max(
            120,
            int(os.environ.get("CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS", "640")),
        )
        self.include_assistant_fact_extracts = os.environ.get(
            "CORTEX_BENCHMARK_INCLUDE_ASSISTANT_FACT_EXTRACTS",
            "0",
        ).strip().lower() in {"1", "true", "yes", "on"}
        self.short_reply_question_max_chars = max(
            48,
            int(os.environ.get("CORTEX_BENCHMARK_SHORT_REPLY_QUESTION_MAX_CHARS", "180")),
        )
        self.store_max_chars = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_STORE_MAX_CHARS", "12000")),
        )
        self.prefer_recall_excerpt = os.environ.get(
            "CORTEX_BENCHMARK_BASE_USE_RECALL_EXCERPT",
            "0",
        ).strip().lower() in {"1", "true", "yes", "on"}
        self.dedupe_identical_store_payloads = os.environ.get(
            "CORTEX_BENCHMARK_DEDUP_IDENTICAL_STORE_PAYLOADS",
            "1",
        ).strip().lower() in {"1", "true", "yes", "on"}
        self.client = httpx.Client(timeout=self.timeout)
        self.token = self._resolve_token()
        self.docs_by_context: dict[str, Document] = {}
        self._serialized_by_context: dict[str, str] = {}
        self._stored_content_digests: set[str] = set()

    def initialize(self) -> None:
        _ = cast(HealthResponse, self._request("GET", "/health", auth_required=False))

    def cleanup(self) -> None:
        self.client.close()

    def prepare(self, store_dir: Path, unit_ids: set[str] | None = None, reset: bool = True) -> None:
        _ = (store_dir, unit_ids)
        if reset:
            self.docs_by_context.clear()
            self._serialized_by_context.clear()
            self._stored_content_digests.clear()
        namespace = os.environ.get("CORTEX_BENCHMARK_NAMESPACE")
        if namespace:
            self.namespace = slugify(namespace)

    def _request(
        self,
        method: str,
        path: str,
        *,
        auth_required: bool = True,
        **kwargs: object,
    ) -> dict[str, object]:
        url = f"{self.base_url}{path}"
        retryable_statuses = {429, 502, 503, 504}
        headers = self._headers(auth_required=auth_required)
        for attempt in range(self.max_retries + 1):
            try:
                response = self.client.request(method, url, headers=headers, **kwargs)
            except httpx.RequestError:
                if attempt >= self.max_retries:
                    raise
                time.sleep(0.1 * (attempt + 1))
                continue
            if response.status_code in retryable_statuses and attempt < self.max_retries:
                time.sleep(0.1 * (attempt + 1))
                continue
            response.raise_for_status()
            if not response.content:
                return {}
            return cast(dict[str, object], response.json())
        raise RuntimeError(f"request retry loop exhausted for {method} {url}")

    def _headers(self, *, auth_required: bool) -> dict[str, str]:
        headers = {
            "X-Cortex-Request": "true",
            "X-Source-Agent": self.source_agent,
        }
        if auth_required:
            headers["Authorization"] = f"Bearer {self.token}"
        return headers

    def _resolve_token(self) -> str:
        token = (os.environ.get("CORTEX_AUTH_TOKEN") or "").strip()
        if token:
            return token
        token_file = Path(os.environ.get("CORTEX_TOKEN_FILE", ""))
        if token_file.exists():
            value = token_file.read_text(encoding="utf-8").strip()
            if value:
                return value
        raise RuntimeError("CORTEX_AUTH_TOKEN or CORTEX_TOKEN_FILE is required")

    def _source_prefix(self, user_id: str | None) -> str:
        if not self.namespace:
            return ""
        if user_id:
            return f"amb::{self.namespace}::user::{user_id}::"
        return f"amb::{self.namespace}::"

    def _context_key(self, doc_id: str, user_id: str | None) -> str:
        if user_id:
            return f"amb::{self.namespace}::user::{user_id}::doc::{doc_id}"
        return f"amb::{self.namespace}::doc::{doc_id}"

    def _serialize_document(self, document: Document) -> str:
        parts: list[str] = []
        timestamp = self._as_text(document.timestamp).strip()
        user_id = self._as_text(document.user_id).strip()
        context = self._as_text(document.context).strip()
        if timestamp:
            parts.append(f"[timestamp] {timestamp}")
        if user_id:
            parts.append(f"[user] {user_id}")
        if context:
            parts.append(f"[context] {context}")
        content = self._as_text(document.content).strip()
        if content:
            parts.append(content)
        return "\n".join(parts)

    def _as_text(value: object | None) -> str:
        if value is None:
            return ""
        if isinstance(value, str):
            return value
        return str(value)

    @staticmethod
    def _split_source_keys(source: str) -> list[str]:
        if not source:
            return []
        parts = re.split(r"(?:\r?\n|\s*,\s*)+", source)
        keys: list[str] = []
        seen: set[str] = set()
        for part in parts:
            key = part.strip()
            if not key or key in seen:
                continue
            seen.add(key)
            keys.append(key)
        return keys

    def _sample_sources(self, results: list[object]) -> list[str]:
        sampled: list[str] = []
        for item in results:
            if not isinstance(item, dict):
                continue
            for source_key in self._split_source_keys(self._as_text(item.get("source"))):
                sampled.append(source_key)
                if len(sampled) >= 3:
                    return sampled
        return sampled

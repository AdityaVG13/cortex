"""Tuned Cortex HTTP client for AMB benchmark runs."""
from __future__ import annotations

import json
import os
import re
import time
from hashlib import sha1
from pathlib import Path
from typing import cast

import httpx

from cortex_http_client_recall import CortexHTTPClientRecallMixin
from cortex_http_client_scoring import CortexHTTPClientScoringMixin
from cortex_http_models import CortexStoredDocument
from cortex_http_types import HealthResponse, RecallResponse
from recall_tuning.client_patterns import *  # noqa: F401,F403
from recall_tuning.slugify import slugify

__all__ = ["CortexHTTPClient", "CortexStoredDocument"]


class CortexHTTPClient(CortexHTTPClientRecallMixin, CortexHTTPClientScoringMixin):
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
        self.max_retries = max(0, int(os.environ.get("CORTEX_BENCHMARK_HTTP_MAX_RETRIES", "6")))
        self.retry_base_seconds = max(
            0.05,
            float(os.environ.get("CORTEX_BENCHMARK_HTTP_RETRY_BASE_SECONDS", "0.25")),
        )
        self.retry_max_seconds = max(
            self.retry_base_seconds,
            float(os.environ.get("CORTEX_BENCHMARK_HTTP_RETRY_MAX_SECONDS", "3.0")),
        )
        self.dedupe_identical_store_payloads = os.environ.get(
            "CORTEX_BENCHMARK_DEDUP_IDENTICAL_STORE_PAYLOADS",
            "1",
        ).strip().lower() in {"1", "true", "yes", "on"}
        self.max_context_chars = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_CONTEXT_MAX_CHARS", "700")),
        )
        self.mcq_context_max_chars = max(
            self.max_context_chars,
            int(os.environ.get("CORTEX_BENCHMARK_MCQ_CONTEXT_MAX_CHARS", "980")),
        )
        self.retrieval_policy = self._normalize_retrieval_policy(
            os.environ.get("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "standard")
        )
        self.query_window_chars = max(
            80,
            int(os.environ.get("CORTEX_BENCHMARK_QUERY_WINDOW_CHARS", "240")),
        )
        self.max_query_windows_per_term = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_MAX_QUERY_WINDOWS_PER_TERM", "3")),
        )
        self.prefer_recall_excerpts = os.environ.get(
            "CORTEX_BENCHMARK_USE_RECALL_EXCERPTS",
            "1",
        ).strip().lower() not in {"0", "false", "no"}
        self.enable_detail_query_variants = os.environ.get(
            "CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS",
            "0",
        ).strip().lower() in {"1", "true", "yes", "on"}
        self.detail_query_variant_budget_ratio = min(
            0.8,
            max(
                0.1,
                float(os.environ.get("CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO", "0.35")),
            ),
        )
        self.detail_query_variant_min_budget = max(
            32,
            int(os.environ.get("CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET", "96")),
        )
        self.user_recall_fanout_multiplier = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_USER_RECALL_FANOUT_MULTIPLIER", "8")),
        )
        self.user_recall_fanout_min = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_USER_RECALL_FANOUT_MIN", "60")),
        )
        self.detail_recall_fanout_multiplier = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MULTIPLIER", "12")),
        )
        self.detail_recall_fanout_min = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_DETAIL_RECALL_FANOUT_MIN", "120")),
        )
        self.detail_siblings_per_seed = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_DETAIL_SIBLINGS_PER_SEED", "2")),
        )
        self.detail_max_added_siblings = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_DETAIL_MAX_ADDED_SIBLINGS", "10")),
        )
        self.detail_sibling_score_margin = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_DETAIL_SIBLING_SCORE_MARGIN", "18")),
        )
        self.answer_source_penalty = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_ANSWER_SOURCE_PENALTY", "22")),
        )
        self.client = httpx.Client(timeout=self.timeout)
        self.docs_by_context: dict[str, CortexStoredDocument] = {}
        self._serialized_by_context: dict[str, str] = {}
        self._content_digest_by_context: dict[str, str] = {}
        self._stored_content_digests: set[str] = set()

    def close(self) -> None:
        self.client.close()

    def healthcheck(self) -> HealthResponse:
        return cast(HealthResponse, self.request("GET", "/health", auth_required=False))

    def reset_namespace(self, namespace: str) -> None:
        self.namespace = slugify(namespace)
        self.docs_by_context.clear()
        self._serialized_by_context.clear()
        self._content_digest_by_context.clear()
        self._stored_content_digests.clear()

    def store_documents(self, documents: list[CortexStoredDocument]) -> None:
        for document in documents:
            normalized = self._normalize_document(document)
            context_key = self.context_key(normalized.id, normalized.user_id)
            serialized = self.serialize_document(normalized)
            digest = sha1(serialized.encode("utf-8")).hexdigest()
            if self._serialized_by_context.get(context_key) == serialized:
                continue
            self.docs_by_context[context_key] = normalized
            self._serialized_by_context[context_key] = serialized
            self._content_digest_by_context[context_key] = digest
            if (
                self.dedupe_identical_store_payloads
                and digest in self._stored_content_digests
            ):
                continue
            self.request(
                "POST",
                "/store",
                json={
                    "decision": serialized,
                    "context": context_key,
                    "type": self.entry_type,
                    "confidence": 1.0,
                },
            )
            self._stored_content_digests.add(digest)

    def recall_documents(
        self,
        query: str,
        *,
        k: int = 10,
        user_id: str | None = None,
    ) -> tuple[list[CortexStoredDocument], RecallResponse]:
        query_profile = self._build_query_profile(query)
        raw_k = max(k, 10)
        if user_id is not None:
            fanout_multiplier = self.user_recall_fanout_multiplier
            fanout_min = self.user_recall_fanout_min
            if bool(query_profile["is_detail_query"]):
                fanout_multiplier = max(fanout_multiplier, self.detail_recall_fanout_multiplier)
                fanout_min = max(fanout_min, self.detail_recall_fanout_min)
            raw_k = max(raw_k * fanout_multiplier, fanout_min)
        source_prefix = ""
        # Keep benchmark runs isolated on shared app daemons.
        if self.namespace:
            source_prefix = f"amb::{self.namespace}::"
            if user_id is not None:
                source_prefix = f"amb::{self.namespace}::user::{user_id}::"
        recall_calls: list[dict[str, object]] = []
        call_plan = self._build_recall_call_plan(
            query,
            query_profile=query_profile,
        )
        primary_call_payload: RecallResponse | None = None
        for call_query, call_budget, call_tag in call_plan:
            if (
                call_tag == "detail-variant"
                and primary_call_payload is not None
                and not self._should_run_detail_variant(primary_call_payload, query_profile=query_profile)
            ):
                continue
            params = {
                "q": call_query,
                "k": str(raw_k),
                "budget": str(call_budget),
            }
            if source_prefix:
                params["source_prefix"] = source_prefix
            call_payload = cast(
                RecallResponse,
                self.request(
                    "GET",
                    "/recall",
                    params=params,
                ),
            )
            results = call_payload.get("results")
            call_token_estimate = 0
            if isinstance(results, list):
                call_token_estimate = sum(
                    int(item.get("tokens", 0))
                    for item in results
                    if isinstance(item, dict)
                )
            recall_calls.append(
                {
                    "tag": call_tag,
                    "query": call_query,
                    "budget": int(call_budget),
                    "payload": call_payload,
                    "token_estimate": call_token_estimate,
                    "result_count": len(results) if isinstance(results, list) else 0,
                }
            )
            if call_tag == "primary":
                primary_call_payload = call_payload
        payload = self._merge_recall_payloads(recall_calls)
        payload = self._filter_recall_payload_by_source_scope(
            payload,
            source_prefix=source_prefix or None,
        )
        self._record_recall_metrics(
            query,
            payload,
            user_id=user_id,
            source_prefix=source_prefix or None,
            recall_calls=recall_calls,
        )
        collected_documents: list[CortexStoredDocument] = []
        seen_sources: set[str] = set()
        for result_index, result in enumerate(payload.get("results") or []):
            source = result.get("source", "")
            source_key = self._normalize_text(source).strip() or f"recall-{len(collected_documents)}"
            if source_key in seen_sources:
                continue
            seen_sources.add(source_key)
            excerpt = self._normalize_text(result.get("excerpt", ""))
            document = self.docs_by_context.get(source)
            if document is None:
                if not excerpt:
                    continue
                document = CortexStoredDocument(
                    id=source_key,
                    content=self._clip_text_by_policy(
                        excerpt,
                        query_profile=query_profile,
                    ),
                    user_id=user_id,
                )
            else:
                content = self._build_query_context_text(
                    query=query,
                    full_content=document.content,
                    excerpt=excerpt if self.prefer_recall_excerpts else "",
                )
                document = CortexStoredDocument(
                    id=document.id,
                    content=content,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=document.context,
                )
            if user_id is not None and document.user_id != user_id:
                continue
            collected_documents.append(document)
        documents = collected_documents
        if bool(query_profile["is_detail_query"]):
            documents = self._expand_fact_family_candidates(
                query=query,
                query_profile=query_profile,
                documents=documents,
            )
        documents = self._rerank_documents(query, documents)
        if bool(query_profile["wants_location"]):
            documents = self._promote_location_family_complement(
                query=query,
                documents=documents,
                k=k,
            )
            documents = self._augment_abroad_location_qualifier(
                query=query,
                documents=documents,
            )
        return documents[:k], payload

    def _record_recall_metrics(
        self,
        query: str,
        payload: RecallResponse,
        *,
        user_id: str | None,
        source_prefix: str | None,
        recall_calls: list[dict[str, object]] | None = None,
    ) -> None:
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
        if recall_calls:
            token_estimate = sum(int(call.get("token_estimate", 0)) for call in recall_calls)
        sources = [
            self._normalize_text(item.get("source"))
            for item in results
            if isinstance(item, dict)
        ]
        recall_call_count = len(recall_calls) if recall_calls else 1
        recall_variant_queries: list[str] = []
        if recall_calls:
            for call in recall_calls:
                call_query = self._normalize_text(call.get("query")).strip()
                if call_query and call_query.lower() != query.lower():
                    recall_variant_queries.append(call_query)
        record = {
            "query": query,
            "user_id": user_id,
            "source_prefix": source_prefix,
            "budget": self.budget,
            "result_count": len(results) if isinstance(results, list) else 0,
            "token_estimate": token_estimate,
            "source_count": len(sources),
            "sample_sources": sources[:3],
            "recall_call_count": recall_call_count,
            "recall_variant_queries": recall_variant_queries,
            "combined_token_estimate": (
                token_estimate
                if not recall_calls
                else sum(
                    int(item.get("tokens", 0))
                    for item in results
                    if isinstance(item, dict)
                )
            ),
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
        url = f"{self.base_url}{path}"
        headers = self.headers(auth_required=auth_required)
        retryable_statuses = {429, 502, 503, 504}
        for attempt in range(self.max_retries + 1):
            try:
                response = self.client.request(
                    method,
                    url,
                    headers=headers,
                    **kwargs,
                )
            except httpx.RequestError:
                if attempt >= self.max_retries:
                    raise
                time.sleep(self._retry_sleep_seconds(attempt=attempt, response=None))
                continue
            if response.status_code in retryable_statuses and attempt < self.max_retries:
                time.sleep(self._retry_sleep_seconds(attempt=attempt, response=response))
                continue
            response.raise_for_status()
            if not response.content:
                return {}
            return response.json()
        raise RuntimeError(f"request retry loop exhausted for {method} {url}")

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
        timestamp = self._normalize_text(document.timestamp)
        user_id = self._normalize_text(document.user_id)
        context = self._normalize_text(document.context)
        if timestamp:
            parts.append(f"[timestamp] {timestamp}")
        if user_id:
            parts.append(f"[user] {user_id}")
        if context:
            parts.append(f"[context] {context}")
        content = self._normalize_text(document.content)
        if content:
            parts.append(content)
        return "\n".join(part for part in parts if part)

    def _normalize_text(self, value: object | None) -> str:
        if value is None:
            return ""
        if isinstance(value, str):
            return value
        return str(value)

    def _normalize_retrieval_policy(self, value: object | None) -> str:
        normalized = self._normalize_text(value).strip().lower()
        if normalized in {"high-detail", "detail-preserving"}:
            return "high-detail"
        return "standard"

    def _normalize_document(self, document: CortexStoredDocument) -> CortexStoredDocument:
        normalized_id = self._normalize_text(document.id).strip()
        if not normalized_id:
            raise ValueError("document id must be a non-empty string")
        normalized_content = self._normalize_text(document.content)
        return CortexStoredDocument(
            id=normalized_id,
            content=normalized_content,
            user_id=self._normalize_text(document.user_id) or None,
            timestamp=self._normalize_text(document.timestamp) or None,
            context=self._normalize_text(document.context) or None,
        )

    def _document_variant_priority(self, document_id: str) -> int:
        if "::fact::" in document_id:
            return 2
        if "::turn::" in document_id:
            return 1
        return 0
    def _retry_sleep_seconds(
        self,
        *,
        attempt: int,
        response: httpx.Response | None,
    ) -> float:
        sleep_seconds = min(self.retry_max_seconds, self.retry_base_seconds * (2**attempt))
        if response is None:
            return sleep_seconds
        retry_after_header = response.headers.get("Retry-After")
        if retry_after_header is None:
            return sleep_seconds
        try:
            retry_after = float(retry_after_header)
        except ValueError:
            return sleep_seconds
        if retry_after <= 0:
            return sleep_seconds
        return min(self.retry_max_seconds, max(sleep_seconds, retry_after))

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

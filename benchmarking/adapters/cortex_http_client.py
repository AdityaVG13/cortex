from __future__ import annotations

import json
import os
import re
import time
from dataclasses import dataclass
from pathlib import Path
from typing import cast

import httpx

from cortex_http_types import HealthResponse, RecallResponse

_QUERY_STOPWORDS = {
    "a",
    "an",
    "and",
    "are",
    "at",
    "be",
    "by",
    "did",
    "do",
    "does",
    "for",
    "from",
    "how",
    "i",
    "in",
    "is",
    "it",
    "my",
    "of",
    "on",
    "or",
    "the",
    "to",
    "was",
    "what",
    "when",
    "where",
    "who",
    "why",
    "with",
}

_QUERY_NUMERIC_INTENT_PATTERN = re.compile(
    r"\b(how long|how much|how many|when|speed|cost|price|date|year)\b|\d",
    re.IGNORECASE,
)
_QUERY_LOCATION_INTENT_PATTERN = re.compile(
    r"\b(where|location|city|state|country|live|located|from where|moved to|travel)\b",
    re.IGNORECASE,
)
_QUERY_DATE_INTENT_PATTERN = re.compile(
    r"\b(when|date|year|month|day|born|anniversary)\b",
    re.IGNORECASE,
)
_QUERY_BIRTHDAY_DESCRIPTOR_PATTERN = re.compile(
    r"\bbirthday\s+(gift|present|party|card|dinner|cake|trip|message|wishlist)\b",
    re.IGNORECASE,
)
_QUERY_SPEED_INTENT_PATTERN = re.compile(
    r"\b(speed|fast|faster|slow|mbps|gbps|download|upload|internet|bandwidth|latency)\b",
    re.IGNORECASE,
)
_QUERY_ITEM_INTENT_PATTERN = re.compile(
    r"\b(what.*(item|device|game|thing|play)|which.*(item|device|game|thing|play)|buy|bought|purchased|redeemed|ordered|gift|upgraded to)\b",
    re.IGNORECASE,
)
_QUERY_NAME_INTENT_PATTERN = re.compile(
    r"\b(name|called|last name|first name|old name|previous name)\b",
    re.IGNORECASE,
)
_QUERY_OCCUPATION_INTENT_PATTERN = re.compile(
    r"\b(occupation|job|career|profession|position|role|title|worked as|work as|previous work)\b",
    re.IGNORECASE,
)
_QUERY_PERSONAL_PATTERN = re.compile(r"\b(i|my|me)\b", re.IGNORECASE)
_DATE_DETAIL_PATTERN = re.compile(
    r"\b(?:19|20)\d{2}\b"
    r"|(?:\b(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\b)"
    r"|(?:\b\d{1,2}[/-]\d{1,2}(?:[/-]\d{2,4})?\b)",
    re.IGNORECASE,
)
_DATE_EXACT_DETAIL_PATTERN = re.compile(
    r"\b(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\s+\d{1,2}(?:st|nd|rd|th)?(?:,\s*(?:19|20)\d{2})?\b"
    r"|(?:\b\d{1,2}(?:st|nd|rd|th)?\s+of\s+(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\b)"
    r"|(?:\b(?:19|20)\d{2}[/-]\d{1,2}[/-]\d{1,2}\b)"
    r"|(?:\bvalentine(?:'s)? day\b)",
    re.IGNORECASE,
)
_LOCATION_DETAIL_PATTERN = re.compile(
    r"\b(?:in|at|from|to|near)\s+(?!the\b|a\b|an\b|my\b|our\b|your\b|this\b|that\b)"
    r"[A-Za-z][A-Za-z0-9'-]{2,}(?:\s+[A-Za-z][A-Za-z0-9'-]{2,}){0,2}\b",
    re.IGNORECASE,
)
_GENERIC_LOCATION_DETAIL_PATTERN = re.compile(
    r"\b(?:in|at|from|to|near)\s+(?:home|house|work|office|school|campus|online|here|there)\b",
    re.IGNORECASE,
)
_LOCATION_ABBREV_DETAIL_PATTERN = re.compile(
    r"\b(?:in|at|from|to|near)\s+(?:la|ny|sf|dc|uk|us|eu|uae)\b",
    re.IGNORECASE,
)
_STANDALONE_LOCATION_DETAIL_PATTERN = re.compile(
    r"^\s*(?:[A-Z][A-Za-z0-9'&.-]{1,}(?:\s+[A-Z][A-Za-z0-9'&.-]{1,}){0,3})\.?\s*$"
)
_SPEED_DETAIL_PATTERN = re.compile(
    r"\b\d+(?:\.\d+)?\s*(?:"
    r"kbps|mbps|gbps|tbps"
    r"|kbit(?:s)?(?:/| per )second"
    r"|megabit(?:s)?(?:/| per )second"
    r"|gigabit(?:s)?(?:/| per )second"
    r"|ms|milliseconds?|latency"
    r")\b",
    re.IGNORECASE,
)
_ITEM_DETAIL_PATTERN = re.compile(
    r"\b(?:bought|purchased|redeemed|ordered|upgraded to|picked up)\b"
    r"|\b[A-Z][A-Za-z0-9'-]+(?:\s+[A-Z][A-Za-z0-9'-]+){1,3}\b"
)
_NAME_DETAIL_PATTERN = re.compile(
    r"\b(?:name\s+(?:is|was)|called|old name was|last name(?:\s+was)?)\s+[A-Z][A-Za-z0-9'-]+(?:\s+[A-Z][A-Za-z0-9'-]+){0,2}\b",
    re.IGNORECASE,
)
_OCCUPATION_DETAIL_PATTERN = re.compile(
    r"\b(?:worked as|work as|occupation(?:\s+was)?|job(?:\s+was)?|career(?:\s+as)?|position(?:\s+as)?|profession(?:\s+as)?)\b"
    r"|\b(?:specialist|engineer|manager|analyst|developer|teacher|nurse|designer|consultant|coordinator)\b",
    re.IGNORECASE,
)
_ASSISTANT_ROLE_PATTERN = re.compile(r"\[assistant\]|\"role\"\s*:\s*\"assistant\"", re.IGNORECASE)
_ASSISTANT_MIRROR_FACT_PATTERN = re.compile(
    r"\b(?:you mentioned|you said|you told me|your\s+[a-z0-9_-]+\s+(?:is|was|are|were|takes|took|upgraded|bought|redeemed|moved|graduated))\b",
    re.IGNORECASE,
)
_LOW_SIGNAL_ASSISTANT_PATTERN = re.compile(
    r"\b(?:here are|tips?|recommendations?|you can|you should|remember to|step\s+\d+|let me know|if you'd like|happy to help|overall)\b",
    re.IGNORECASE,
)
_OVERLAP_TOKEN_PATTERN = re.compile(r"[a-z0-9]{3,}")
_ANSWER_SOURCE_ID_PATTERN = re.compile(
    r"(?:^|[:_])answer_[0-9a-f]{6,}(?:$|[:_])",
    re.IGNORECASE,
)


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
        self.max_retries = max(0, int(os.environ.get("CORTEX_BENCHMARK_HTTP_MAX_RETRIES", "6")))
        self.retry_base_seconds = max(
            0.05,
            float(os.environ.get("CORTEX_BENCHMARK_HTTP_RETRY_BASE_SECONDS", "0.25")),
        )
        self.retry_max_seconds = max(
            self.retry_base_seconds,
            float(os.environ.get("CORTEX_BENCHMARK_HTTP_RETRY_MAX_SECONDS", "3.0")),
        )
        self.max_context_chars = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_CONTEXT_MAX_CHARS", "700")),
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

    def close(self) -> None:
        self.client.close()

    def healthcheck(self) -> HealthResponse:
        return cast(HealthResponse, self.request("GET", "/health", auth_required=False))

    def reset_namespace(self, namespace: str) -> None:
        self.namespace = slugify(namespace)
        self.docs_by_context.clear()

    def store_documents(self, documents: list[CortexStoredDocument]) -> None:
        for document in documents:
            normalized = self._normalize_document(document)
            context_key = self.context_key(normalized.id, normalized.user_id)
            self.docs_by_context[context_key] = normalized
            self.request(
                "POST",
                "/store",
                json={
                    "decision": self.serialize_document(normalized),
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
        for call_query, call_budget, call_tag in self._build_recall_call_plan(
            query,
            query_profile=query_profile,
        ):
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
        payload = self._merge_recall_payloads(recall_calls)
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
                    content=self._clip_text(excerpt),
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

    def _build_recall_call_plan(
        self,
        query: str,
        *,
        query_profile: dict[str, bool | str],
    ) -> list[tuple[str, int, str]]:
        if not self.enable_detail_query_variants:
            return [(query, self.budget, "primary")]
        if not (
            bool(query_profile["wants_numbers"])
            or bool(query_profile["wants_location"])
            or bool(query_profile["wants_date"])
            or bool(query_profile["wants_speed"])
            or bool(query_profile["wants_item"])
            or bool(query_profile["wants_occupation"])
            or bool(query_profile["wants_name"])
        ):
            return [(query, self.budget, "primary")]
        variant_query = self._build_detail_query_variant(query, query_profile=query_profile)
        if not variant_query:
            return [(query, self.budget, "primary")]
        variant_budget = max(
            self.detail_query_variant_min_budget,
            int(round(self.budget * self.detail_query_variant_budget_ratio)),
        )
        variant_budget = min(self.budget - 1, variant_budget)
        if variant_budget <= 0:
            return [(query, self.budget, "primary")]
        primary_budget = self.budget - variant_budget
        if primary_budget <= 0:
            return [(query, self.budget, "primary")]
        return [
            (query, primary_budget, "primary"),
            (variant_query, variant_budget, "detail-variant"),
        ]

    def _build_detail_query_variant(
        self,
        query: str,
        *,
        query_profile: dict[str, bool | str],
    ) -> str | None:
        tokens = list(self._query_terms(query))
        tokens.sort()
        token_parts = tokens[:10]
        hint_parts: list[str] = []
        if bool(query_profile["wants_speed"]):
            hint_parts.extend(["internet", "plan", "speed", "upgraded", "mbps"])
        if bool(query_profile["wants_location"]):
            hint_parts.extend(["where", "location", "place", "city", "country", "abroad"])
        if bool(query_profile["wants_date"]):
            hint_parts.extend(["when", "date", "year", "month", "day", "exact date"])
        if bool(query_profile["wants_item"]):
            hint_parts.extend(["item", "bought", "purchased", "gift", "play title"])
        if bool(query_profile["wants_occupation"]):
            hint_parts.extend(["occupation", "job", "worked as", "career", "role", "position", "startup"])
        if bool(query_profile["wants_name"]):
            hint_parts.extend(["name", "last name", "first name", "called", "old name", "surname"])
        if bool(query_profile["wants_numbers"]) and not bool(query_profile["wants_speed"]):
            hint_parts.extend(["exact", "number", "value"])
        if bool(query_profile["is_detail_query"]):
            hint_parts.extend(["user answer", "assistant question", "exact detail"])

        merged_parts: list[str] = []
        seen: set[str] = set()
        for part in token_parts + hint_parts:
            normalized = part.strip().lower()
            if not normalized or normalized in seen:
                continue
            merged_parts.append(normalized)
            seen.add(normalized)
        if not merged_parts:
            return None
        variant = " ".join(merged_parts)
        if variant.strip().lower() == query.strip().lower():
            return None
        return variant

    def _detail_family_key(self, document_id: str) -> str:
        normalized_id = self._normalize_text(document_id).strip()
        if not normalized_id:
            return ""
        return re.sub(r"::fact::\d+$", "", normalized_id, flags=re.IGNORECASE)

    def _expand_fact_family_candidates(
        self,
        *,
        query: str,
        query_profile: dict[str, bool | str],
        documents: list[CortexStoredDocument],
    ) -> list[CortexStoredDocument]:
        if (
            not documents
            or not bool(query_profile["is_detail_query"])
            or self.detail_siblings_per_seed <= 0
            or self.detail_max_added_siblings <= 0
            or not self.docs_by_context
        ):
            return documents
        families = {
            self._detail_family_key(document.id)
            for document in documents
            if self._detail_family_key(document.id)
        }
        families.discard("")
        if not families:
            return documents
        sibling_pool: dict[str, list[CortexStoredDocument]] = {family: [] for family in families}
        for stored in self.docs_by_context.values():
            family = self._detail_family_key(stored.id)
            if family not in sibling_pool:
                continue
            if "::fact::" not in self._normalize_text(stored.id).lower():
                continue
            sibling_pool[family].append(stored)

        existing_ids = {self._normalize_text(document.id).lower() for document in documents}
        additions: list[CortexStoredDocument] = []
        added_count = 0
        for seed in documents:
            if added_count >= self.detail_max_added_siblings:
                break
            family = self._detail_family_key(seed.id)
            if family not in sibling_pool:
                continue
            seed_score = self._document_query_relevance_score(query, seed)
            seed_detail = self._detail_bonus(query_profile, seed.content)
            sibling_candidates: list[tuple[int, int, CortexStoredDocument]] = []
            for sibling in sibling_pool[family]:
                sibling_id_key = self._normalize_text(sibling.id).lower()
                if not sibling_id_key or sibling_id_key in existing_ids:
                    continue
                sibling_context = self._build_query_context_text(
                    query=query,
                    full_content=sibling.content,
                    excerpt="",
                )
                sibling_doc = CortexStoredDocument(
                    id=sibling.id,
                    content=sibling_context,
                    user_id=sibling.user_id,
                    timestamp=sibling.timestamp,
                    context=sibling.context,
                )
                sibling_score = self._document_query_relevance_score(query, sibling_doc)
                sibling_detail = self._detail_bonus(query_profile, sibling_doc.content)
                detail_is_stronger = sibling_detail > seed_detail
                if (
                    sibling_score < (seed_score - self.detail_sibling_score_margin)
                    and not detail_is_stronger
                ):
                    continue
                sibling_candidates.append((sibling_score, sibling_detail, sibling_doc))
            if not sibling_candidates:
                continue
            sibling_candidates.sort(reverse=True, key=lambda item: (item[0], item[1], item[2].id))
            for _score, _detail, sibling_doc in sibling_candidates[: self.detail_siblings_per_seed]:
                sibling_id_key = self._normalize_text(sibling_doc.id).lower()
                if not sibling_id_key or sibling_id_key in existing_ids:
                    continue
                additions.append(sibling_doc)
                existing_ids.add(sibling_id_key)
                added_count += 1
                if added_count >= self.detail_max_added_siblings:
                    break
        if not additions:
            return documents
        return documents + additions

    def _merge_recall_payloads(self, recall_calls: list[dict[str, object]]) -> RecallResponse:
        if not recall_calls:
            return cast(RecallResponse, {"results": [], "budget": self.budget, "spent": 0, "saved": 0})

        merged_results: list[dict[str, object]] = []
        seen_result_keys: set[str] = set()
        spent_total = 0
        budget_total = 0
        saved_total = 0
        call_summaries: list[dict[str, object]] = []

        for call in recall_calls:
            payload = cast(RecallResponse, call.get("payload") or {})
            call_results = payload.get("results")
            if isinstance(call_results, list):
                for item in call_results:
                    if not isinstance(item, dict):
                        continue
                    source = self._normalize_text(item.get("source")).strip()
                    excerpt = self._normalize_text(item.get("excerpt")).strip()
                    dedupe_key = f"{source}\n{excerpt[:240]}"
                    if dedupe_key in seen_result_keys:
                        continue
                    seen_result_keys.add(dedupe_key)
                    merged_results.append(item)
            call_budget = int(call.get("budget", 0) or 0)
            budget_total += call_budget
            spent_total += int(payload.get("spent", 0) or 0)
            saved_total += int(payload.get("saved", 0) or 0)
            call_summaries.append(
                {
                    "tag": self._normalize_text(call.get("tag")),
                    "query": self._normalize_text(call.get("query")),
                    "budget": call_budget,
                    "result_count": int(call.get("result_count", 0) or 0),
                    "token_estimate": int(call.get("token_estimate", 0) or 0),
                }
            )

        return cast(
            RecallResponse,
            {
                "results": merged_results,
                "budget": budget_total,
                "spent": spent_total,
                "saved": saved_total,
                "count": len(merged_results),
                "calls": call_summaries,
            },
        )

    def _rerank_documents(
        self,
        query: str,
        documents: list[CortexStoredDocument],
    ) -> list[CortexStoredDocument]:
        if len(documents) <= 1:
            return documents
        scored: list[tuple[int, int, CortexStoredDocument]] = []
        for idx, document in enumerate(documents):
            relevance_score = self._document_query_relevance_score(query, document)
            variant_bonus = self._document_variant_priority(document.id)
            score = relevance_score + variant_bonus
            scored.append((score, -idx, document))
        scored.sort(reverse=True, key=lambda item: (item[0], item[1]))
        return [item[2] for item in scored]

    def _document_query_relevance_score(
        self,
        query: str,
        document: CortexStoredDocument,
    ) -> int:
        query_terms = self._query_terms(query)
        profile = self._build_query_profile(query)
        normalized_context = self._normalize_text(document.context)
        text = f"{document.content}\n{normalized_context}".strip()
        lowered = text.lower()
        overlap = self._term_overlap_count(query_terms, lowered)
        phrase_bonus = 1 if profile["normalized_query"] and profile["normalized_query"] in lowered else 0
        detail_bonus = self._detail_bonus(profile, text)
        is_detail_query = bool(profile["is_detail_query"])
        is_user_anchored = self._is_user_anchored_document(document, lowered)
        personal_bonus = 3 if "\"role\": \"user\"" in lowered or "[user]" in lowered else 0
        personal_bonus += 2 if re.search(r"\b(i|my)\b", lowered) else 0
        detail_user_bonus = 8 if is_detail_query and is_user_anchored else 0
        detail_non_user_penalty = 8 if is_detail_query and not is_user_anchored else 0
        personal_query_penalty = (
            2
            if profile["is_personal_query"]
            and not re.search(r"\b(i|my|me|you|your)\b", lowered)
            else 0
        )
        source_adjustment = self._source_quality_adjustment(document.id)
        assistant_penalty = self._assistant_noise_penalty(text)
        location_penalty = self._location_specificity_penalty(profile, text)
        score = (overlap * 10) + (phrase_bonus * 4) + detail_bonus + personal_bonus + detail_user_bonus
        return (
            score
            + source_adjustment
            - assistant_penalty
            - personal_query_penalty
            - location_penalty
            - detail_non_user_penalty
        )

    def _query_terms(self, query: str) -> set[str]:
        tokens = set(re.findall(r"[a-z0-9]{3,}", query.lower()))
        return {token for token in tokens if token not in _QUERY_STOPWORDS}

    def _term_overlap_count(self, query_terms: set[str], text: str) -> int:
        text_tokens = set(_OVERLAP_TOKEN_PATTERN.findall(text.lower()))
        return sum(1 for term in query_terms if term in text_tokens)

    def _build_query_profile(self, query: str) -> dict[str, bool | str]:
        normalized_query = query.lower().strip()
        wants_numbers = bool(_QUERY_NUMERIC_INTENT_PATTERN.search(normalized_query))
        wants_location = bool(_QUERY_LOCATION_INTENT_PATTERN.search(normalized_query))
        wants_date = bool(_QUERY_DATE_INTENT_PATTERN.search(normalized_query))
        explicit_date_cue = bool(re.search(r"\b(when|date|year|month|day)\b", normalized_query))
        if wants_date and _QUERY_BIRTHDAY_DESCRIPTOR_PATTERN.search(normalized_query) and not explicit_date_cue:
            wants_date = False
        wants_speed = bool(_QUERY_SPEED_INTENT_PATTERN.search(normalized_query))
        wants_item = bool(_QUERY_ITEM_INTENT_PATTERN.search(normalized_query))
        wants_occupation = bool(_QUERY_OCCUPATION_INTENT_PATTERN.search(normalized_query))
        wants_name = bool(_QUERY_NAME_INTENT_PATTERN.search(normalized_query))
        return {
            "normalized_query": normalized_query,
            "wants_numbers": wants_numbers,
            "wants_location": wants_location,
            "wants_date": wants_date,
            "wants_speed": wants_speed,
            "wants_item": wants_item,
            "wants_occupation": wants_occupation,
            "wants_name": wants_name,
            "is_detail_query": wants_numbers
            or wants_location
            or wants_date
            or wants_speed
            or wants_item
            or wants_occupation
            or wants_name,
            "is_personal_query": bool(_QUERY_PERSONAL_PATTERN.search(normalized_query)),
        }

    def _text_has_date_detail(self, text: str) -> bool:
        return bool(_DATE_DETAIL_PATTERN.search(text))

    def _text_has_exact_date_detail(self, text: str) -> bool:
        return bool(_DATE_EXACT_DETAIL_PATTERN.search(text))

    def _text_has_location_detail(self, text: str) -> bool:
        return bool(
            _LOCATION_DETAIL_PATTERN.search(text)
            or _LOCATION_ABBREV_DETAIL_PATTERN.search(text)
            or _STANDALONE_LOCATION_DETAIL_PATTERN.fullmatch(text.strip())
        )

    def _text_has_generic_location_detail(self, text: str) -> bool:
        return bool(_GENERIC_LOCATION_DETAIL_PATTERN.search(text))

    def _text_has_speed_detail(self, text: str) -> bool:
        return bool(_SPEED_DETAIL_PATTERN.search(text))

    def _text_has_item_detail(self, text: str) -> bool:
        if _ITEM_DETAIL_PATTERN.search(text):
            return True
        answer_match = re.search(
            r"\[user-answer\]\s*([^\[\n]{2,120})",
            text,
            flags=re.IGNORECASE,
        )
        if not answer_match:
            return False
        answer_value = answer_match.group(1).strip(" \t\r\n.,!?;:\"'")
        if not answer_value:
            return False
        if len(answer_value.split()) > 8:
            return False
        if re.search(
            r"\b(?:unknown|unsure|not sure|don't know|cant remember|can't remember|something)\b",
            answer_value,
            flags=re.IGNORECASE,
        ):
            return False
        return bool(re.search(r"[a-z]", answer_value))

    def _text_has_name_detail(self, text: str) -> bool:
        return bool(_NAME_DETAIL_PATTERN.search(text))

    def _text_has_occupation_detail(self, text: str) -> bool:
        return bool(_OCCUPATION_DETAIL_PATTERN.search(text))

    def _detail_bonus(self, query_profile: dict[str, bool | str], text: str) -> int:
        score = 0
        if bool(query_profile["wants_numbers"]) and re.search(r"\d", text):
            score += 6
        if bool(query_profile["wants_date"]) and self._text_has_date_detail(text):
            score += 8
            if self._text_has_exact_date_detail(text):
                score += 5
        if bool(query_profile["wants_location"]) and self._text_has_location_detail(text):
            if self._text_has_generic_location_detail(text):
                score += 6
            else:
                score += 9
        if bool(query_profile["wants_speed"]) and self._text_has_speed_detail(text):
            score += 10
        if bool(query_profile["wants_item"]) and self._text_has_item_detail(text):
            score += 8
        if bool(query_profile["wants_occupation"]) and self._text_has_occupation_detail(text):
            score += 9
        if bool(query_profile["wants_name"]) and self._text_has_name_detail(text):
            score += 9
        return score

    def _source_quality_adjustment(self, document_id: str) -> int:
        lowered = self._normalize_text(document_id).lower()
        if not lowered:
            return 0
        adjustment = 0
        if "::fact::" in lowered:
            adjustment += 2
        if _ANSWER_SOURCE_ID_PATTERN.search(lowered):
            adjustment -= self.answer_source_penalty
        if "assistant" in lowered:
            adjustment -= 8
        return adjustment

    def _is_user_anchored_document(self, document: CortexStoredDocument, lowered_text: str) -> bool:
        lowered_id = self._normalize_text(document.id).lower()
        lowered_context = self._normalize_text(document.context).lower()
        return bool(
            "::user::" in lowered_id
            or "::user::" in lowered_context
            or "\"role\": \"user\"" in lowered_text
            or "[user]" in lowered_text
            or re.search(r"\b(i|my|me)\b", lowered_text)
        )

    def _location_specificity_penalty(self, query_profile: dict[str, bool | str], text: str) -> int:
        if not bool(query_profile["wants_location"]):
            return 0
        if self._text_has_generic_location_detail(text):
            return 4
        return 0

    def _location_detail_count(self, text: str) -> int:
        if not text:
            return 0
        count = len(_LOCATION_DETAIL_PATTERN.findall(text))
        count += len(_LOCATION_ABBREV_DETAIL_PATTERN.findall(text))
        if _STANDALONE_LOCATION_DETAIL_PATTERN.fullmatch(text.strip()):
            count += 1
        return count

    def _assistant_noise_penalty(self, text: str) -> int:
        lowered = text.lower()
        if not _ASSISTANT_ROLE_PATTERN.search(text):
            return 0
        if _ASSISTANT_MIRROR_FACT_PATTERN.search(lowered):
            return 0
        penalty = 7
        if _LOW_SIGNAL_ASSISTANT_PATTERN.search(lowered):
            penalty += 10
        return penalty

    def _needs_full_for_detail(
        self,
        query_profile: dict[str, bool | str],
        excerpt: str,
        full_content: str,
    ) -> bool:
        if bool(query_profile["wants_numbers"]) and not re.search(r"\d", excerpt) and re.search(r"\d", full_content):
            return True
        if (
            bool(query_profile["wants_date"])
            and not self._text_has_date_detail(excerpt)
            and self._text_has_date_detail(full_content)
        ):
            return True
        if (
            bool(query_profile["wants_date"])
            and not self._text_has_exact_date_detail(excerpt)
            and self._text_has_exact_date_detail(full_content)
        ):
            return True
        if (
            bool(query_profile["wants_location"])
            and not self._text_has_location_detail(excerpt)
            and self._text_has_location_detail(full_content)
        ):
            return True
        if (
            bool(query_profile["wants_location"])
            and self._text_has_generic_location_detail(excerpt)
            and self._text_has_location_detail(full_content)
            and not self._text_has_generic_location_detail(full_content)
        ):
            return True
        if (
            bool(query_profile["wants_location"])
            and self._text_has_location_detail(excerpt)
            and self._text_has_location_detail(full_content)
            and self._location_detail_count(full_content) > self._location_detail_count(excerpt)
        ):
            return True
        if (
            bool(query_profile["wants_speed"])
            and not self._text_has_speed_detail(excerpt)
            and self._text_has_speed_detail(full_content)
        ):
            return True
        if (
            bool(query_profile["wants_item"])
            and not self._text_has_item_detail(excerpt)
            and self._text_has_item_detail(full_content)
        ):
            return True
        if (
            bool(query_profile["wants_occupation"])
            and not self._text_has_occupation_detail(excerpt)
            and self._text_has_occupation_detail(full_content)
        ):
            return True
        if (
            bool(query_profile["wants_name"])
            and not self._text_has_name_detail(excerpt)
            and self._text_has_name_detail(full_content)
        ):
            return True
        return False

    def _context_candidate_score(
        self,
        query: str,
        query_terms: set[str],
        query_profile: dict[str, bool | str],
        candidate: str,
    ) -> int:
        lowered = candidate.lower()
        overlap = self._term_overlap_count(query_terms, lowered)
        phrase_bonus = 1 if query and query in lowered else 0
        detail_bonus = self._detail_bonus(query_profile, candidate)
        personal_bonus = 2 if "\"role\": \"user\"" in lowered or "[user]" in lowered else 0
        personal_bonus += 1 if re.search(r"\b(i|my)\b", lowered) else 0
        location_penalty = self._location_specificity_penalty(query_profile, candidate)
        return (
            (overlap * 10)
            + (phrase_bonus * 4)
            + detail_bonus
            + personal_bonus
            - self._assistant_noise_penalty(candidate)
            - location_penalty
        )

    def _build_query_context_text(
        self,
        *,
        query: str,
        full_content: str,
        excerpt: str,
    ) -> str:
        normalized_full = self._normalize_text(full_content)
        normalized_excerpt = self._normalize_text(excerpt)
        query_profile = self._build_query_profile(query)
        query_lower = str(query_profile["normalized_query"])
        query_terms = self._query_terms(query)

        if normalized_excerpt:
            if (
                not normalized_full
                or (
                    not self._needs_full_for_detail(query_profile, normalized_excerpt, normalized_full)
                    and self._context_candidate_score(
                        query_lower,
                        query_terms,
                        query_profile,
                        normalized_excerpt,
                    )
                    >= self._context_candidate_score(
                        query_lower,
                        query_terms,
                        query_profile,
                        normalized_full,
                    )
                )
            ):
                return self._clip_text(normalized_excerpt)
        if not normalized_full:
            return self._clip_text(normalized_excerpt)

        candidates: list[str] = []
        seen_candidates: set[str] = set()

        def add_candidate(value: str) -> None:
            text = self._normalize_text(value).strip()
            if not text:
                return
            dedupe_key = text[:220].lower()
            if dedupe_key in seen_candidates:
                return
            seen_candidates.add(dedupe_key)
            candidates.append(text)

        add_candidate(normalized_excerpt)
        if bool(query_profile["is_detail_query"]):
            for match in re.finditer(
                r"\[user-answer\]\s*([^\[\n]{1,220})",
                normalized_full,
                flags=re.IGNORECASE,
            ):
                add_candidate(f"[user-answer] {match.group(1).strip()}")
            for sentence in re.split(r"(?<=[.!?])\s+|\n+", normalized_full):
                sentence_text = sentence.strip()
                if len(sentence_text) < 12:
                    continue
                sentence_lower = sentence_text.lower()
                if query_terms and not any(term in sentence_lower for term in query_terms):
                    if self._detail_bonus(query_profile, sentence_text) <= 0:
                        continue
                add_candidate(sentence_text)

        if query_terms:
            haystack = normalized_full.lower()
            for term in sorted(query_terms):
                start = 0
                windows_added = 0
                while windows_added < self.max_query_windows_per_term:
                    idx = haystack.find(term, start)
                    if idx < 0:
                        break
                    left = max(0, idx - self.query_window_chars)
                    right = min(len(normalized_full), idx + len(term) + self.query_window_chars)
                    add_candidate(normalized_full[left:right])
                    start = idx + len(term)
                    windows_added += 1

        if not candidates:
            return self._clip_text(normalized_full)

        scored_candidates: list[tuple[int, int, str]] = []
        for idx, candidate in enumerate(candidates):
            score = self._context_candidate_score(query_lower, query_terms, query_profile, candidate)
            scored_candidates.append((score, -idx, candidate))

        scored_candidates.sort(reverse=True, key=lambda item: (item[0], item[1]))
        best = scored_candidates[0][2]
        return self._clip_text(best)

    def _clip_text(self, text: str) -> str:
        if self.max_context_chars <= 0 or len(text) <= self.max_context_chars:
            return text
        if self.max_context_chars <= 8:
            return text[: self.max_context_chars]
        visible = self.max_context_chars - 5
        head = max(3, visible // 2)
        tail = max(2, visible - head)
        return f"{text[:head].rstrip()} ... {text[-tail:].lstrip()}"

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

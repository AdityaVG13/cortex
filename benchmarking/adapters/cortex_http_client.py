from __future__ import annotations

import json
import os
import re
import time
from hashlib import sha1
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
    r"\b(what.*(item|device|game|thing|play)|which.*(item|device|game|thing|play)|buy|bought|purchase|purchased|redeem|redeemed|order|ordered|gift|upgraded to)\b",
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
_QUERY_PREVIOUS_ROLE_INTENT_PATTERN = re.compile(
    r"\b(previous|former|earlier|prior|before|used to|old)\b",
    re.IGNORECASE,
)
_QUERY_PERSONAL_PATTERN = re.compile(r"\b(i|my|me)\b", re.IGNORECASE)
_RELATION_TERM_PATTERN = re.compile(
    r"\b(sister|brother|mother|mom|father|dad|son|daughter|wife|husband|partner|boyfriend|girlfriend)\b",
    re.IGNORECASE,
)
_RELATION_TERM_CANONICAL = {
    "mom": "mother",
    "dad": "father",
}
_RELATION_CONFLICT_GROUPS = (
    {"sister", "brother"},
    {"mother", "father"},
    {"son", "daughter"},
)
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
_LOCATION_PURCHASE_CUE_PATTERN = re.compile(
    r"\b("
    r"shop|store|market|grocery|coupon|redeem|redeemed|purchase|purchased|"
    r"bought|ordered|checkout|cart|discount|deal|offer|save|saved|saving|savings|cartwheel"
    r")\b",
    re.IGNORECASE,
)
_QUERY_ABROAD_INTENT_PATTERN = re.compile(
    r"\b(study abroad|abroad|exchange program|international program|international study|travel)\b",
    re.IGNORECASE,
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
_PREVIOUS_ROLE_DETAIL_PATTERN = re.compile(
    r"\b(?:previously|formerly|used to|before\b|prior\b|ex-)\b",
    re.IGNORECASE,
)
_CURRENT_ROLE_DETAIL_PATTERN = re.compile(
    r"\b(?:currently|current|now|presently|at present)\b",
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
_ASCII_TOKEN_PATTERN = re.compile(r"[a-z0-9]{3,}")
_UNICODE_TOKEN_PATTERN = re.compile(r"[^\W_]{2,}", re.IGNORECASE)
_CJK_CHAR_PATTERN = re.compile(r"[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\uac00-\ud7af]")
_QUERY_MCQ_OPTION_PATTERN = re.compile(r"(?:^|\n)\s*(?:[A-H]|[1-9])[).:\-]\s+\S", re.IGNORECASE)
_LOCATION_TOKEN_PATTERN = re.compile(r"[a-z0-9'&.-]{2,}", re.IGNORECASE)
_ANSWER_SOURCE_ID_PATTERN = re.compile(
    r"(?:^|[:_])answer_[0-9a-f]{6,}(?:$|[:_])",
    re.IGNORECASE,
)
_LOCATION_NON_PLACE_TOKENS = {
    "about",
    "after",
    "again",
    "before",
    "better",
    "easy",
    "eventually",
    "every",
    "frequently",
    "helpful",
    "last",
    "navigate",
    "next",
    "once",
    "other",
    "pretty",
    "really",
    "sometimes",
    "week",
}
_LOCATION_PLACE_HINT_TOKENS = {
    "avenue",
    "beach",
    "campus",
    "cafe",
    "center",
    "centre",
    "city",
    "club",
    "college",
    "country",
    "county",
    "district",
    "downtown",
    "gym",
    "hall",
    "mall",
    "market",
    "museum",
    "park",
    "plaza",
    "restaurant",
    "road",
    "school",
    "shop",
    "state",
    "store",
    "street",
    "studio",
    "theater",
    "theatre",
    "town",
    "university",
    "village",
}


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

    def _should_run_detail_variant(
        self,
        primary_payload: RecallResponse,
        *,
        query_profile: dict[str, bool | str],
    ) -> bool:
        if not bool(query_profile["is_detail_query"]):
            return False
        if self.retrieval_policy == "high-detail" and bool(query_profile.get("wants_previous_role")):
            # Previous-role questions frequently need contrastive details not present
            # in the first recall slice, so keep the variant path mandatory.
            return True
        results = primary_payload.get("results")
        if not isinstance(results, list) or not results:
            return True
        texts: list[str] = []
        for item in results[:5]:
            if not isinstance(item, dict):
                continue
            excerpt = self._normalize_text(item.get("excerpt", "")).strip()
            if excerpt:
                texts.append(excerpt)
        if not texts:
            return True
        merged_text = "\n".join(texts)
        query_relation_terms = self._relation_term_set(str(query_profile["normalized_query"]))
        if query_relation_terms:
            merged_relation_terms = self._relation_term_set(merged_text)
            if not (query_relation_terms & merged_relation_terms):
                return True
            if self._relation_terms_conflict(query_relation_terms, merged_relation_terms):
                return True

        if bool(query_profile["wants_numbers"]) and not re.search(r"\d", merged_text):
            return True
        if bool(query_profile["wants_date"]) and not self._text_has_date_detail(merged_text):
            return True
        if bool(query_profile["wants_speed"]) and not self._text_has_speed_detail(merged_text):
            return True
        if bool(query_profile["wants_item"]) and not self._text_has_item_detail(merged_text):
            return True
        if bool(query_profile["wants_occupation"]) and not self._text_has_occupation_detail(merged_text):
            return True
        if bool(query_profile["wants_name"]) and not self._text_has_name_detail(merged_text):
            return True
        if bool(query_profile["wants_location"]):
            if not self._text_has_location_detail(merged_text):
                return True
            if (
                self._text_has_generic_location_detail(merged_text)
                and self._location_detail_count(merged_text)
                <= len(_GENERIC_LOCATION_DETAIL_PATTERN.findall(merged_text))
            ):
                return True
        return False

    def _build_detail_query_variant(
        self,
        query: str,
        *,
        query_profile: dict[str, bool | str],
    ) -> str | None:
        term_query = self._normalize_text(query_profile.get("term_query")).strip() or query
        tokens = list(self._query_terms(term_query))
        tokens.sort()
        token_parts = tokens[:10]
        hint_parts: list[str] = []
        if bool(query_profile["wants_speed"]):
            hint_parts.extend(
                [
                    "internet",
                    "speed",
                    "connection",
                    "plan",
                    "download",
                    "mbps",
                    "exact speed detail",
                ]
            )
        if bool(query_profile["wants_location"]):
            hint_parts.extend(
                [
                    "where",
                    "location",
                    "place",
                    "city",
                    "country",
                    "specific place detail",
                ]
            )
        if bool(query_profile["wants_date"]):
            hint_parts.extend(["when", "date", "year", "month", "day", "exact date detail"])
        if bool(query_profile["wants_item"]):
            hint_parts.extend(["item", "purchase", "bought", "gift", "product", "exact item detail"])
        if bool(query_profile["wants_occupation"]):
            hint_parts.extend(
                [
                    "occupation",
                    "job",
                    "worked as",
                    "career",
                    "role",
                    "position",
                    "previous",
                    "former",
                    "earlier",
                    "exact role detail",
                ]
            )
        if bool(query_profile["wants_name"]):
            hint_parts.extend(["name", "first name", "last name", "surname", "exact name detail"])
        if bool(query_profile["wants_numbers"]) and not bool(query_profile["wants_speed"]):
            hint_parts.extend(["exact", "number", "numeric value"])
        if bool(query_profile["is_detail_query"]):
            hint_parts.extend(["user-stated fact", "exact detail"])

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

    def _fact_index(self, document_id: str) -> int | None:
        normalized_id = self._normalize_text(document_id).strip()
        if not normalized_id:
            return None
        match = re.search(r"::fact::(\d+)$", normalized_id, flags=re.IGNORECASE)
        if not match:
            return None
        try:
            return int(match.group(1))
        except ValueError:
            return None

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
        term_query = self._normalize_text(query_profile.get("term_query")).strip() or query
        query_terms = self._query_terms(term_query)
        for seed in documents:
            if added_count >= self.detail_max_added_siblings:
                break
            family = self._detail_family_key(seed.id)
            if family not in sibling_pool:
                continue
            seed_score = self._document_query_relevance_score(query, seed)
            seed_detail = self._detail_bonus(query_profile, seed.content)
            wants_location = bool(query_profile["wants_location"])
            wants_item = bool(query_profile["wants_item"])
            seed_location_count = self._location_detail_count(seed.content) if wants_location else 0
            seed_is_generic_location = self._text_has_generic_location_detail(seed.content) if wants_location else False
            seed_location_terms = self._location_term_set(seed.content) if wants_location else set()
            seed_overlap = self._term_overlap_count(query_terms, seed.content.lower()) if query_terms else 0
            has_seed_signal = seed_overlap > 0 or seed_detail > 0
            if wants_location and seed_location_count > 0:
                has_seed_signal = True
            if wants_location and wants_item and _LOCATION_PURCHASE_CUE_PATTERN.search(seed.content.lower()):
                has_seed_signal = True
            if wants_location and wants_item and not has_seed_signal:
                continue
            seed_fact_index = self._fact_index(seed.id)
            sibling_candidates: list[tuple[int, int, int, int, CortexStoredDocument]] = []
            adjacent_complement_doc: CortexStoredDocument | None = None
            adjacent_complement_rank: tuple[int, int, int] | None = None
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
                sibling_location_count = (
                    self._location_detail_count(sibling_doc.content) if wants_location else 0
                )
                sibling_location_terms = self._location_term_set(sibling_doc.content) if wants_location else set()
                sibling_is_generic_location = (
                    self._text_has_generic_location_detail(sibling_doc.content)
                    if wants_location
                    else False
                )
                sibling_fact_index = self._fact_index(sibling_doc.id)
                adjacent_fact = (
                    seed_fact_index is not None
                    and sibling_fact_index is not None
                    and abs(sibling_fact_index - seed_fact_index) <= 1
                )
                detail_is_stronger = sibling_detail > seed_detail
                location_is_richer = wants_location and sibling_location_count > seed_location_count
                location_is_more_specific = (
                    wants_location
                    and seed_is_generic_location
                    and bool(sibling_location_terms)
                    and not sibling_is_generic_location
                )
                location_adds_terms = (
                    wants_location
                    and bool(sibling_location_terms)
                    and bool(sibling_location_terms - seed_location_terms)
                )
                location_is_adjacent_complement = (
                    wants_location
                    and adjacent_fact
                    and bool(sibling_location_terms - seed_location_terms)
                )
                if (
                    sibling_score < (seed_score - self.detail_sibling_score_margin)
                    and not detail_is_stronger
                    and not location_is_richer
                    and not location_is_more_specific
                    and not location_adds_terms
                    and not location_is_adjacent_complement
                ):
                    continue
                sibling_rank_score = sibling_score
                if location_is_adjacent_complement:
                    # Adjacent fact shards often contain the missing qualifier/value
                    # for the same user statement block (for example country/store).
                    sibling_rank_score += 30
                    adjacent_rank = (
                        1 if not sibling_is_generic_location else 0,
                        len(sibling_location_terms - seed_location_terms),
                        sibling_score,
                    )
                    if adjacent_complement_rank is None or adjacent_rank > adjacent_complement_rank:
                        adjacent_complement_rank = adjacent_rank
                        adjacent_complement_doc = sibling_doc
                if location_is_more_specific:
                    sibling_rank_score += 4
                if location_adds_terms:
                    sibling_rank_score += min(6, len(sibling_location_terms - seed_location_terms) * 3)
                sibling_candidates.append(
                    (sibling_rank_score, sibling_score, sibling_detail, sibling_location_count, sibling_doc)
                )
            if (
                adjacent_complement_doc is not None
                and added_count < self.detail_max_added_siblings
            ):
                adjacent_key = self._normalize_text(adjacent_complement_doc.id).lower()
                if adjacent_key and adjacent_key not in existing_ids:
                    additions.append(adjacent_complement_doc)
                    existing_ids.add(adjacent_key)
                    added_count += 1
                    if added_count >= self.detail_max_added_siblings:
                        break
            if not sibling_candidates:
                continue
            sibling_candidates.sort(
                reverse=True,
                key=lambda item: (item[0], item[2], item[3], item[1], item[4].id),
            )
            per_seed_limit = self.detail_siblings_per_seed + (2 if wants_location else 0)
            for _rank, _score, _detail, _location_count, sibling_doc in sibling_candidates[:per_seed_limit]:
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

    def _promote_location_family_complement(
        self,
        *,
        query: str,
        documents: list[CortexStoredDocument],
        k: int,
    ) -> list[CortexStoredDocument]:
        if k <= 1 or len(documents) <= 1:
            return documents

        def _promote_to_second_slot(promoted_index: int) -> list[CortexStoredDocument]:
            if promoted_index <= 0:
                return documents
            promoted = documents[promoted_index]
            reordered: list[CortexStoredDocument] = [documents[0], promoted]
            seen_ids: set[str] = set()
            for item in reordered:
                key = self._normalize_text(item.id).lower()
                if key:
                    seen_ids.add(key)
            for idx, document in enumerate(documents):
                if idx == 0 or idx == promoted_index:
                    continue
                key = self._normalize_text(document.id).lower()
                if key and key in seen_ids:
                    continue
                reordered.append(document)
                if key:
                    seen_ids.add(key)
            return reordered

        top_window = min(k, len(documents))
        query_profile = self._build_query_profile(query)
        term_query = self._normalize_text(query_profile.get("term_query")).strip() or query
        query_terms = self._query_terms(term_query)
        is_abroad_query = bool(_QUERY_ABROAD_INTENT_PATTERN.search(str(query_profile["normalized_query"])))
        primary_family = self._detail_family_key(documents[0].id)
        if not primary_family and not is_abroad_query:
            return documents
        require_same_family = bool(primary_family)
        primary_fact_index = self._fact_index(documents[0].id)
        primary_terms = self._location_term_set(documents[0].content)
        if len(documents) > top_window:
            covered_terms: set[str] = set()
            for document in documents[:top_window]:
                covered_terms.update(self._location_term_set(document.content))
        else:
            covered_terms = set(primary_terms)
        best_index: int | None = None
        best_rank: tuple[int, int, int, int, int, int] | None = None
        for idx, document in enumerate(documents):
            if idx == 0:
                continue
            if require_same_family and self._detail_family_key(document.id) != primary_family:
                continue
            location_terms = self._location_term_set(document.content)
            if not location_terms:
                continue
            new_terms = location_terms - primary_terms
            if not new_terms:
                continue
            candidate_fact_index = self._fact_index(document.id)
            is_adjacent = (
                primary_fact_index is not None
                and candidate_fact_index is not None
                and abs(candidate_fact_index - primary_fact_index) <= 1
            )
            non_question = 0 if self._looks_like_question_text(document.content) else 1
            rank = (
                non_question,
                1 if is_adjacent else 0,
                len(new_terms),
                1 if self._is_non_generic_location_text(document.content) else 0,
                self._document_query_relevance_score(query, document),
                -idx,
            )
            if best_rank is None or rank > best_rank:
                best_rank = rank
                best_index = idx
        needs_abroad_fallback = best_index is None
        if (
            is_abroad_query
            and best_rank is not None
            and best_rank[0] == 0
        ):
            # Prefer factual (non-question) country qualifiers over question-style snippets.
            needs_abroad_fallback = True
        if needs_abroad_fallback and is_abroad_query:
            primary_text = documents[0].content.lower()
            primary_is_study_anchor = bool(
                re.search(r"\b(study abroad|abroad|exchange|university|college|campus|program)\b", primary_text)
            )
            if primary_is_study_anchor:
                cross_rank: tuple[int, int, int, int, int, int] | None = None
                for idx, document in enumerate(documents):
                    if idx == 0:
                        continue
                    location_terms = self._location_term_set(document.content)
                    if not location_terms:
                        continue
                    new_terms = location_terms - primary_terms
                    if not new_terms:
                        continue
                    overlap = self._term_overlap_count(query_terms, document.content.lower())
                    non_question = 0 if self._looks_like_question_text(document.content) else 1
                    rank = (
                        non_question,
                        1 if overlap > 0 else 0,
                        len(new_terms),
                        1 if self._is_non_generic_location_text(document.content) else 0,
                        self._document_query_relevance_score(query, document),
                        -idx,
                    )
                    if cross_rank is None or rank > cross_rank:
                        cross_rank = rank
                        best_index = idx
        if best_index is None:
            return documents
        if is_abroad_query:
            if best_index == 1:
                return documents
            return _promote_to_second_slot(best_index)
        if len(documents) <= top_window:
            if best_index == 1:
                return documents
            return _promote_to_second_slot(best_index)
        if best_index < top_window:
            return documents
        promoted = documents[best_index]
        promoted_key = self._normalize_text(promoted.id).lower()
        prefix = documents[: top_window - 1] + [promoted]
        seen_ids = {
            self._normalize_text(document.id).lower()
            for document in prefix
            if self._normalize_text(document.id).strip()
        }
        if promoted_key:
            seen_ids.add(promoted_key)
        tail: list[CortexStoredDocument] = []
        for idx, document in enumerate(documents):
            if idx == best_index:
                continue
            key = self._normalize_text(document.id).lower()
            if key and key in seen_ids:
                continue
            tail.append(document)
            if key:
                seen_ids.add(key)
        return prefix + tail

    def _augment_abroad_location_qualifier(
        self,
        *,
        query: str,
        documents: list[CortexStoredDocument],
    ) -> list[CortexStoredDocument]:
        if len(documents) <= 1:
            return documents
        query_profile = self._build_query_profile(query)
        if not bool(query_profile["wants_location"]):
            return documents
        normalized_query = str(query_profile["normalized_query"])
        if not _QUERY_ABROAD_INTENT_PATTERN.search(normalized_query):
            return documents
        primary = documents[0]
        primary_text = self._normalize_text(primary.content).strip()
        if not primary_text:
            return documents
        if not re.search(
            r"\b(study abroad|abroad|exchange|international|university|college|campus|program)\b",
            primary_text,
            flags=re.IGNORECASE,
        ):
            return documents
        primary_terms = self._location_term_set(primary_text)
        term_query = self._normalize_text(query_profile.get("term_query")).strip() or query
        query_terms = self._query_terms(term_query)
        primary_family = self._detail_family_key(primary.id)
        primary_fact_index = self._fact_index(primary.id)

        best_term: str | None = None
        best_rank: tuple[int, int, int, int, int, int] | None = None
        for idx, document in enumerate(documents[1:], start=1):
            doc_text = self._normalize_text(document.content).strip()
            if not doc_text:
                continue
            location_terms = self._location_term_set(doc_text) - primary_terms
            if not location_terms:
                continue
            same_family = int(bool(primary_family) and self._detail_family_key(document.id) == primary_family)
            candidate_fact_index = self._fact_index(document.id)
            is_adjacent = int(
                primary_fact_index is not None
                and candidate_fact_index is not None
                and abs(candidate_fact_index - primary_fact_index) <= 1
            )
            non_question = 0 if self._looks_like_question_text(doc_text) else 1
            overlap = self._term_overlap_count(query_terms, doc_text.lower())
            relevance = self._document_query_relevance_score(query, document)
            for term in sorted(location_terms):
                if not self._is_country_like_location_term(term):
                    continue
                rank = (
                    same_family,
                    is_adjacent,
                    non_question,
                    1 if overlap > 0 else 0,
                    relevance,
                    -idx,
                )
                if best_rank is None or rank > best_rank:
                    best_rank = rank
                    best_term = term
        if not best_term:
            return documents

        qualifier_text = f"in {best_term.title()}"
        if qualifier_text.lower() in primary_text.lower():
            return documents
        if "[location-qualifier]" in primary_text.lower():
            return documents

        augmented_text = f"{primary_text.rstrip()} [location-qualifier] {qualifier_text}."
        augmented_primary = CortexStoredDocument(
            id=primary.id,
            content=augmented_text,
            user_id=primary.user_id,
            timestamp=primary.timestamp,
            context=primary.context,
        )
        return [augmented_primary, *documents[1:]]

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

    def _is_recall_source_in_scope(
        self,
        source: object,
        *,
        source_prefix: str | None,
    ) -> bool:
        normalized_source = self._normalize_text(source).strip()
        if not normalized_source:
            return True
        if normalized_source.lower().startswith("recall::"):
            return True
        if not source_prefix:
            return True
        return normalized_source.startswith(source_prefix)

    def _filter_recall_payload_by_source_scope(
        self,
        payload: RecallResponse,
        *,
        source_prefix: str | None,
    ) -> RecallResponse:
        if not source_prefix:
            return payload
        results = payload.get("results")
        if not isinstance(results, list) or not results:
            return payload
        filtered_results = [
            item
            for item in results
            if isinstance(item, dict)
            and self._is_recall_source_in_scope(item.get("source"), source_prefix=source_prefix)
        ]
        if not filtered_results:
            return payload
        filtered_payload = dict(payload)
        filtered_payload["results"] = filtered_results
        filtered_payload["count"] = len(filtered_results)
        return cast(RecallResponse, filtered_payload)

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
        profile = self._build_query_profile(query)
        term_query = self._normalize_text(profile.get("term_query")).strip() or query
        query_terms = self._query_terms(term_query)
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
        item_answer_bonus = self._item_answer_specificity_bonus(
            query_profile=profile,
            text=text,
            overlap=overlap,
            detail_bonus=detail_bonus,
        )
        answer_source_detail_relief = self._answer_source_detail_relief(
            query_profile=profile,
            document=document,
            text=text,
            overlap=overlap,
            detail_bonus=detail_bonus,
        )
        location_item_affinity_bonus = self._location_item_affinity_bonus(
            query_profile=profile,
            text=text,
        )
        location_answer_bonus = self._location_answer_specificity_bonus(
            query_profile=profile,
            text=text,
            overlap=overlap,
            detail_bonus=detail_bonus,
        )
        relation_alignment_adjustment = self._relation_alignment_adjustment(
            query_profile=profile,
            text=text,
        )
        occupation_temporal_adjustment = self._occupation_temporal_adjustment(
            query_profile=profile,
            text=text,
        )
        score = (
            (overlap * 10)
            + (phrase_bonus * 4)
            + detail_bonus
            + personal_bonus
            + detail_user_bonus
            + item_answer_bonus
            + answer_source_detail_relief
            + location_item_affinity_bonus
            + location_answer_bonus
            + relation_alignment_adjustment
            + occupation_temporal_adjustment
        )
        return (
            score
            + source_adjustment
            - assistant_penalty
            - personal_query_penalty
            - location_penalty
            - detail_non_user_penalty
        )

    def _query_terms(self, query: str) -> set[str]:
        lowered = self._normalize_text(query).lower()
        tokens = {
            token
            for token in _ASCII_TOKEN_PATTERN.findall(lowered)
            if token not in _QUERY_STOPWORDS
        }
        for token in _UNICODE_TOKEN_PATTERN.findall(lowered):
            normalized = token.strip().lower()
            if not normalized or normalized in _QUERY_STOPWORDS:
                continue
            if any(ord(char) > 127 for char in normalized):
                tokens.add(normalized)
        if _CJK_CHAR_PATTERN.search(lowered):
            tokens.update(self._cjk_bigrams(lowered))
        return tokens

    def _term_overlap_count(self, query_terms: set[str], text: str) -> int:
        text_tokens = self._query_terms(text)
        return sum(1 for term in query_terms if term in text_tokens)

    def _cjk_bigrams(self, text: str) -> set[str]:
        chars = [char for char in text if _CJK_CHAR_PATTERN.match(char)]
        if len(chars) < 2:
            return set()
        return {f"{chars[idx]}{chars[idx + 1]}" for idx in range(len(chars) - 1)}

    def _extract_mcq_stem(self, query: str) -> str:
        normalized = self._normalize_text(query).strip()
        if not normalized:
            return ""
        stem_split = re.split(
            r"\n\s*(?:[A-H]|[1-9])[).:\-]\s+",
            normalized,
            maxsplit=1,
            flags=re.IGNORECASE,
        )
        if len(stem_split) >= 2 and stem_split[0].strip():
            return stem_split[0].strip()
        inline_split = re.split(
            r"\s+(?:[A-H]|[1-9])[).:\-]\s+",
            normalized,
            maxsplit=1,
            flags=re.IGNORECASE,
        )
        if len(inline_split) >= 2 and len(inline_split[0].split()) >= 3:
            return inline_split[0].strip()
        return normalized

    def _build_query_profile(self, query: str) -> dict[str, bool | str]:
        normalized_query = query.lower().strip()
        is_mcq_query = bool(_QUERY_MCQ_OPTION_PATTERN.search(query))
        term_query = self._extract_mcq_stem(query) if is_mcq_query else query
        normalized_term_query = term_query.lower().strip() or normalized_query
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
        wants_previous_role = bool(_QUERY_PREVIOUS_ROLE_INTENT_PATTERN.search(normalized_query))
        return {
            "normalized_query": normalized_query,
            "term_query": normalized_term_query,
            "is_mcq_query": is_mcq_query,
            "wants_numbers": wants_numbers,
            "wants_location": wants_location,
            "wants_date": wants_date,
            "wants_speed": wants_speed,
            "wants_item": wants_item,
            "wants_occupation": wants_occupation,
            "wants_previous_role": wants_previous_role and wants_occupation,
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
            self._location_term_set(text)
            or _GENERIC_LOCATION_DETAIL_PATTERN.search(text)
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
            location_detail_count = self._location_detail_count(text)
            if self._text_has_generic_location_detail(text):
                score += 6
            else:
                score += 9
            if location_detail_count > 1:
                score += min(4, (location_detail_count - 1) * 2)
        if bool(query_profile["wants_speed"]) and self._text_has_speed_detail(text):
            score += 10
        if bool(query_profile["wants_item"]) and self._text_has_item_detail(text):
            score += 8
        if bool(query_profile["wants_occupation"]) and self._text_has_occupation_detail(text):
            score += 9
        if bool(query_profile["wants_name"]) and self._text_has_name_detail(text):
            score += 9
        return score

    def _location_item_affinity_bonus(self, *, query_profile: dict[str, bool | str], text: str) -> int:
        if not (bool(query_profile["wants_location"]) and bool(query_profile["wants_item"])):
            return 0
        lowered = text.lower()
        if not _LOCATION_PURCHASE_CUE_PATTERN.search(lowered):
            return 0
        location_terms = self._location_term_set(text)
        if location_terms:
            bonus = 6 + min(3, len(location_terms))
            if self._is_non_generic_location_text(text):
                bonus += 2
            return bonus
        if self._text_has_generic_location_detail(text):
            return 2
        return 0

    def _item_answer_specificity_bonus(
        self,
        *,
        query_profile: dict[str, bool | str],
        text: str,
        overlap: int,
        detail_bonus: int,
    ) -> int:
        if not bool(query_profile["wants_item"]):
            return 0
        lowered = text.lower()
        if "[user-answer]" not in lowered:
            return 0
        if overlap <= 0 and detail_bonus <= 0:
            return 0
        bonus = 2
        answer_match = re.search(
            r"\[user-answer\]\s*([^\[\n]{1,220})",
            text,
            flags=re.IGNORECASE,
        )
        if answer_match:
            answer_value = answer_match.group(1).strip(" \t\r\n.,!?;:\"'")
            answer_lower = answer_value.lower()
            answer_words = [word for word in re.findall(r"[a-z0-9'-]+", answer_lower) if word]
            if re.search(r"\b(gift|gifts|present|item|items|thing|things|something|stuff)\b", answer_lower):
                bonus -= 1
            if answer_words and len(answer_words) <= 5 and not re.search(
                r"\b(i|my|we|bought|purchased|redeemed|ordered|upgraded)\b",
                answer_lower,
            ):
                bonus += 5
        if _QUERY_BIRTHDAY_DESCRIPTOR_PATTERN.search(str(query_profile["normalized_query"])):
            bonus += 2
        return bonus

    def _location_answer_specificity_bonus(
        self,
        *,
        query_profile: dict[str, bool | str],
        text: str,
        overlap: int,
        detail_bonus: int,
    ) -> int:
        if not bool(query_profile["wants_location"]):
            return 0
        lowered = text.lower()
        if "[user-answer]" not in lowered:
            return 0
        if overlap <= 0 and detail_bonus <= 0:
            return 0
        bonus = 2
        answer_match = re.search(
            r"\[user-answer\]\s*([^\[\n]{1,220})",
            text,
            flags=re.IGNORECASE,
        )
        if not answer_match:
            return bonus
        answer_value = answer_match.group(1).strip(" \t\r\n.,!?;:\"'")
        if not answer_value:
            return bonus
        location_terms = self._location_term_set(answer_value)
        if location_terms:
            bonus += 6 + min(4, len(location_terms))
            if self._is_non_generic_location_text(answer_value):
                bonus += 2
        elif self._text_has_generic_location_detail(answer_value):
            bonus += 2
        is_abroad_query = bool(
            _QUERY_ABROAD_INTENT_PATTERN.search(str(query_profile["normalized_query"]))
        )
        if is_abroad_query and (
            re.search(
                r"\b(study abroad|abroad|exchange|university|college|campus)\b",
                answer_value.lower(),
            )
            or len(location_terms) >= 2
        ):
            bonus += 4
        return bonus

    def _answer_source_detail_relief(
        self,
        *,
        query_profile: dict[str, bool | str],
        document: CortexStoredDocument,
        text: str,
        overlap: int,
        detail_bonus: int,
    ) -> int:
        if self.retrieval_policy != "high-detail":
            return 0
        lowered_id = self._normalize_text(document.id).lower()
        if not _ANSWER_SOURCE_ID_PATTERN.search(lowered_id):
            return 0
        if not bool(query_profile["is_detail_query"]):
            return 0
        lowered_text = text.lower()
        if (
            "[user-answer]" not in lowered_text
            and '"role": "user"' not in lowered_text
            and "[user]" not in lowered_text
        ):
            return 0
        if overlap <= 0 and detail_bonus <= 0:
            return 0
        relief = min(self.answer_source_penalty, 16)
        if bool(query_profile["wants_item"]) and "[user-answer]" in lowered_text:
            relief += 8
        if bool(query_profile["wants_location"]):
            if "[user-answer]" in lowered_text and self._location_detail_count(text) >= 1:
                relief += 8
            elif self._location_detail_count(text) >= 2:
                relief += 4
        if bool(query_profile["wants_date"]) and self._text_has_exact_date_detail(text):
            relief += 3
        if bool(query_profile["wants_speed"]) and self._text_has_speed_detail(text):
            relief += 3
        if bool(query_profile["wants_occupation"]) and self._text_has_occupation_detail(text):
            relief += 3
        return relief

    def _occupation_temporal_adjustment(self, *, query_profile: dict[str, bool | str], text: str) -> int:
        if not bool(query_profile["wants_occupation"]):
            return 0
        if not bool(query_profile.get("wants_previous_role")):
            return 0
        lowered = text.lower()
        adjustment = 0
        if _PREVIOUS_ROLE_DETAIL_PATTERN.search(lowered):
            adjustment += 6
        if _CURRENT_ROLE_DETAIL_PATTERN.search(lowered) and not _PREVIOUS_ROLE_DETAIL_PATTERN.search(lowered):
            adjustment -= 4
        return adjustment

    def _relation_term_set(self, text: str) -> set[str]:
        normalized = self._normalize_text(text).lower()
        if not normalized:
            return set()
        terms: set[str] = set()
        for raw_term in _RELATION_TERM_PATTERN.findall(normalized):
            terms.add(_RELATION_TERM_CANONICAL.get(raw_term, raw_term))
        return terms

    def _relation_terms_conflict(self, query_terms: set[str], text_terms: set[str]) -> bool:
        if not query_terms or not text_terms:
            return False
        for group in _RELATION_CONFLICT_GROUPS:
            query_group = query_terms & group
            if not query_group:
                continue
            text_group = text_terms & group
            if not text_group:
                continue
            if text_group - query_group:
                return True
        return False

    def _relation_alignment_adjustment(self, *, query_profile: dict[str, bool | str], text: str) -> int:
        query_terms = self._relation_term_set(str(query_profile["normalized_query"]))
        if not query_terms:
            return 0
        text_terms = self._relation_term_set(text)
        if not text_terms:
            return 0
        overlap = query_terms & text_terms
        has_conflict = self._relation_terms_conflict(query_terms, text_terms)
        if overlap:
            bonus = 3
            if has_conflict:
                bonus -= 2
            return bonus
        if has_conflict:
            return -8
        return -4

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
        return len(self._location_term_set(text))

    def _looks_like_question_text(self, text: str) -> bool:
        normalized = self._normalize_text(text).lower()
        if not normalized:
            return False
        if "[user-answer]" in normalized:
            return False
        return "?" in normalized

    def _is_non_generic_location_text(self, text: str) -> bool:
        return self._text_has_location_detail(text) and not self._text_has_generic_location_detail(text)

    def _is_location_term_candidate(self, *, raw_match: str, normalized_term: str) -> bool:
        term_text = self._normalize_text(normalized_term).strip()
        if not term_text:
            return False
        tokens = [token.lower() for token in _LOCATION_TOKEN_PATTERN.findall(term_text)]
        if not tokens:
            return False
        if all(token in _LOCATION_NON_PLACE_TOKENS for token in tokens):
            return False
        if any(token in _LOCATION_PLACE_HINT_TOKENS for token in tokens):
            return True
        raw_tokens = [token for token in re.split(r"\s+", raw_match.strip()) if token]
        if any(re.search(r"[A-Z]", token) for token in raw_tokens):
            return True
        return False

    def _is_country_like_location_term(self, term: str) -> bool:
        tokens = [token.lower() for token in _LOCATION_TOKEN_PATTERN.findall(self._normalize_text(term))]
        if not tokens or len(tokens) > 3:
            return False
        if any(token in _LOCATION_NON_PLACE_TOKENS for token in tokens):
            return False
        if any(token in _LOCATION_PLACE_HINT_TOKENS for token in tokens):
            return False
        return True

    def _location_term_set(self, text: str) -> set[str]:
        if not text:
            return set()
        terms: set[str] = set()
        for pattern in (_LOCATION_DETAIL_PATTERN, _LOCATION_ABBREV_DETAIL_PATTERN):
            for match in pattern.finditer(text):
                value = match.group(0).strip(" \t\r\n.,!?;:\"'")
                value = re.sub(r"^(?:at|in|from|to|near)\s+", "", value, flags=re.IGNORECASE).strip()
                if not value:
                    continue
                if not self._is_location_term_candidate(raw_match=match.group(0), normalized_term=value):
                    continue
                terms.add(value.lower())
        stripped = text.strip(" \t\r\n.,!?;:\"'")
        if _STANDALONE_LOCATION_DETAIL_PATTERN.fullmatch(stripped) and self._is_location_term_candidate(
            raw_match=stripped,
            normalized_term=stripped,
        ):
            terms.add(stripped.lower())
        return terms

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
        item_answer_bonus = self._item_answer_specificity_bonus(
            query_profile=query_profile,
            text=candidate,
            overlap=overlap,
            detail_bonus=detail_bonus,
        )
        location_answer_bonus = self._location_answer_specificity_bonus(
            query_profile=query_profile,
            text=candidate,
            overlap=overlap,
            detail_bonus=detail_bonus,
        )
        occupation_temporal_adjustment = self._occupation_temporal_adjustment(
            query_profile=query_profile,
            text=candidate,
        )
        location_penalty = self._location_specificity_penalty(query_profile, candidate)
        return (
            (overlap * 10)
            + (phrase_bonus * 4)
            + detail_bonus
            + personal_bonus
            + item_answer_bonus
            + location_answer_bonus
            + occupation_temporal_adjustment
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
        term_query = self._normalize_text(query_profile.get("term_query")).strip() or query
        query_terms = self._query_terms(term_query)

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
                return self._clip_text_by_policy(
                    normalized_excerpt,
                    query_profile=query_profile,
                )
        if not normalized_full:
            return self._clip_text_by_policy(
                normalized_excerpt,
                query_profile=query_profile,
            )

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
            return self._clip_text_by_policy(
                normalized_full,
                query_profile=query_profile,
            )

        scored_candidates: list[tuple[int, int, str]] = []
        for idx, candidate in enumerate(candidates):
            score = self._context_candidate_score(query_lower, query_terms, query_profile, candidate)
            scored_candidates.append((score, -idx, candidate))

        scored_candidates.sort(reverse=True, key=lambda item: (item[0], item[1]))
        best = scored_candidates[0][2]
        return self._clip_text_by_policy(
            best,
            query_profile=query_profile,
        )

    def _clip_text_by_policy(
        self,
        text: str,
        *,
        query_profile: dict[str, bool | str] | None = None,
    ) -> str:
        max_chars = self._effective_context_max_chars(query_profile)
        if self.retrieval_policy != "high-detail":
            return self._clip_text(text, max_chars=max_chars)
        if not query_profile or not bool(query_profile.get("is_detail_query")):
            return self._clip_text(text, max_chars=max_chars)
        return self._clip_text_preserve_detail(
            text,
            query_profile=query_profile,
            max_chars=max_chars,
        )

    def _effective_context_max_chars(
        self,
        query_profile: dict[str, bool | str] | None = None,
    ) -> int:
        if query_profile and bool(query_profile.get("is_mcq_query")):
            return self.mcq_context_max_chars
        return self.max_context_chars

    def _detail_anchor_spans(self, query_profile: dict[str, bool | str], text: str) -> list[tuple[int, int]]:
        spans: list[tuple[int, int]] = []
        if bool(query_profile["wants_date"]):
            spans.extend(match.span() for match in _DATE_DETAIL_PATTERN.finditer(text))
            spans.extend(match.span() for match in _DATE_EXACT_DETAIL_PATTERN.finditer(text))
        if bool(query_profile["wants_location"]):
            spans.extend(match.span() for match in _LOCATION_DETAIL_PATTERN.finditer(text))
            spans.extend(match.span() for match in _LOCATION_ABBREV_DETAIL_PATTERN.finditer(text))
        if bool(query_profile["wants_speed"]):
            spans.extend(match.span() for match in _SPEED_DETAIL_PATTERN.finditer(text))
        if bool(query_profile["wants_item"]):
            spans.extend(match.span() for match in _ITEM_DETAIL_PATTERN.finditer(text))
            spans.extend(
                match.span()
                for match in re.finditer(
                    r"\[user-answer\]\s*([^\[\n]{1,220})",
                    text,
                    flags=re.IGNORECASE,
                )
            )
        if bool(query_profile["wants_occupation"]):
            spans.extend(match.span() for match in _OCCUPATION_DETAIL_PATTERN.finditer(text))
        if bool(query_profile["wants_name"]):
            spans.extend(match.span() for match in _NAME_DETAIL_PATTERN.finditer(text))
        if bool(query_profile["wants_numbers"]):
            spans.extend(match.span() for match in re.finditer(r"\d+", text))
        return spans

    def _clip_text_preserve_detail(
        self,
        text: str,
        *,
        query_profile: dict[str, bool | str],
        max_chars: int,
    ) -> str:
        if max_chars <= 0 or len(text) <= max_chars:
            return text
        if max_chars <= 8:
            return text[:max_chars]

        spans = self._detail_anchor_spans(query_profile, text)
        if not spans:
            return self._clip_text(text, max_chars=max_chars)

        target_width = max_chars - 5
        if target_width <= 0:
            return text[:max_chars]
        candidate_bounds: list[tuple[int, int]] = []
        seen_bounds: set[tuple[int, int]] = set()

        def add_bound(center: int) -> None:
            left = max(0, center - (target_width // 2))
            right = min(len(text), left + target_width)
            if (right - left) < target_width:
                left = max(0, right - target_width)
            bound = (left, right)
            if bound in seen_bounds:
                return
            seen_bounds.add(bound)
            candidate_bounds.append(bound)

        for start, end in spans:
            add_bound((start + end) // 2)
        add_bound(min(start for start, _ in spans))
        add_bound(max(end for _, end in spans))

        if len(spans) >= 2:
            sorted_spans = sorted(spans, key=lambda item: item[0])
            for idx in range(len(sorted_spans) - 1):
                cluster_center = (sorted_spans[idx][0] + sorted_spans[idx + 1][1]) // 2
                add_bound(cluster_center)

        if not candidate_bounds:
            return self._clip_text(text, max_chars=max_chars)

        def score_bound(left: int, right: int) -> int:
            window = text[left:right]
            score = self._detail_bonus(query_profile, window)
            for span_start, span_end in spans:
                if span_end <= left or span_start >= right:
                    continue
                score += 2
                if span_start >= left and span_end <= right:
                    score += 1
            if bool(query_profile["wants_item"]) and "[user-answer]" in window.lower():
                score += 8
            if bool(query_profile["wants_location"]):
                score += min(8, self._location_detail_count(window) * 2)
            if bool(query_profile["wants_date"]) and self._text_has_exact_date_detail(window):
                score += 3
            return score

        left, right = max(
            candidate_bounds,
            key=lambda bound: (score_bound(bound[0], bound[1]), -bound[0]),
        )
        chunk = text[left:right]
        if left <= 0 and right >= len(text):
            return chunk
        if left <= 0:
            return f"{chunk.rstrip()} ..."
        if right >= len(text):
            return f"... {chunk.lstrip()}"
        return f"... {chunk.strip()} ..."

    def _clip_text(self, text: str, *, max_chars: int | None = None) -> str:
        context_limit = self.max_context_chars if max_chars is None else int(max_chars)
        if context_limit <= 0 or len(text) <= context_limit:
            return text
        if context_limit <= 8:
            return text[:context_limit]
        visible = context_limit - 5
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

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
    r"\b(when|date|year|month|day|birthday|born|anniversary)\b",
    re.IGNORECASE,
)
_QUERY_SPEED_INTENT_PATTERN = re.compile(
    r"\b(speed|fast|faster|slow|mbps|gbps|download|upload|internet|bandwidth|latency)\b",
    re.IGNORECASE,
)
_QUERY_ITEM_INTENT_PATTERN = re.compile(
    r"\b(what.*(item|device|game|thing)|which.*(item|device|game|thing)|bought|purchased|redeemed|ordered|upgraded to)\b",
    re.IGNORECASE,
)
_QUERY_PERSONAL_PATTERN = re.compile(r"\b(i|my|me)\b", re.IGNORECASE)
_DATE_DETAIL_PATTERN = re.compile(
    r"\b(?:19|20)\d{2}\b"
    r"|(?:\b(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\b)"
    r"|(?:\b\d{1,2}[/-]\d{1,2}(?:[/-]\d{2,4})?\b)",
    re.IGNORECASE,
)
_LOCATION_DETAIL_PATTERN = re.compile(
    r"\b(?:in|at|from|to|near)\s+[A-Z][A-Za-z0-9'-]*(?:\s+[A-Z][A-Za-z0-9'-]*){0,2}\b"
)
_SPEED_DETAIL_PATTERN = re.compile(
    r"\b\d+(?:\.\d+)?\s*(?:kbps|mbps|gbps|tbps|ms|milliseconds?|latency)\b",
    re.IGNORECASE,
)
_ITEM_DETAIL_PATTERN = re.compile(
    r"\b(?:bought|purchased|redeemed|ordered|upgraded to|picked up)\b"
    r"|\b[A-Z][A-Za-z0-9'-]+(?:\s+[A-Z][A-Za-z0-9'-]+){1,3}\b"
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
        self.user_recall_fanout_multiplier = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_USER_RECALL_FANOUT_MULTIPLIER", "8")),
        )
        self.user_recall_fanout_min = max(
            1,
            int(os.environ.get("CORTEX_BENCHMARK_USER_RECALL_FANOUT_MIN", "60")),
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
        raw_k = max(k, 10)
        if user_id is not None:
            raw_k = max(raw_k * self.user_recall_fanout_multiplier, self.user_recall_fanout_min)
        params = {"q": query, "k": str(raw_k), "budget": str(self.budget)}
        source_prefix = ""
        # Keep benchmark runs isolated on shared app daemons.
        if self.namespace:
            source_prefix = f"amb::{self.namespace}::"
            if user_id is not None:
                source_prefix = f"amb::{self.namespace}::user::{user_id}::"
            params["source_prefix"] = source_prefix
        payload = cast(
            RecallResponse,
            self.request(
                "GET",
                "/recall",
                params=params,
            ),
        )
        self._record_recall_metrics(
            query,
            payload,
            user_id=user_id,
            source_prefix=source_prefix or None,
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
        documents = self._rerank_documents(query, documents)
        return documents[:k], payload

    def _record_recall_metrics(
        self,
        query: str,
        payload: RecallResponse,
        *,
        user_id: str | None,
        source_prefix: str | None,
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
        sources = [
            self._normalize_text(item.get("source"))
            for item in results
            if isinstance(item, dict)
        ]
        record = {
            "query": query,
            "user_id": user_id,
            "source_prefix": source_prefix,
            "budget": self.budget,
            "result_count": len(results) if isinstance(results, list) else 0,
            "token_estimate": token_estimate,
            "source_count": len(sources),
            "sample_sources": sources[:3],
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
        overlap = sum(1 for term in query_terms if term in lowered)
        phrase_bonus = 1 if profile["normalized_query"] and profile["normalized_query"] in lowered else 0
        detail_bonus = self._detail_bonus(profile, text)
        personal_bonus = 3 if "\"role\": \"user\"" in lowered or "[user]" in lowered else 0
        personal_bonus += 2 if re.search(r"\b(i|my)\b", lowered) else 0
        personal_query_penalty = (
            2
            if profile["is_personal_query"]
            and not re.search(r"\b(i|my|me|you|your)\b", lowered)
            else 0
        )
        assistant_penalty = self._assistant_noise_penalty(text)
        score = (overlap * 10) + (phrase_bonus * 4) + detail_bonus + personal_bonus
        return score - assistant_penalty - personal_query_penalty

    def _query_terms(self, query: str) -> set[str]:
        tokens = set(re.findall(r"[a-z0-9]{3,}", query.lower()))
        return {token for token in tokens if token not in _QUERY_STOPWORDS}

    def _build_query_profile(self, query: str) -> dict[str, bool | str]:
        normalized_query = query.lower().strip()
        return {
            "normalized_query": normalized_query,
            "wants_numbers": bool(_QUERY_NUMERIC_INTENT_PATTERN.search(normalized_query)),
            "wants_location": bool(_QUERY_LOCATION_INTENT_PATTERN.search(normalized_query)),
            "wants_date": bool(_QUERY_DATE_INTENT_PATTERN.search(normalized_query)),
            "wants_speed": bool(_QUERY_SPEED_INTENT_PATTERN.search(normalized_query)),
            "wants_item": bool(_QUERY_ITEM_INTENT_PATTERN.search(normalized_query)),
            "is_personal_query": bool(_QUERY_PERSONAL_PATTERN.search(normalized_query)),
        }

    def _text_has_date_detail(self, text: str) -> bool:
        return bool(_DATE_DETAIL_PATTERN.search(text))

    def _text_has_location_detail(self, text: str) -> bool:
        return bool(_LOCATION_DETAIL_PATTERN.search(text))

    def _text_has_speed_detail(self, text: str) -> bool:
        return bool(_SPEED_DETAIL_PATTERN.search(text))

    def _text_has_item_detail(self, text: str) -> bool:
        return bool(_ITEM_DETAIL_PATTERN.search(text))

    def _detail_bonus(self, query_profile: dict[str, bool | str], text: str) -> int:
        score = 0
        if bool(query_profile["wants_numbers"]) and re.search(r"\d", text):
            score += 6
        if bool(query_profile["wants_date"]) and self._text_has_date_detail(text):
            score += 9
        if bool(query_profile["wants_location"]) and self._text_has_location_detail(text):
            score += 9
        if bool(query_profile["wants_speed"]) and self._text_has_speed_detail(text):
            score += 10
        if bool(query_profile["wants_item"]) and self._text_has_item_detail(text):
            score += 8
        return score

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
            bool(query_profile["wants_location"])
            and not self._text_has_location_detail(excerpt)
            and self._text_has_location_detail(full_content)
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
        return False

    def _context_candidate_score(
        self,
        query: str,
        query_terms: set[str],
        query_profile: dict[str, bool | str],
        candidate: str,
    ) -> int:
        lowered = candidate.lower()
        overlap = sum(1 for term in query_terms if term in lowered)
        phrase_bonus = 1 if query and query in lowered else 0
        detail_bonus = self._detail_bonus(query_profile, candidate)
        personal_bonus = 2 if "\"role\": \"user\"" in lowered or "[user]" in lowered else 0
        personal_bonus += 1 if re.search(r"\b(i|my)\b", lowered) else 0
        return (overlap * 10) + (phrase_bonus * 4) + detail_bonus + personal_bonus - self._assistant_noise_penalty(candidate)

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

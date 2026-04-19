from __future__ import annotations

import json
import os
import re
import time
from hashlib import sha1
from pathlib import Path
from typing import cast

import httpx
from cortex_http_types import HealthResponse, RecallResponse
from memory_bench.memory.base import MemoryProvider
from memory_bench.models import Document

_ASCII_TOKEN_PATTERN = re.compile(r"[a-z0-9][a-z0-9._'-]*")
_QUERY_STOPWORDS = {
    "a",
    "an",
    "and",
    "are",
    "as",
    "at",
    "be",
    "by",
    "did",
    "do",
    "for",
    "from",
    "how",
    "i",
    "in",
    "is",
    "it",
    "me",
    "my",
    "of",
    "on",
    "or",
    "that",
    "the",
    "to",
    "was",
    "what",
    "when",
    "where",
    "which",
    "who",
    "with",
}
_QUERY_NUMERIC_INTENT_PATTERN = re.compile(
    r"\b(?:how many|how much|number|amount|year|date|age|old|speed|distance)\b",
    re.IGNORECASE,
)
_QUERY_LOCATION_INTENT_PATTERN = re.compile(
    r"\b(?:where|which country|country|city|town|state|province|abroad|location|place)\b",
    re.IGNORECASE,
)
_QUERY_DATE_INTENT_PATTERN = re.compile(
    r"\b(?:when|date|year|month|day|birthday)\b",
    re.IGNORECASE,
)
_QUERY_ITEM_INTENT_PATTERN = re.compile(
    r"\b(?:gift|present|item|buy|bought|purchase|redeem|redeemed|model|brand)\b",
    re.IGNORECASE,
)
_QUERY_PROFILE_INTENT_PATTERN = re.compile(
    r"\b(?:occupation|profession|career|job|role|position|worked as|work as)\b",
    re.IGNORECASE,
)
_QUERY_EDUCATION_INTENT_PATTERN = re.compile(
    r"\b(?:degree|major|minor|graduat(?:e|ed|ion)?|bachelor|master|doctorate|phd)\b",
    re.IGNORECASE,
)
_QUERY_EVENT_INTENT_PATTERN = re.compile(
    r"\b(?:play|theater|theatre|concert|show|production|musical|movie|film|attend|attended)\b",
    re.IGNORECASE,
)
_QUERY_BELIEF_INTENT_PATTERN = re.compile(
    r"\b(?:stance|belief|beliefs|spiritual|spirituality|religion|religious|faith|atheist|agnostic)\b",
    re.IGNORECASE,
)
_QUERY_ABROAD_INTENT_PATTERN = re.compile(
    r"\b(study abroad|abroad|exchange program|international program|international study|travel)\b",
    re.IGNORECASE,
)
_DATE_DETAIL_PATTERN = re.compile(
    r"\b(?:19|20)\d{2}\b"
    r"|\b(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\b"
    r"|\b\d{1,2}[/-]\d{1,2}(?:[/-]\d{2,4})?\b",
    re.IGNORECASE,
)
_PROFILE_DETAIL_PATTERN = re.compile(
    r"\b(?:occupation|profession|career|job|role|position|worked|working|specialist|manager|engineer|startup|company)\b",
    re.IGNORECASE,
)
_PROFILE_TITLE_PATTERN = re.compile(
    r"\b(?:specialist|manager|engineer|developer|analyst|coordinator|designer|consultant|assistant|director|lead|teacher|nurse|accountant|architect|marketer)\b",
    re.IGNORECASE,
)
_PROFILE_COMPANY_PATTERN = re.compile(
    r"\b(?:at|for)\s+(?:a|an|the)?\s*[a-z][a-z0-9'&.-]{1,}(?:\s+[a-z][a-z0-9'&.-]{1,}){0,3}\b",
    re.IGNORECASE,
)
_PREVIOUS_OCCUPATION_DETAIL_PATTERN = re.compile(
    r"\b(?:previous role as|former role as|prior role as|worked as|job as|occupation was|profession was)\b",
    re.IGNORECASE,
)
_EDUCATION_DETAIL_PATTERN = re.compile(
    r"\b(?:degree|major|minor|graduat(?:e|ed|ion)?|university|college|bachelor|master|phd|doctorate)\b",
    re.IGNORECASE,
)
_EVENT_DETAIL_PATTERN = re.compile(
    r"\b(?:play|theater|theatre|concert|show|production|musical|movie|film|attended|attend)\b",
    re.IGNORECASE,
)
_BELIEF_DETAIL_PATTERN = re.compile(
    r"\b(?:spiritual|spirituality|belief|beliefs|religion|religious|faith|atheist|agnostic|buddhism|stance)\b",
    re.IGNORECASE,
)
_PREVIOUS_ROLE_DETAIL_PATTERN = re.compile(
    r"\b(?:previously|formerly|used to|prior|before|ex-)\b",
    re.IGNORECASE,
)
_CURRENT_ACTIVITY_PATTERN = re.compile(
    r"\b(?:sold|sales|market|farmers' market|festival|booth|vendor)\b",
    re.IGNORECASE,
)
_GENERIC_LOCATION_DETAIL_PATTERN = re.compile(
    r"\b(?:home|house|work|office|school|college|university|hospital|store|shop|market|restaurant|mall|city|town|country)\b",
    re.IGNORECASE,
)
_ITEM_DETAIL_PATTERN = re.compile(
    r"\b(?:gift|present|item|model|brand|color|size|edition|version|ticket|subscription|plan|membership)\b",
    re.IGNORECASE,
)
_STORE_BRAND_PATTERN = re.compile(
    r"\b(?:target|walmart|costco|kroger|aldi|publix|safeway|walgreens|cvs|whole foods|trader joe'?s)\b",
    re.IGNORECASE,
)
_LOCATION_CLAUSE_PATTERN = re.compile(
    r"\b(?:in|at|from|to|near)\s+([a-z][a-z0-9'&.-]*(?:\s+[a-z][a-z0-9'&.-]*){0,3})\b",
    re.IGNORECASE,
)
_LOCATION_NON_PLACE_TOKENS = {
    "a",
    "an",
    "and",
    "for",
    "from",
    "in",
    "inside",
    "my",
    "of",
    "on",
    "our",
    "the",
    "their",
    "to",
    "your",
}
_LOCATION_PLACE_HINT_TOKENS = {
    "avenue",
    "beach",
    "campus",
    "city",
    "college",
    "country",
    "district",
    "downtown",
    "mall",
    "market",
    "park",
    "restaurant",
    "road",
    "school",
    "shop",
    "state",
    "store",
    "street",
    "theater",
    "theatre",
    "town",
    "university",
}
_QUERY_TERM_SYNONYMS: dict[str, set[str]] = {
    "degree": {"major", "graduated", "graduate", "graduation", "university", "college"},
    "major": {"degree", "graduated", "university", "college"},
    "graduate": {"graduated", "degree", "major", "university", "college"},
    "graduated": {"graduate", "degree", "major", "university", "college"},
    "occupation": {
        "job",
        "role",
        "position",
        "profession",
        "career",
        "worked",
        "work",
        "specialist",
        "company",
        "startup",
    },
    "profession": {"occupation", "job", "role", "career"},
    "job": {"occupation", "role", "position", "profession", "career", "worked", "specialist", "company"},
    "role": {"occupation", "job", "position", "profession", "worked", "specialist"},
    "career": {"occupation", "job", "role", "profession"},
    "previous": {"former", "prior", "past", "earlier", "used"},
    "former": {"previous", "prior", "past"},
    "prior": {"previous", "former", "past"},
    "spirituality": {"spiritual", "belief", "beliefs", "faith", "religion", "atheist", "agnostic"},
    "spiritual": {"spirituality", "belief", "beliefs", "faith", "religion", "atheist", "agnostic"},
    "belief": {"beliefs", "stance", "spirituality", "spiritual", "faith", "religion"},
    "beliefs": {"belief", "stance", "spirituality", "spiritual", "faith", "religion"},
    "stance": {"belief", "beliefs", "spirituality", "spiritual", "faith", "religion", "atheist", "agnostic"},
    "faith": {"belief", "beliefs", "religion", "spirituality", "spiritual"},
    "religion": {"religious", "faith", "belief", "beliefs", "spirituality", "spiritual"},
    "religious": {"religion", "faith", "belief", "beliefs", "spirituality", "spiritual"},
    "atheist": {"atheism", "agnostic", "spirituality", "belief", "stance"},
    "agnostic": {"atheist", "spirituality", "belief", "stance"},
    "play": {"theater", "theatre", "production", "show", "musical", "attended"},
    "theater": {"play", "theatre", "production", "show"},
    "theatre": {"play", "theater", "production", "show"},
    "production": {"play", "theater", "theatre", "show"},
    "redeem": {"redeemed", "coupon", "store"},
    "redeemed": {"redeem", "coupon", "store"},
    "coupon": {"redeem", "redeemed", "discount", "store"},
}
_TOKEN_SUFFIX_RULES: tuple[str, ...] = ("'s", "ing", "ed", "ers", "er", "es", "s")


def _slugify(value: str) -> str:
    normalized = re.sub(r"[^a-zA-Z0-9._-]+", "-", value.strip().lower())
    return normalized.strip("-") or "default"


class CortexHTTPBaseMemoryProvider(MemoryProvider):
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
        self.namespace = _slugify(os.environ.get("CORTEX_BENCHMARK_NAMESPACE", "default"))
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
            self.namespace = _slugify(namespace)

    def ingest(self, documents: list[Document]) -> None:
        for document in documents:
            expanded_docs = self._expand_document(document)
            for expanded in expanded_docs:
                for store_doc in self._split_for_store(expanded):
                    context_key = self._context_key(store_doc.id, store_doc.user_id)
                    serialized = self._serialize_document(store_doc)
                    if self._serialized_by_context.get(context_key) == serialized:
                        self.docs_by_context[context_key] = store_doc
                        continue
                    self.docs_by_context[context_key] = store_doc
                    self._serialized_by_context[context_key] = serialized
                    digest = sha1(serialized.encode("utf-8")).hexdigest()
                    if (
                        self.dedupe_identical_store_payloads
                        and digest in self._stored_content_digests
                    ):
                        continue
                    self._request(
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

    def retrieve(
        self,
        query: str,
        k: int = 10,
        user_id: str | None = None,
        query_timestamp: str | None = None,
    ) -> tuple[list[Document], RecallResponse]:
        _ = query_timestamp
        query_k = max(1, int(k))
        raw_k = max(query_k * self.recall_fanout_multiplier, self.recall_fanout_min)
        query_profile = self._build_query_profile(query)
        term_query = self._as_text(query_profile.get("term_query")).strip() or query
        query_terms = self._query_terms(term_query)
        recall_budget = self.budget
        if bool(query_profile["wants_previous_role"]) or bool(query_profile["wants_belief"]):
            recall_budget = max(recall_budget, 420)
        recall_query = query
        use_detail_variant = (
            bool(query_profile["wants_profile"])
            or bool(query_profile["wants_education"])
            or bool(query_profile["wants_belief"])
            or (bool(query_profile["wants_item"]) and bool(query_profile["wants_location"]))
        )
        if use_detail_variant:
            detail_variant = self._build_detail_query_variant(
                query,
                query_profile=query_profile,
            )
            if detail_variant:
                recall_query = detail_variant
        params: dict[str, str] = {
            "q": recall_query,
            "k": str(raw_k),
            "budget": str(recall_budget),
        }
        source_prefix = self._source_prefix(user_id)
        if source_prefix:
            params["source_prefix"] = source_prefix
        payload = cast(RecallResponse, self._request("GET", "/recall", params=params))
        self._record_recall_metrics(
            query=query,
            payload=payload,
            user_id=user_id,
            source_prefix=source_prefix or None,
            requested_budget=recall_budget,
        )

        documents: list[Document] = []
        fallback_documents: list[Document] = []
        seen_document_ids: set[str] = set()
        for result in payload.get("results") or []:
            source = self._as_text((result or {}).get("source")).strip()
            excerpt = self._as_text((result or {}).get("excerpt")).strip()
            source_keys = self._split_source_keys(source)
            appended_stored = False
            for source_key in source_keys:
                stored = self.docs_by_context.get(source_key)
                if stored is None:
                    continue
                if user_id is not None and stored.user_id != user_id:
                    continue
                normalized_id = self._normalize_text(stored.id).lower()
                if not normalized_id or normalized_id in seen_document_ids:
                    continue
                content = self._build_query_context_text(
                    query=query,
                    query_profile=query_profile,
                    query_terms=query_terms,
                    full_content=stored.content,
                    excerpt=excerpt,
                )
                documents.append(
                    Document(
                        id=stored.id,
                        content=content,
                        user_id=stored.user_id,
                        timestamp=stored.timestamp,
                        context=stored.context,
                    )
                )
                seen_document_ids.add(normalized_id)
                appended_stored = True
                continue
            if appended_stored:
                continue
            if not excerpt:
                continue
            document_id = source or f"recall-{len(documents)}"
            fallback_documents.append(
                Document(
                    id=document_id,
                    content=excerpt,
                    user_id=user_id,
                )
            )
        if bool(query_profile["is_detail_query"]):
            documents = self._expand_fact_family_candidates(
                query=query,
                query_profile=query_profile,
                query_terms=query_terms,
                documents=documents,
                user_id=user_id,
            )
        documents = self._rerank_documents(query, documents)
        if bool(query_profile["wants_location"]):
            documents = self._promote_location_family_complement(
                query=query,
                documents=documents,
                k=query_k,
            )
            documents = self._augment_item_location_qualifier(
                query=query,
                documents=documents,
            )
            documents = self._augment_abroad_location_qualifier(
                query=query,
                documents=documents,
            )
        for document in documents:
            normalized_id = self._normalize_text(document.id).lower()
            if normalized_id:
                seen_document_ids.add(normalized_id)
        if len(documents) < query_k:
            for fallback in fallback_documents:
                normalized_id = self._normalize_text(fallback.id).lower()
                if normalized_id and normalized_id in seen_document_ids:
                    continue
                if normalized_id:
                    seen_document_ids.add(normalized_id)
                documents.append(fallback)
                if len(documents) >= query_k:
                    break
        return documents[:query_k], payload

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

    def _expand_document(self, document: Document) -> list[Document]:
        base_content = self._as_text(document.content)
        expanded: list[Document] = []
        if self.store_full_docs:
            expanded.append(
                Document(
                    id=document.id,
                    content=base_content,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=document.context,
                )
            )

        if not self.enable_fact_extracts or self.max_fact_extracts_per_doc <= 0:
            if not expanded:
                expanded.append(
                    Document(
                        id=document.id,
                        content=base_content,
                        user_id=document.user_id,
                        timestamp=document.timestamp,
                        context=document.context,
                    )
                )
            return expanded

        snippets = self._extract_fact_snippets(base_content)
        base_context = self._as_text(document.context).strip()
        for idx, snippet in enumerate(snippets, start=1):
            fact_context = f"{base_context} [fact-extract]".strip()
            expanded.append(
                Document(
                    id=f"{document.id}::fact::{idx}",
                    content=snippet,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=fact_context or None,
                )
            )
            if idx >= self.max_fact_extracts_per_doc:
                break

        if not expanded:
            expanded.append(
                Document(
                    id=document.id,
                    content=base_content,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=document.context,
                )
            )
        return expanded

    def _extract_fact_snippets(self, content: str) -> list[str]:
        raw = content.strip()
        if not raw.startswith("[") or not raw.endswith("]"):
            return []
        try:
            payload = json.loads(raw)
        except Exception:
            return []
        if not isinstance(payload, list):
            return []

        personal_markers = (
            " i ",
            " my ",
            " me ",
            " i've ",
            " i'm ",
            " i'd ",
            " i was ",
            " i just ",
            " i recently ",
            " i used to ",
            " i graduated ",
            " i redeemed ",
            " i bought ",
            " i attended ",
            " i upgraded ",
            " i volunteered ",
            " by the way ",
        )
        sentence_split = re.compile(r"(?<=[.!?])\s+")
        fact_verb_pattern = re.compile(
            r"\bi\s+(?:was|am|have|had|got|bought|take|takes|took|attend|attends|attended|graduated|upgraded|packed|changed|remember|used to|redeemed|repainted|volunteered|worked|studied|moved)\b"
        )
        date_measure_pattern = re.compile(
            r"\b(?:19|20)\d{2}\b"
            r"|\b(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\b"
            r"|\b\d{1,2}[/-]\d{1,2}(?:[/-]\d{2,4})?\b"
            r"|\b\d+(?:\.\d+)?\s*(?:kbps|mbps|gbps|minutes?|hours?|days?|weeks?|months?|years?|km|miles?|dollars?)\b",
            re.IGNORECASE,
        )
        item_detail_pattern = re.compile(
            r"\b(?:gift|present)\s+(?:was|is)\s+(?:a|an|the)\s+[a-z][a-z0-9'-]{2,}(?:\s+[a-z][a-z0-9'-]{2,}){0,3}\b"
            r"|\b(?:a|an|the)\s+(?:yellow|blue|red|green|black|white|silver|gold|pink|purple|orange|navy|brown|gray|grey)\s+[a-z][a-z0-9'-]{2,}\b",
            re.IGNORECASE,
        )
        location_pattern = re.compile(
            r"\b(?:at|in|from|to|near)\s+[A-Za-z][A-Za-z0-9'-]*(?:\s+[A-Za-z][A-Za-z0-9'-]*){0,2}\b",
            re.IGNORECASE,
        )
        assistant_fact_pattern = re.compile(r"\b(?:you|your|yours)\b", re.IGNORECASE)
        assistant_reflection_pattern = re.compile(
            r"\b(?:you mentioned|you said|you told me|your\s+[a-z0-9_-]+\s+(?:is|was|are|were|takes|took|upgraded|bought|redeemed|moved|graduated))\b",
            re.IGNORECASE,
        )
        proper_noun_pattern = re.compile(
            r"\b[A-Z][A-Za-z0-9'-]+(?:\s+[A-Z][A-Za-z0-9'-]+){0,2}\b"
        )
        low_signal_pattern = re.compile(
            r"\b(?:here are|tips?|recommendations?|you can|you should|remember to|step\s+\d+|let me know|if you'd like|happy to help|overall)\b",
            re.IGNORECASE,
        )
        high_signal_sentence_prefixes = (
            "i ",
            "i'm ",
            "i've ",
            "i was ",
            "i just ",
            "i recently ",
            "my ",
        )
        short_fact_pattern = re.compile(
            r"^(?:"
            r"\$?\d+(?:\.\d+)?\s*(?:kbps|mbps|gbps|minutes?|hours?|days?|weeks?|months?|years?|miles?|km|%)?"
            r"|(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\s+\d{1,2}(?:st|nd|rd|th)?"
            r"|(?:\d{1,2}(?:st|nd|rd|th)?\s+of\s+(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*)"
            r"|(?:a|an|the)\s+[a-z][a-z0-9'-]{2,}(?:\s+[a-z][a-z0-9'-]{2,}){0,4}"
            r"|[A-Z][A-Za-z0-9'&.-]{2,}(?:\s+[A-Z][A-Za-z0-9'&.-]{2,}){0,3}"
            r")\.?$",
            re.IGNORECASE,
        )
        short_non_answer_pattern = re.compile(
            r"^(?:yes|no|ok|okay|sure|maybe|thanks|thank you|got it|sounds good)\.?$",
            re.IGNORECASE,
        )

        snippets: list[str] = []
        seen: set[str] = set()
        ranked_candidates: list[tuple[int, int, int, str]] = []
        sequence = 0
        parsed_turns: list[tuple[str, str]] = []
        for turn in payload:
            if not isinstance(turn, dict):
                continue
            message_text = self._as_text(turn.get("content") or turn.get("text"))
            if not message_text:
                continue
            role = self._as_text(turn.get("role") or turn.get("speaker")).strip().lower()
            normalized = re.sub(r"\s+", " ", message_text).strip().replace("\u2019", "'")
            if role == "user":
                if len(normalized) < 4:
                    continue
            elif len(normalized) < 20:
                continue
            parsed_turns.append((role, normalized))

        if not parsed_turns:
            return []

        user_turns = [(idx, role, text) for idx, (role, text) in enumerate(parsed_turns) if role == "user"]
        candidate_turns = user_turns if user_turns else [
            (idx, role, text) for idx, (role, text) in enumerate(parsed_turns)
        ]
        for turn_index, role, normalized in candidate_turns:
            role_prefix = role if role else "message"
            role_score = 6 if role_prefix == "user" else 1
            sentences = [segment.strip() for segment in sentence_split.split(normalized) if segment.strip()]
            for sentence_index, sentence in enumerate(sentences):
                compact = sentence
                if sentence_index + 1 < len(sentences):
                    adjacent = sentences[sentence_index + 1]
                    merge_adjacent = bool(
                        date_measure_pattern.search(adjacent)
                        or location_pattern.search(adjacent)
                        or item_detail_pattern.search(adjacent)
                        or proper_noun_pattern.search(adjacent)
                    )
                    if merge_adjacent and len(compact) + len(adjacent) + 1 <= self.fact_extract_max_chars:
                        compact = f"{compact} {adjacent}"
                short_reply_candidate = False
                previous_text = ""
                previous_is_question = False
                if turn_index > 0:
                    previous_text = parsed_turns[turn_index - 1][1]
                    previous_is_question = bool(
                        "?" in previous_text
                        or re.search(r"\b(where|when|what|which|who|how)\b", previous_text.lower())
                    )
                if len(compact) < 24:
                    if role_prefix == "user":
                        compact_value = compact.rstrip(".!?").strip()
                        if (
                            compact_value
                            and len(compact_value) >= 3
                            and not short_non_answer_pattern.fullmatch(compact_value)
                            and (
                                short_fact_pattern.fullmatch(compact_value)
                                or (
                                    previous_is_question
                                    and len(compact_value) <= 48
                                    and not low_signal_pattern.search(compact_value)
                                )
                            )
                        ):
                            short_reply_candidate = True
                    if not short_reply_candidate:
                        continue
                marker_haystack = f" {compact.lower()} "
                has_personal_marker = any(marker in marker_haystack for marker in personal_markers)
                if role_prefix == "user":
                    if not has_personal_marker and not short_reply_candidate:
                        continue
                else:
                    if not any(marker in marker_haystack for marker in (" i ", " my ", " by the way ")):
                        continue

                score = role_score
                if compact.lower().startswith(high_signal_sentence_prefixes):
                    score += 2
                if "by the way" in marker_haystack:
                    score += 2
                if fact_verb_pattern.search(marker_haystack):
                    score += 3
                if date_measure_pattern.search(compact):
                    score += 2
                if location_pattern.search(compact):
                    score += 2
                if item_detail_pattern.search(compact):
                    score += 2
                if low_signal_pattern.search(compact):
                    score -= 4
                if short_reply_candidate:
                    score += 5
                    if previous_is_question:
                        score += 3
                if len(compact) <= 220:
                    score += 1
                if score <= 0:
                    continue

                snippet_source = compact
                if short_reply_candidate and previous_text:
                    question_text = previous_text.strip()
                    if len(question_text) > self.short_reply_question_max_chars:
                        if self.short_reply_question_max_chars <= 5:
                            question_text = question_text[: self.short_reply_question_max_chars]
                        else:
                            question_text = (
                                question_text[: self.short_reply_question_max_chars - 3].rstrip()
                                + "..."
                            )
                    snippet_source = f"[user-answer] {compact}"
                    if question_text:
                        snippet_source = f"{snippet_source} [assistant-question] {question_text}"
                snippet_text = self._clip_snippet(snippet_source)
                snippet = f"[{role_prefix}] {snippet_text}".strip()
                ranked_candidates.append((score, sequence, turn_index, snippet))
                sequence += 1

        if not ranked_candidates:
            for _turn_index, role, normalized in candidate_turns[:4]:
                marker_haystack = f" {normalized.lower()} "
                role_prefix = role if role else "message"
                role_score = 2 if role_prefix == "user" else 0
                if role_score <= 0:
                    continue
                if not any(marker in marker_haystack for marker in personal_markers):
                    continue
                snippet_text = self._clip_snippet(normalized)
                snippet = f"[{role_prefix}] {snippet_text}".strip()
                ranked_candidates.append((role_score, sequence, -1, snippet))
                sequence += 1

        if user_turns and self.include_assistant_fact_extracts:
            for idx, (role, normalized) in enumerate(parsed_turns):
                if role != "assistant":
                    continue
                if idx <= 0 or parsed_turns[idx - 1][0] != "user":
                    continue
                for sentence in sentence_split.split(normalized):
                    compact = sentence.strip()
                    if len(compact) < 24 or len(compact) > 260:
                        continue
                    if low_signal_pattern.search(compact):
                        continue
                    factual_signal = 0
                    if date_measure_pattern.search(compact):
                        factual_signal += 2
                    if location_pattern.search(compact):
                        factual_signal += 1
                    if proper_noun_pattern.search(compact):
                        factual_signal += 1
                    if re.search(r"\d", compact):
                        factual_signal += 1
                    has_mirror_pronoun = bool(assistant_fact_pattern.search(compact))
                    has_reflection = bool(assistant_reflection_pattern.search(compact))
                    if factual_signal <= 0:
                        continue
                    if not has_mirror_pronoun:
                        continue
                    if not has_reflection and factual_signal < 3:
                        continue
                    score = 1 + factual_signal + (1 if has_reflection else 0) + (1 if len(compact) <= 220 else 0)
                    snippet_text = self._clip_snippet(compact)
                    snippet = f"[assistant] {snippet_text}".strip()
                    ranked_candidates.append((score, sequence, idx, snippet))
                    sequence += 1

        ranked_candidates.sort(key=lambda item: (-item[0], item[1]))
        first_pass_turn_cap = 1
        per_turn_counts: dict[int, int] = {}
        deferred: list[tuple[int, int, int, str]] = []
        for candidate in ranked_candidates:
            _score, _seq, turn_index, snippet = candidate
            if snippet in seen:
                continue
            if turn_index >= 0 and per_turn_counts.get(turn_index, 0) >= first_pass_turn_cap:
                deferred.append(candidate)
                continue
            seen.add(snippet)
            snippets.append(snippet)
            if turn_index >= 0:
                per_turn_counts[turn_index] = per_turn_counts.get(turn_index, 0) + 1
            if len(snippets) >= self.max_fact_extracts_per_doc:
                break
        if len(snippets) < self.max_fact_extracts_per_doc:
            for _score, _seq, _turn_index, snippet in deferred:
                if snippet in seen:
                    continue
                seen.add(snippet)
                snippets.append(snippet)
                if len(snippets) >= self.max_fact_extracts_per_doc:
                    break
        return snippets

    def _clip_snippet(self, value: str) -> str:
        if len(value) <= self.fact_extract_max_chars:
            return value
        if self.fact_extract_max_chars <= 32:
            return value[: self.fact_extract_max_chars]
        half = max(12, (self.fact_extract_max_chars - 5) // 2)
        return f"{value[:half].rstrip()} ... {value[-half:].lstrip()}"

    def _split_for_store(self, document: Document) -> list[Document]:
        content = self._as_text(document.content)
        if self.store_max_chars <= 0 or len(content) <= self.store_max_chars:
            return [
                Document(
                    id=document.id,
                    content=content,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=document.context,
                )
            ]

        chunks = self._split_content_for_store(content)
        if len(chunks) <= 1:
            return [
                Document(
                    id=document.id,
                    content=content,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=document.context,
                )
            ]

        width = max(2, len(str(len(chunks))))
        base_context = self._as_text(document.context)
        chunked: list[Document] = []
        for idx, chunk in enumerate(chunks, start=1):
            part_context = f"{base_context} [store-part {idx}/{len(chunks)}]".strip()
            chunked.append(
                Document(
                    id=f"{document.id}::part::{idx:0{width}d}",
                    content=chunk,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=part_context or None,
                )
            )
        return chunked

    def _split_content_for_store(self, content: str) -> list[str]:
        if self.store_max_chars <= 0 or len(content) <= self.store_max_chars:
            return [content]
        chunks: list[str] = []
        start = 0
        hard_cap = self.store_max_chars
        while start < len(content):
            remaining = len(content) - start
            if remaining <= hard_cap:
                chunks.append(content[start:])
                break
            end = start + hard_cap
            search_start = start + max(1, hard_cap // 3)
            boundary = -1
            for idx in range(end - 1, search_start - 1, -1):
                if content[idx] in "\n.!?;":
                    boundary = idx + 1
                    break
            if boundary <= start:
                boundary = end
            chunks.append(content[start:boundary])
            start = boundary
        return chunks

    def _normalize_text(self, value: object | None) -> str:
        return re.sub(r"\s+", " ", self._as_text(value)).strip()

    def _build_query_profile(self, query: str) -> dict[str, bool | str]:
        normalized_query = self._normalize_text(query).lower()
        wants_numbers = bool(_QUERY_NUMERIC_INTENT_PATTERN.search(normalized_query))
        wants_location = bool(_QUERY_LOCATION_INTENT_PATTERN.search(normalized_query))
        wants_date = bool(_QUERY_DATE_INTENT_PATTERN.search(normalized_query))
        wants_item = bool(_QUERY_ITEM_INTENT_PATTERN.search(normalized_query))
        wants_profile = bool(_QUERY_PROFILE_INTENT_PATTERN.search(normalized_query))
        wants_education = bool(_QUERY_EDUCATION_INTENT_PATTERN.search(normalized_query))
        wants_event = bool(_QUERY_EVENT_INTENT_PATTERN.search(normalized_query))
        wants_belief = bool(_QUERY_BELIEF_INTENT_PATTERN.search(normalized_query))
        wants_previous_role = bool(
            wants_profile and re.search(r"\b(previous|former|prior|used to|earlier)\b", normalized_query)
        )
        term_query = normalized_query
        if wants_previous_role:
            term_query = (
                f"{normalized_query} worked as job title profession role company startup before used to"
            ).strip()
        if wants_belief and re.search(r"\b(previous|former|prior|used to|earlier|stance)\b", normalized_query):
            term_query = (
                f"{term_query} stance belief beliefs spirituality faith religion religious atheist agnostic"
            ).strip()
        is_detail_query = (
            wants_numbers
            or wants_location
            or wants_date
            or wants_item
            or wants_profile
            or wants_education
            or wants_event
            or wants_belief
        )
        return {
            "normalized_query": normalized_query,
            "term_query": term_query,
            "wants_numbers": wants_numbers,
            "wants_location": wants_location,
            "wants_date": wants_date,
            "wants_item": wants_item,
            "wants_profile": wants_profile,
            "wants_education": wants_education,
            "wants_event": wants_event,
            "wants_belief": wants_belief,
            "wants_previous_role": wants_previous_role,
            "is_detail_query": is_detail_query,
        }

    def _query_terms(self, query: str) -> set[str]:
        lowered = self._normalize_text(query).lower()
        seed_terms = {
            token
            for token in _ASCII_TOKEN_PATTERN.findall(lowered)
            if token not in _QUERY_STOPWORDS and len(token) >= 2
        }
        expanded: set[str] = set()
        for token in seed_terms:
            token_forms = self._token_forms(token)
            expanded.update(token_forms)
            for form in token_forms:
                expanded.update(_QUERY_TERM_SYNONYMS.get(form, set()))
        return expanded

    def _text_terms(self, text: str) -> set[str]:
        lowered = self._normalize_text(text).lower()
        seed_terms = {
            token
            for token in _ASCII_TOKEN_PATTERN.findall(lowered)
            if token not in _QUERY_STOPWORDS and len(token) >= 2
        }
        terms: set[str] = set()
        for token in seed_terms:
            terms.update(self._token_forms(token))
        return terms

    def _token_forms(self, token: str) -> set[str]:
        forms: set[str] = {token}
        if token.endswith("ies") and len(token) > 4:
            forms.add(token[:-3] + "y")
        for suffix in _TOKEN_SUFFIX_RULES:
            if not token.endswith(suffix):
                continue
            stem = token[: -len(suffix)] if suffix else token
            if len(stem) >= 3:
                forms.add(stem)
        return forms

    def _build_detail_query_variant(
        self,
        query: str,
        *,
        query_profile: dict[str, bool | str],
    ) -> str | None:
        term_query = self._as_text(query_profile.get("term_query")).strip() or query
        ordered_seed_tokens = [
            token
            for token in _ASCII_TOKEN_PATTERN.findall(term_query.lower())
            if token not in _QUERY_STOPWORDS and len(token) >= 2
        ]
        token_parts: list[str] = []
        seen_tokens: set[str] = set()
        for token in ordered_seed_tokens:
            for candidate in [token, *sorted(self._token_forms(token))]:
                normalized = self._normalize_text(candidate).strip().lower()
                if (
                    not normalized
                    or normalized in _QUERY_STOPWORDS
                    or len(normalized) < 2
                    or normalized in seen_tokens
                ):
                    continue
                token_parts.append(normalized)
                seen_tokens.add(normalized)
            for synonym in sorted(_QUERY_TERM_SYNONYMS.get(token, set())):
                normalized = self._normalize_text(synonym).strip().lower()
                if (
                    not normalized
                    or normalized in _QUERY_STOPWORDS
                    or len(normalized) < 2
                    or normalized in seen_tokens
                ):
                    continue
                token_parts.append(normalized)
                seen_tokens.add(normalized)
        for token in sorted(self._query_terms(term_query)):
            normalized = self._normalize_text(token).strip().lower()
            if (
                not normalized
                or normalized in _QUERY_STOPWORDS
                or len(normalized) < 2
                or normalized in seen_tokens
            ):
                continue
            token_parts.append(normalized)
            seen_tokens.add(normalized)
            if len(token_parts) >= 18:
                break
        token_parts = token_parts[:18]
        hint_parts: list[str] = []
        if bool(query_profile["wants_location"]):
            hint_parts.extend(["where", "location", "city", "country", "place"])
        if bool(query_profile["wants_date"]):
            hint_parts.extend(["when", "date", "year", "month", "day"])
        if bool(query_profile["wants_item"]):
            hint_parts.extend(["item", "purchase", "redeemed", "store", "exact detail"])
        if bool(query_profile["wants_profile"]):
            hint_parts.extend(
                [
                    "occupation",
                    "job",
                    "worked as",
                    "role",
                    "career",
                    "position",
                ]
            )
        if bool(query_profile["wants_previous_role"]):
            hint_parts.extend(["worked as", "job title", "profession", "previous", "former", "prior"])
        if bool(query_profile["wants_education"]):
            hint_parts.extend(["degree", "major", "graduated", "university", "college"])
        if bool(query_profile["wants_event"]):
            hint_parts.extend(["play", "theater", "production", "attended"])
        if bool(query_profile["wants_belief"]):
            hint_parts.extend(
                [
                    "stance",
                    "belief",
                    "spirituality",
                    "faith",
                    "religion",
                    "atheist",
                    "agnostic",
                    "used to",
                    "previous",
                ]
            )
        if bool(query_profile["wants_numbers"]):
            hint_parts.extend(["exact", "number", "value"])

        merged_parts: list[str] = []
        seen: set[str] = set()
        for part in token_parts + hint_parts:
            normalized = self._normalize_text(part).strip().lower()
            if not normalized or normalized in seen:
                continue
            merged_parts.append(normalized)
            seen.add(normalized)
        if not merged_parts:
            return None
        variant = " ".join(merged_parts)
        if variant == query.strip().lower():
            return None
        return variant

    def _term_overlap_count(self, query_terms: set[str], text: str) -> int:
        if not query_terms:
            return 0
        text_tokens = self._text_terms(text)
        return sum(1 for term in query_terms if term in text_tokens)

    def _detail_family_key(self, document_id: str) -> str:
        normalized_id = self._normalize_text(document_id).strip()
        if not normalized_id:
            return ""
        return re.sub(r"::(?:fact|part)::\d+$", "", normalized_id, flags=re.IGNORECASE)

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

    def _location_term_set(self, text: str) -> set[str]:
        lowered = self._normalize_text(text).lower()
        terms: set[str] = set()
        for match in _LOCATION_CLAUSE_PATTERN.finditer(lowered):
            candidate = self._normalize_text(match.group(1)).lower()
            if not candidate:
                continue
            tokens = [token for token in re.split(r"\s+", candidate) if token]
            if not tokens:
                continue
            if all(token in _LOCATION_NON_PLACE_TOKENS for token in tokens):
                continue
            if any(token in _LOCATION_NON_PLACE_TOKENS for token in tokens[:1]):
                continue
            terms.add(candidate)
        return terms

    def _text_has_generic_location_detail(self, text: str) -> bool:
        return bool(_GENERIC_LOCATION_DETAIL_PATTERN.search(text))

    def _location_detail_count(self, text: str) -> int:
        return len(self._location_term_set(text))

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
        return bool(re.search(r"[a-z]", answer_value, flags=re.IGNORECASE))

    def _detail_bonus(self, query_profile: dict[str, bool | str], text: str) -> int:
        score = 0
        if bool(query_profile["wants_numbers"]) and re.search(r"\d", text):
            score += 6
        if bool(query_profile["wants_date"]) and _DATE_DETAIL_PATTERN.search(text):
            score += 8
        if bool(query_profile["wants_location"]):
            location_count = self._location_detail_count(text)
            if location_count > 0:
                score += 9 + min(4, max(0, location_count - 1) * 2)
            elif self._text_has_generic_location_detail(text):
                score += 4
        if bool(query_profile["wants_item"]) and self._text_has_item_detail(text):
            score += 8
        if bool(query_profile["wants_item"]) and bool(query_profile["wants_location"]):
            if _STORE_BRAND_PATTERN.search(text):
                score += 10
        if bool(query_profile["wants_profile"]) and _PROFILE_DETAIL_PATTERN.search(text):
            score += 8
            if bool(query_profile["wants_previous_role"]):
                if _PREVIOUS_ROLE_DETAIL_PATTERN.search(text):
                    score += 14
                if _PREVIOUS_OCCUPATION_DETAIL_PATTERN.search(text):
                    score += 12
                if _PROFILE_TITLE_PATTERN.search(text):
                    score += 6
                if _PROFILE_COMPANY_PATTERN.search(text):
                    score += 4
                if _CURRENT_ACTIVITY_PATTERN.search(text):
                    score -= 12
        if bool(query_profile["wants_education"]) and _EDUCATION_DETAIL_PATTERN.search(text):
            score += 9
        if bool(query_profile["wants_event"]) and _EVENT_DETAIL_PATTERN.search(text):
            score += 8
        if bool(query_profile["wants_belief"]) and _BELIEF_DETAIL_PATTERN.search(text):
            score += 10
        return score

    def _document_variant_priority(self, document_id: str) -> int:
        lowered = self._normalize_text(document_id).lower()
        if not lowered:
            return 0
        priority = 0
        if "::fact::" in lowered:
            priority += 4
        if "::part::" in lowered:
            priority += 1
        return priority

    def _is_user_anchored_document(self, document: Document, lowered_text: str) -> bool:
        lowered_id = self._normalize_text(document.id).lower()
        lowered_context = self._normalize_text(document.context).lower()
        return bool(
            "::user::" in lowered_id
            or "::user::" in lowered_context
            or "\"role\": \"user\"" in lowered_text
            or "[user]" in lowered_text
            or re.search(r"\b(i|my|me)\b", lowered_text)
        )

    def _document_query_relevance_score(
        self,
        query: str,
        document: Document,
        *,
        query_profile: dict[str, bool | str] | None = None,
        query_terms: set[str] | None = None,
    ) -> int:
        profile = query_profile or self._build_query_profile(query)
        terms = query_terms
        if terms is None:
            term_query = self._as_text(profile.get("term_query")).strip() or query
            terms = self._query_terms(term_query)
        text = f"{self._normalize_text(document.content)}\n{self._normalize_text(document.context)}".strip()
        lowered = text.lower()
        overlap = self._term_overlap_count(terms, lowered)
        normalized_query = self._as_text(profile.get("normalized_query")).strip()
        phrase_bonus = 1 if normalized_query and normalized_query in lowered else 0
        detail_bonus = self._detail_bonus(profile, text)
        personal_bonus = 0
        if "\"role\": \"user\"" in lowered or "[user]" in lowered:
            personal_bonus += 3
        if re.search(r"\b(i|my|me)\b", lowered):
            personal_bonus += 2
        detail_user_bonus = (
            6
            if bool(profile["is_detail_query"]) and self._is_user_anchored_document(document, lowered)
            else 0
        )
        detail_non_user_penalty = (
            4
            if bool(profile["is_detail_query"]) and not self._is_user_anchored_document(document, lowered)
            else 0
        )
        return (
            (overlap * 10)
            + (phrase_bonus * 4)
            + detail_bonus
            + personal_bonus
            + detail_user_bonus
            + self._document_variant_priority(document.id)
            - detail_non_user_penalty
        )

    def _context_candidate_score(
        self,
        *,
        query_profile: dict[str, bool | str],
        query_terms: set[str],
        candidate: str,
    ) -> int:
        lowered = candidate.lower()
        overlap = self._term_overlap_count(query_terms, lowered)
        return (overlap * 10) + self._detail_bonus(query_profile, candidate)

    def _build_query_context_text(
        self,
        *,
        query: str,
        query_profile: dict[str, bool | str],
        query_terms: set[str],
        full_content: str,
        excerpt: str,
    ) -> str:
        _ = query
        normalized_full = self._normalize_text(full_content)
        normalized_excerpt = self._normalize_text(excerpt)
        if not normalized_excerpt:
            return normalized_full
        if not normalized_full:
            return normalized_excerpt

        excerpt_score = self._context_candidate_score(
            query_profile=query_profile,
            query_terms=query_terms,
            candidate=normalized_excerpt,
        )
        full_score = self._context_candidate_score(
            query_profile=query_profile,
            query_terms=query_terms,
            candidate=normalized_full,
        )
        if self.prefer_recall_excerpt:
            return normalized_excerpt if excerpt_score >= full_score else normalized_full
        if bool(query_profile["is_detail_query"]) and excerpt_score >= (full_score + 2):
            return normalized_excerpt
        return normalized_full

    def _expand_fact_family_candidates(
        self,
        *,
        query: str,
        query_profile: dict[str, bool | str],
        query_terms: set[str],
        documents: list[Document],
        user_id: str | None,
    ) -> list[Document]:
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

        sibling_pool: dict[str, list[Document]] = {family: [] for family in families}
        for stored in self.docs_by_context.values():
            if user_id is not None and stored.user_id != user_id:
                continue
            family = self._detail_family_key(stored.id)
            if family not in sibling_pool:
                continue
            if "::fact::" not in self._normalize_text(stored.id).lower():
                continue
            sibling_pool[family].append(
                Document(
                    id=stored.id,
                    content=stored.content,
                    user_id=stored.user_id,
                    timestamp=stored.timestamp,
                    context=stored.context,
                )
            )

        existing_ids = {self._normalize_text(document.id).lower() for document in documents}
        additions: list[Document] = []
        added_count = 0
        wants_location = bool(query_profile["wants_location"])
        for seed in documents:
            if added_count >= self.detail_max_added_siblings:
                break
            family = self._detail_family_key(seed.id)
            if family not in sibling_pool:
                continue
            seed_score = self._document_query_relevance_score(
                query,
                seed,
                query_profile=query_profile,
                query_terms=query_terms,
            )
            seed_detail = self._detail_bonus(query_profile, self._normalize_text(seed.content))
            seed_location_count = self._location_detail_count(seed.content) if wants_location else 0
            seed_fact_index = self._fact_index(seed.id)
            sibling_candidates: list[tuple[int, int, int, int, Document]] = []
            for sibling in sibling_pool[family]:
                sibling_id_key = self._normalize_text(sibling.id).lower()
                if not sibling_id_key or sibling_id_key in existing_ids:
                    continue
                sibling_score = self._document_query_relevance_score(
                    query,
                    sibling,
                    query_profile=query_profile,
                    query_terms=query_terms,
                )
                sibling_detail = self._detail_bonus(
                    query_profile,
                    self._normalize_text(sibling.content),
                )
                sibling_location_count = self._location_detail_count(sibling.content) if wants_location else 0
                sibling_fact_index = self._fact_index(sibling.id)
                adjacent_fact = (
                    seed_fact_index is not None
                    and sibling_fact_index is not None
                    and abs(sibling_fact_index - seed_fact_index) <= 1
                )
                if (
                    sibling_score < (seed_score - self.detail_sibling_score_margin)
                    and sibling_detail <= seed_detail
                    and not adjacent_fact
                    and not (wants_location and sibling_location_count > seed_location_count)
                ):
                    continue
                sibling_rank = sibling_score
                if sibling_detail > seed_detail:
                    sibling_rank += 4
                if adjacent_fact:
                    sibling_rank += 6
                if wants_location and sibling_location_count > seed_location_count:
                    sibling_rank += min(6, (sibling_location_count - seed_location_count) * 2)
                sibling_candidates.append(
                    (
                        sibling_rank,
                        sibling_score,
                        sibling_detail,
                        sibling_location_count,
                        sibling,
                    )
                )
            if not sibling_candidates:
                continue
            sibling_candidates.sort(
                reverse=True,
                key=lambda item: (item[0], item[2], item[3], item[1], self._normalize_text(item[4].id)),
            )
            per_seed_limit = self.detail_siblings_per_seed + (1 if wants_location else 0)
            for _rank, _score, _detail, _location_count, sibling in sibling_candidates[:per_seed_limit]:
                sibling_id_key = self._normalize_text(sibling.id).lower()
                if not sibling_id_key or sibling_id_key in existing_ids:
                    continue
                additions.append(sibling)
                existing_ids.add(sibling_id_key)
                added_count += 1
                if added_count >= self.detail_max_added_siblings:
                    break
        if not additions:
            return documents
        return documents + additions

    def _looks_like_question_text(self, text: str) -> bool:
        normalized = self._normalize_text(text).lower()
        if not normalized:
            return False
        if "[user-answer]" in normalized:
            return False
        return "?" in normalized

    def _is_country_like_location_term(self, term: str) -> bool:
        tokens = [
            token.lower()
            for token in re.findall(r"[a-z0-9'&.-]{2,}", self._normalize_text(term), flags=re.IGNORECASE)
        ]
        if not tokens or len(tokens) > 3:
            return False
        if any(token in _LOCATION_NON_PLACE_TOKENS for token in tokens):
            return False
        if any(token in _LOCATION_PLACE_HINT_TOKENS for token in tokens):
            return False
        return True

    def _promote_location_family_complement(
        self,
        *,
        query: str,
        documents: list[Document],
        k: int,
    ) -> list[Document]:
        if k <= 1 or len(documents) <= 1:
            return documents
        query_profile = self._build_query_profile(query)
        if not bool(query_profile["wants_location"]):
            return documents
        wants_item = bool(query_profile["wants_item"])
        top_window = len(documents) if wants_item else min(k, len(documents))
        primary_family = self._detail_family_key(documents[0].id)
        primary_terms = self._location_term_set(documents[0].content)
        best_index: int | None = None
        best_rank: tuple[int, int, int, int, int] | None = None
        for idx, document in enumerate(documents[1:top_window], start=1):
            location_terms = self._location_term_set(document.content) - primary_terms
            if not location_terms:
                continue
            same_family = (
                wants_item
                and bool(primary_family)
                and self._detail_family_key(document.id) == primary_family
            )
            rank = (
                1 if same_family else 0,
                0 if self._looks_like_question_text(document.content) else 1,
                len(location_terms),
                1 if self._is_non_generic_location_text(document.content) else 0,
                self._document_query_relevance_score(query, document),
            )
            if best_rank is None or rank > best_rank:
                best_rank = rank
                best_index = idx
        if best_index is None or best_index <= 1:
            return documents
        promoted = documents[best_index]
        reordered: list[Document] = [documents[0], promoted]
        for idx, document in enumerate(documents):
            if idx in {0, best_index}:
                continue
            reordered.append(document)
        return reordered

    def _is_non_generic_location_text(self, text: str) -> bool:
        return self._location_detail_count(text) > 0 and not self._text_has_generic_location_detail(text)

    def _augment_item_location_qualifier(
        self,
        *,
        query: str,
        documents: list[Document],
    ) -> list[Document]:
        if len(documents) <= 1:
            return documents
        query_profile = self._build_query_profile(query)
        if not (bool(query_profile["wants_item"]) and bool(query_profile["wants_location"])):
            return documents
        primary = documents[0]
        primary_text = self._normalize_text(primary.content).strip()
        if not primary_text:
            return documents
        if "[location-qualifier]" in primary_text.lower():
            return documents
        if not re.search(r"\b(?:redeem|redeemed|coupon|buy|bought|purchase|purchased|get|got)\b", primary_text, re.IGNORECASE):
            return documents
        primary_family = self._detail_family_key(primary.id)
        primary_terms = self._location_term_set(primary_text)
        best_store: tuple[int, str] | None = None
        for candidate in documents[1:]:
            candidate_text = self._normalize_text(candidate.content).strip()
            if not candidate_text or self._looks_like_question_text(candidate_text):
                continue
            same_family = bool(primary_family) and self._detail_family_key(candidate.id) == primary_family
            relevance = self._document_query_relevance_score(query, candidate)
            for match in _STORE_BRAND_PATTERN.finditer(candidate_text):
                store_term = self._normalize_text(match.group(0)).strip()
                if not store_term:
                    continue
                store_term_lower = store_term.lower()
                if store_term_lower in primary_text.lower():
                    continue
                rank = relevance + (14 if same_family else 0)
                if best_store is None or rank > best_store[0]:
                    best_store = (rank, store_term)
        if best_store is None:
            return documents
        store_value = best_store[1]
        normalized_store = store_value.title()
        if store_value.isupper() and len(store_value) <= 4:
            normalized_store = store_value.upper()
        elif "'" in store_value:
            normalized_store = " ".join(part.capitalize() for part in store_value.split())
        qualifier_text = f"at {normalized_store}"
        augmented_text = f"{primary_text.rstrip()} [location-qualifier] {qualifier_text}."
        augmented_primary = Document(
            id=primary.id,
            content=augmented_text,
            user_id=primary.user_id,
            timestamp=primary.timestamp,
            context=primary.context,
        )
        return [augmented_primary, *documents[1:]]

    def _augment_abroad_location_qualifier(
        self,
        *,
        query: str,
        documents: list[Document],
    ) -> list[Document]:
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
        if "[location-qualifier]" in primary_text.lower():
            return documents
        primary_terms = self._location_term_set(primary_text)
        candidate_terms = {
            term for term in primary_terms if self._is_country_like_location_term(term)
        }
        for document in documents[1:]:
            for term in self._location_term_set(document.content):
                if term in primary_terms:
                    continue
                if self._is_country_like_location_term(term):
                    candidate_terms.add(term)
        if not candidate_terms:
            return documents
        selected_term = sorted(candidate_terms, key=lambda term: (len(term.split()), len(term), term))[0]
        qualifier_text = f"in {selected_term.title()}"
        augmented_text = f"{primary_text.rstrip()} [location-qualifier] {qualifier_text}."
        augmented_primary = Document(
            id=primary.id,
            content=augmented_text,
            user_id=primary.user_id,
            timestamp=primary.timestamp,
            context=primary.context,
        )
        return [augmented_primary, *documents[1:]]

    def _rerank_documents(self, query: str, documents: list[Document]) -> list[Document]:
        if len(documents) <= 1:
            return documents
        query_profile = self._build_query_profile(query)
        term_query = self._as_text(query_profile.get("term_query")).strip() or query
        query_terms = self._query_terms(term_query)
        scored: list[tuple[int, int, Document]] = []
        for idx, document in enumerate(documents):
            score = self._document_query_relevance_score(
                query,
                document,
                query_profile=query_profile,
                query_terms=query_terms,
            )
            scored.append((score, -idx, document))
        scored.sort(reverse=True, key=lambda item: (item[0], item[1]))
        return [item[2] for item in scored]

    def _record_recall_metrics(
        self,
        *,
        query: str,
        payload: RecallResponse,
        user_id: str | None,
        source_prefix: str | None,
        requested_budget: int | None = None,
    ) -> None:
        if not self.metrics_file:
            return
        results = payload.get("results")
        if not isinstance(results, list):
            results = []
        token_estimate = sum(
            int(item.get("tokens", 0))
            for item in results
            if isinstance(item, dict)
        )
        record = {
            "query": query,
            "user_id": user_id,
            "source_prefix": source_prefix,
            "budget": int(requested_budget or self.budget),
            "result_count": len(results),
            "token_estimate": token_estimate,
            "source_count": len(results),
            "sample_sources": self._sample_sources(results),
            "recall_call_count": 1,
            "recall_variant_queries": [],
            "combined_token_estimate": token_estimate,
        }
        path = Path(self.metrics_file)
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, ensure_ascii=True))
            handle.write("\n")

    @staticmethod
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

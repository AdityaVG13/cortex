from __future__ import annotations

import json
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
        self._enable_fact_extracts = os.environ.get(
            "CORTEX_BENCHMARK_ENABLE_FACT_EXTRACTS",
            "1",
        ).strip().lower() not in {"0", "false", "no"}
        self._store_full_docs = os.environ.get(
            "CORTEX_BENCHMARK_STORE_FULL_DOCS",
            "1",
        ).strip().lower() not in {"0", "false", "no"}
        self._max_fact_extracts_per_doc = max(
            0,
            int(os.environ.get("CORTEX_BENCHMARK_MAX_FACT_EXTRACTS_PER_DOC", "12")),
        )
        self._fact_extract_max_chars = max(
            120,
            int(os.environ.get("CORTEX_BENCHMARK_FACT_EXTRACT_MAX_CHARS", "640")),
        )

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
        for document in documents:
            self._pending_docs.extend(self._expand_document(document))
        self._flush_pending(force=False)

    @staticmethod
    def _normalize_text(value: object | None) -> str:
        if value is None:
            return ""
        if isinstance(value, str):
            return value
        return str(value)

    def _expand_document(self, document: Document) -> list[CortexStoredDocument]:
        base_content = self._normalize_text(document.content)
        expanded: list[CortexStoredDocument] = []
        if self._store_full_docs:
            expanded.append(
                CortexStoredDocument(
                    id=document.id,
                    content=base_content,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=document.context,
                )
            )
        if not self._enable_fact_extracts or self._max_fact_extracts_per_doc <= 0:
            if not expanded:
                expanded.append(
                    CortexStoredDocument(
                        id=document.id,
                        content=base_content,
                        user_id=document.user_id,
                        timestamp=document.timestamp,
                        context=document.context,
                    )
                )
            return expanded
        for idx, snippet in enumerate(self._extract_fact_snippets(base_content), start=1):
            expanded.append(
                CortexStoredDocument(
                    id=f"{document.id}::fact::{idx}",
                    content=snippet,
                    user_id=document.user_id,
                    timestamp=document.timestamp,
                    context=f"{self._normalize_text(document.context)} [fact-extract]".strip(),
                )
            )
            if idx >= self._max_fact_extracts_per_doc:
                break
        if not expanded:
            expanded.append(
                CortexStoredDocument(
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
            r"\bi\s+(?:was|am|have|had|got|bought|took|attended|graduated|upgraded|packed|changed|remember|used to|redeemed|repainted|volunteered|worked|studied|moved)\b"
        )
        date_measure_pattern = re.compile(
            r"\b(?:19|20)\d{2}\b"
            r"|\b(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\b"
            r"|\b\d{1,2}[/-]\d{1,2}(?:[/-]\d{2,4})?\b"
            r"|\b\d+(?:\.\d+)?\s*(?:kbps|mbps|gbps|minutes?|hours?|days?|weeks?|months?|years?|km|miles?|dollars?)\b",
            re.IGNORECASE,
        )
        location_pattern = re.compile(
            r"\b(?:at|in|from|to|near)\s+[A-Z][A-Za-z0-9'-]*(?:\s+[A-Z][A-Za-z0-9'-]*){0,2}\b"
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

        def _clip_snippet(value: str) -> str:
            if len(value) <= self._fact_extract_max_chars:
                return value
            if self._fact_extract_max_chars <= 32:
                return value[: self._fact_extract_max_chars]
            half = max(12, (self._fact_extract_max_chars - 5) // 2)
            return f"{value[:half].rstrip()} ... {value[-half:].lstrip()}"

        snippets: list[str] = []
        seen: set[str] = set()
        ranked_candidates: list[tuple[int, int, str]] = []
        sequence = 0
        parsed_turns: list[tuple[str, str]] = []
        for turn in payload:
            if not isinstance(turn, dict):
                continue
            message_text = self._normalize_text(turn.get("content") or turn.get("text"))
            if not message_text:
                continue
            role = self._normalize_text(turn.get("role") or turn.get("speaker")).strip().lower()
            normalized = re.sub(r"\s+", " ", message_text).strip().replace("\u2019", "'")
            if len(normalized) < 20:
                continue
            parsed_turns.append((role, normalized))

        if not parsed_turns:
            return []

        user_turns = [(role, text) for role, text in parsed_turns if role == "user"]
        candidate_turns = user_turns if user_turns else parsed_turns
        for role, normalized in candidate_turns:
            role_prefix = role if role else "message"
            role_score = 6 if role_prefix == "user" else 1
            for sentence in sentence_split.split(normalized):
                compact = sentence.strip()
                if len(compact) < 24:
                    continue
                marker_haystack = f" {compact.lower()} "
                if role_prefix == "user":
                    if not any(marker in marker_haystack for marker in personal_markers):
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
                if low_signal_pattern.search(compact):
                    score -= 4
                if len(compact) <= 220:
                    score += 1
                if score <= 0:
                    continue

                snippet_text = _clip_snippet(compact)
                snippet = f"[{role_prefix}] {snippet_text}".strip()
                ranked_candidates.append((score, sequence, snippet))
                sequence += 1

        if not ranked_candidates:
            for role, normalized in candidate_turns[:4]:
                marker_haystack = f" {normalized.lower()} "
                role_prefix = role if role else "message"
                role_score = 2 if role_prefix == "user" else 0
                if role_score <= 0:
                    continue
                if not any(marker in marker_haystack for marker in personal_markers):
                    continue
                snippet_text = _clip_snippet(normalized)
                snippet = f"[{role_prefix}] {snippet_text}".strip()
                ranked_candidates.append((role_score, sequence, snippet))
                sequence += 1

        if user_turns:
            for idx, (role, normalized) in enumerate(parsed_turns):
                if role != "assistant":
                    continue
                if idx <= 0 or parsed_turns[idx - 1][0] != "user":
                    continue
                for sentence in sentence_split.split(normalized):
                    compact = sentence.strip()
                    if len(compact) < 24 or len(compact) > 260:
                        continue
                    marker_haystack = f" {compact.lower()} "
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
                    snippet_text = _clip_snippet(compact)
                    snippet = f"[assistant] {snippet_text}".strip()
                    ranked_candidates.append((score, sequence, snippet))
                    sequence += 1

        ranked_candidates.sort(key=lambda item: (-item[0], item[1]))
        for _score, _seq, snippet in ranked_candidates:
            if snippet in seen:
                continue
            seen.add(snippet)
            snippets.append(snippet)
            if len(snippets) >= self._max_fact_extracts_per_doc:
                break
        return snippets

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

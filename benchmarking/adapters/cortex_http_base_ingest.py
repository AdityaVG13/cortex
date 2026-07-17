"""Document ingest mixin for CortexHTTPBaseMemoryProvider."""
from __future__ import annotations

import json
import re
from hashlib import sha1

from memory_bench.models import Document
from recall_tuning.base_patterns import *  # noqa: F403

class CortexHTTPBaseIngestMixin:
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


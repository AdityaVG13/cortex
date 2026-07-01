"""Retrieve and rerank mixin for CortexHTTPBaseMemoryProvider."""
from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import cast

from cortex_http_types import RecallResponse
from memory_bench.models import Document
from recall_tuning.base_patterns import *  # noqa: F403

class CortexHTTPBaseRecallMixin:
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
        # Strict benchmark runs must honor the configured token budget for every query.
        recall_budget = self.budget
        recall_query = query
        use_detail_variant = (
            bool(query_profile["wants_profile"])
            or bool(query_profile["wants_name"])
            or bool(query_profile["wants_education"])
            or bool(query_profile["wants_belief"])
            or bool(query_profile["wants_item"])
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

    def _normalize_text(self, value: object | None) -> str:
        return re.sub(r"\s+", " ", self._as_text(value)).strip()

    def _build_query_profile(self, query: str) -> dict[str, bool | str]:
        normalized_query = self._normalize_text(query).lower()
        wants_numbers = bool(_QUERY_NUMERIC_INTENT_PATTERN.search(normalized_query))
        wants_location = bool(_QUERY_LOCATION_INTENT_PATTERN.search(normalized_query))
        wants_date = bool(_QUERY_DATE_INTENT_PATTERN.search(normalized_query))
        wants_item = bool(_QUERY_ITEM_INTENT_PATTERN.search(normalized_query))
        wants_profile = bool(_QUERY_PROFILE_INTENT_PATTERN.search(normalized_query))
        wants_name = bool(_QUERY_NAME_INTENT_PATTERN.search(normalized_query))
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
        if wants_name and re.search(r"\b(previous|former|prior|used to|earlier|before|old|changed)\b", normalized_query):
            term_query = (
                f"{term_query} last name surname old name previous name changed from called"
            ).strip()
        is_detail_query = (
            wants_numbers
            or wants_location
            or wants_date
            or wants_item
            or wants_profile
            or wants_name
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
            "wants_name": wants_name,
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
            hint_parts.extend(["item", "purchase", "redeemed", "store", "color", "paint", "repainted", "exact detail"])
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
        if bool(query_profile["wants_name"]):
            hint_parts.extend(["name", "last name", "surname", "called", "old name", "previous name"])
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

    def _text_has_name_detail(self, text: str) -> bool:
        return bool(_NAME_DETAIL_PATTERN.search(text))

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
            if re.search(
                r"\b(gift|gifts|present|item|items|thing|things|something|stuff)\b",
                answer_lower,
            ):
                bonus -= 2
            if answer_words and len(answer_words) <= 5 and not re.search(
                r"\b(i|my|we|bought|purchased|redeemed|ordered|got)\b",
                answer_lower,
            ):
                bonus += 6
        if _QUERY_BIRTHDAY_DESCRIPTOR_PATTERN.search(str(query_profile["normalized_query"])):
            bonus += 2
        return bonus

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
        if bool(query_profile["wants_name"]) and self._text_has_name_detail(text):
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
        item_answer_bonus = self._item_answer_specificity_bonus(
            query_profile=profile,
            text=text,
            overlap=overlap,
            detail_bonus=detail_bonus,
        )
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
            + item_answer_bonus
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
        term_query = self._as_text(query_profile.get("term_query")).strip() or query
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


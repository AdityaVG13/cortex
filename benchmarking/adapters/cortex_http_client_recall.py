"""Recall orchestration mixin for CortexHTTPClient."""
from __future__ import annotations

import json
import re
from pathlib import Path
from typing import cast

from cortex_http_types import RecallResponse
from cortex_http_models import CortexStoredDocument
from recall_tuning.client_patterns import *  # noqa: F401,F403

class CortexHTTPClientRecallMixin:
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

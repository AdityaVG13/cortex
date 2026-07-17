"""Query scoring and reranking mixin for CortexHTTPClient."""
from __future__ import annotations

import re

from recall_tuning.client_patterns import *  # noqa: F403

class CortexHTTPClientScoringMixin:
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

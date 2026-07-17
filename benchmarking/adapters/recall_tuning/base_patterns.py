"""Query/detail regex patterns for the cortex-http-base provider adapter."""
from __future__ import annotations

import re

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
    r"\b(?:gift|present|item|buy|bought|purchase|redeem|redeemed|model|brand|color|paint|painted|repaint|repainted|wall|walls)\b",
    re.IGNORECASE,
)
_QUERY_BIRTHDAY_DESCRIPTOR_PATTERN = re.compile(
    r"\bbirthday\s+(gift|present|party|card|dinner|cake|trip|message|wishlist)\b",
    re.IGNORECASE,
)
_QUERY_NAME_INTENT_PATTERN = re.compile(
    r"\b(?:name|called|last name|first name|old name|previous name|surname)\b",
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
_NAME_DETAIL_PATTERN = re.compile(
    r"\b(?:name\s+(?:is|was)|called|old name was|last name(?:\s+was)?|surname(?:\s+was)?)\s+[A-Z][A-Za-z0-9'-]+(?:\s+[A-Z][A-Za-z0-9'-]+){0,2}\b",
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
    "coast",
    "ocean",
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
    "name": {"last", "first", "surname", "called", "previous", "old"},
    "last": {"name", "surname", "family"},
    "surname": {"last", "name", "family", "changed"},
    "color": {"paint", "repaint", "shade"},
    "paint": {"color", "repaint", "walls"},
    "repaint": {"paint", "color", "walls"},
}
_TOKEN_SUFFIX_RULES: tuple[str, ...] = ("'s", "ing", "ed", "ers", "er", "es", "s")

__all__ = [
    "_ASCII_TOKEN_PATTERN",
    "_QUERY_STOPWORDS",
    "_QUERY_NUMERIC_INTENT_PATTERN",
    "_QUERY_LOCATION_INTENT_PATTERN",
    "_QUERY_DATE_INTENT_PATTERN",
    "_QUERY_ITEM_INTENT_PATTERN",
    "_QUERY_BIRTHDAY_DESCRIPTOR_PATTERN",
    "_QUERY_NAME_INTENT_PATTERN",
    "_QUERY_PROFILE_INTENT_PATTERN",
    "_QUERY_EDUCATION_INTENT_PATTERN",
    "_QUERY_EVENT_INTENT_PATTERN",
    "_QUERY_BELIEF_INTENT_PATTERN",
    "_QUERY_ABROAD_INTENT_PATTERN",
    "_DATE_DETAIL_PATTERN",
    "_PROFILE_DETAIL_PATTERN",
    "_PROFILE_TITLE_PATTERN",
    "_PROFILE_COMPANY_PATTERN",
    "_PREVIOUS_OCCUPATION_DETAIL_PATTERN",
    "_EDUCATION_DETAIL_PATTERN",
    "_EVENT_DETAIL_PATTERN",
    "_BELIEF_DETAIL_PATTERN",
    "_NAME_DETAIL_PATTERN",
    "_PREVIOUS_ROLE_DETAIL_PATTERN",
    "_CURRENT_ACTIVITY_PATTERN",
    "_GENERIC_LOCATION_DETAIL_PATTERN",
    "_ITEM_DETAIL_PATTERN",
    "_STORE_BRAND_PATTERN",
    "_LOCATION_CLAUSE_PATTERN",
    "_LOCATION_NON_PLACE_TOKENS",
    "_LOCATION_PLACE_HINT_TOKENS",
]

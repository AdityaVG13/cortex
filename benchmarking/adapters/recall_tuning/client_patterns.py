"""Query/detail regex patterns for the tuned cortex-http client adapter."""
from __future__ import annotations

import re

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

__all__ = [
    "_QUERY_STOPWORDS",
    "_QUERY_NUMERIC_INTENT_PATTERN",
    "_QUERY_LOCATION_INTENT_PATTERN",
    "_QUERY_DATE_INTENT_PATTERN",
    "_QUERY_BIRTHDAY_DESCRIPTOR_PATTERN",
    "_QUERY_SPEED_INTENT_PATTERN",
    "_QUERY_ITEM_INTENT_PATTERN",
    "_QUERY_NAME_INTENT_PATTERN",
    "_QUERY_OCCUPATION_INTENT_PATTERN",
    "_QUERY_PREVIOUS_ROLE_INTENT_PATTERN",
    "_QUERY_PERSONAL_PATTERN",
    "_RELATION_TERM_PATTERN",
    "_RELATION_TERM_CANONICAL",
    "_RELATION_CONFLICT_GROUPS",
    "_DATE_DETAIL_PATTERN",
    "_DATE_EXACT_DETAIL_PATTERN",
    "_LOCATION_DETAIL_PATTERN",
    "_GENERIC_LOCATION_DETAIL_PATTERN",
    "_LOCATION_ABBREV_DETAIL_PATTERN",
    "_STANDALONE_LOCATION_DETAIL_PATTERN",
    "_SPEED_DETAIL_PATTERN",
    "_ITEM_DETAIL_PATTERN",
    "_LOCATION_PURCHASE_CUE_PATTERN",
    "_QUERY_ABROAD_INTENT_PATTERN",
    "_NAME_DETAIL_PATTERN",
    "_OCCUPATION_DETAIL_PATTERN",
    "_PREVIOUS_ROLE_DETAIL_PATTERN",
    "_CURRENT_ROLE_DETAIL_PATTERN",
    "_ASSISTANT_ROLE_PATTERN",
    "_ASSISTANT_MIRROR_FACT_PATTERN",
    "_LOW_SIGNAL_ASSISTANT_PATTERN",
    "_ASCII_TOKEN_PATTERN",
    "_UNICODE_TOKEN_PATTERN",
    "_CJK_CHAR_PATTERN",
    "_QUERY_MCQ_OPTION_PATTERN",
    "_LOCATION_TOKEN_PATTERN",
    "_ANSWER_SOURCE_ID_PATTERN",
    "_LOCATION_NON_PLACE_TOKENS",
    "_LOCATION_PLACE_HINT_TOKENS",
]

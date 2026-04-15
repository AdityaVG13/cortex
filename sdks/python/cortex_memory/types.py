"""Typed payloads for the Cortex Python SDK."""

from __future__ import annotations

from typing import TypedDict


class RecallResultItemBase(TypedDict):
    source: str
    relevance: float
    excerpt: str
    method: str


class RecallResultItem(RecallResultItemBase, total=False):
    tokens: int


class RecallResponseBase(TypedDict):
    results: list[RecallResultItem]
    budget: int
    spent: int
    saved: int


class RecallResponse(RecallResponseBase, total=False):
    mode: str
    tier: str
    cached: bool
    latencyMs: int
    count: int
    semanticAvailable: bool


class PeekMatch(TypedDict):
    source: str
    relevance: float
    method: str


class PeekResponse(TypedDict):
    count: int
    matches: list[PeekMatch]


class StoreConflict(TypedDict, total=False):
    status: str
    conflict_record_id: int
    classification: str
    source_decision_id: int | None
    target_decision_id: int | None
    resolution_strategy: str | None
    resolved_by: str | None
    resolved_at: str | None
    similarity_jaccard: float
    similarity_cosine: float


class StoreEntry(TypedDict, total=False):
    stored: bool
    action: str
    id: int
    reason: str
    surprise: float
    quality: int
    status: str
    classification: str
    target_id: int
    conflictWith: int
    resolution_strategy: str
    supersedes: int
    conflict: StoreConflict


class StoreResponse(TypedDict):
    stored: bool
    entry: StoreEntry


class HealthStats(TypedDict):
    memories: int
    decisions: int
    embeddings: int
    events: int
    home: str


class HealthRuntime(TypedDict):
    version: str
    mode: str
    port: int
    db_path: str
    token_path: str
    pid_path: str
    ipc_endpoint: str | None
    ipc_kind: str | None
    executable: str
    owner: str | None


class HealthResponse(TypedDict):
    status: str
    ready: bool
    degraded: bool
    db_corrupted: bool
    embedding_status: str
    team_mode: bool
    db_freelist_pages: int
    db_size_bytes: int
    db_soft_limit_bytes: int
    db_hard_limit_bytes: int
    db_pressure: str
    db_soft_utilization: float
    storage_bytes: int
    backup_count: int
    log_bytes: int
    stats: HealthStats
    runtime: HealthRuntime


class ExportRowBase(TypedDict):
    id: int
    source_agent: str | None
    source_client: str | None
    source_model: str | None
    confidence: float | None
    reasoning_depth: str | None
    trust_score: float | None
    status: str | None
    score: float | None
    retrievals: int | None
    pinned: int | None
    created_at: str | None
    updated_at: str | None


class ExportMemoryRow(ExportRowBase):
    text: str
    source: str | None
    type: str | None
    tags: str | None


class ExportDecisionRow(ExportRowBase):
    decision: str
    context: str | None
    type: str | None


class ExportResponse(TypedDict):
    version: int
    exported_at: str
    memories: list[ExportMemoryRow]
    decisions: list[ExportDecisionRow]
    memories_count: int
    decisions_count: int


class BootCapsule(TypedDict, total=False):
    name: str
    tokens: int
    priority: float
    utility: float
    truncated: bool


class BootSavings(TypedDict):
    rawBaseline: int
    served: int
    saved: int
    percent: int


class BootResponse(TypedDict):
    bootPrompt: str
    tokenEstimate: int
    profile: str
    capsules: list[BootCapsule]
    savings: BootSavings


class ImportMemoryBase(TypedDict):
    text: str


class ImportMemory(ImportMemoryBase, total=False):
    source: str
    type: str
    tags: str
    source_agent: str
    source_client: str
    source_model: str
    confidence: float
    reasoning_depth: str
    trust_score: float
    score: float


class ImportDecisionBase(TypedDict):
    decision: str


class ImportDecision(ImportDecisionBase, total=False):
    context: str
    type: str
    source_agent: str
    source_client: str
    source_model: str
    confidence: float
    reasoning_depth: str
    trust_score: float
    score: float


class ImportPayload(TypedDict, total=False):
    memories: list[ImportMemory]
    decisions: list[ImportDecision]


class StoreRequest(TypedDict, total=False):
    decision: str
    context: str
    type: str
    source_agent: str
    source_model: str
    confidence: float
    reasoning_depth: str
    ttl_seconds: int


class ImportedCounts(TypedDict):
    memories: int
    decisions: int


class ImportResponse(TypedDict):
    imported: ImportedCounts


class DiaryResponse(TypedDict):
    written: bool
    agent: str
    path: str


class ForgetResponse(TypedDict):
    affected: int


class ShutdownResponse(TypedDict):
    shutdown: bool

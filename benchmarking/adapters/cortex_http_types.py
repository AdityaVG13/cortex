"""Typed payloads shared by the Cortex benchmarking HTTP adapter."""

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

"""Core Cortex client using httpx for async/sync HTTP calls."""

# SPDX-License-Identifier: MIT

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Optional, cast
from urllib.parse import urlparse

import httpx

from .types import (
    BootResponse,
    DiaryResponse,
    ExportResponse,
    ForgetResponse,
    HealthResponse,
    ImportPayload,
    ImportResponse,
    PeekResponse,
    RecallResponse,
    ShutdownResponse,
    StoreResponse,
)

_DEFAULT_BASE = "http://127.0.0.1:7437"
_CORTEX_HEADERS = {"X-Cortex-Request": "true"}


def _read_token() -> Optional[str]:
    home = Path(os.environ.get("USERPROFILE", os.environ.get("HOME", ".")))
    token_path = home / ".cortex" / "cortex.token"
    try:
        return token_path.read_text().strip() or None
    except (FileNotFoundError, PermissionError):
        return None


def _is_loopback_base_url(base_url: str) -> bool:
    try:
        parsed = urlparse(base_url)
    except ValueError:
        return False
    if parsed.scheme not in ("http", "https"):
        return False
    host = (parsed.hostname or "").lower()
    return host in {"127.0.0.1", "localhost", "::1"}


class CortexClient:
    """Synchronous + async Python client for the Cortex daemon.

    Usage::

        from cortex_memory import CortexClient

        client = CortexClient()
        health = client.health()
        results = client.recall("What is Cortex?", budget=200)
        client.store("New decision", source_agent="my-script")
    """

    def __init__(
        self,
        base_url: str = _DEFAULT_BASE,
        token: Optional[str] = None,
        timeout: float = 10.0,
        source_agent: str = "python-sdk",
    ):
        self.base_url = base_url.rstrip("/")
        if token:
            self.token = token
        elif _is_loopback_base_url(self.base_url):
            self.token = _read_token()
        else:
            raise ValueError(
                "Remote Cortex base_url requires explicit token. "
                "Pass token=... when using non-loopback targets."
            )
        self.timeout = timeout
        normalized_source_agent = source_agent.strip()
        self.source_agent = normalized_source_agent or "python-sdk"

    def _headers(self) -> dict[str, str]:
        h = dict(_CORTEX_HEADERS)
        h["X-Source-Agent"] = self.source_agent
        if self.token:
            h["Authorization"] = f"Bearer {self.token}"
        return h

    def _get(self, path: str, params: Optional[dict[str, object]] = None) -> dict[str, object]:
        with httpx.Client(timeout=self.timeout) as c:
            resp = c.get(
                f"{self.base_url}{path}",
                headers=self._headers(),
                params=params,
            )
            resp.raise_for_status()
            return resp.json()

    def _post(self, path: str, json: Optional[dict[str, object]] = None) -> dict[str, object]:
        with httpx.Client(timeout=self.timeout) as c:
            resp = c.post(
                f"{self.base_url}{path}",
                headers=self._headers(),
                json=json or {},
            )
            resp.raise_for_status()
            return resp.json()

    # ── Public API ──────────────────────────────────────────────────

    def health(self) -> HealthResponse:
        """Check daemon health (no auth required)."""
        with httpx.Client(timeout=self.timeout) as c:
            resp = c.get(f"{self.base_url}/health")
            resp.raise_for_status()
            return cast(HealthResponse, resp.json())

    def recall(
        self,
        query: str,
        budget: int = 200,
        k: int = 10,
        agent: Optional[str] = None,
    ) -> RecallResponse:
        params: dict[str, object] = {"q": query, "budget": budget, "k": k}
        if agent:
            params["agent"] = agent
        return cast(RecallResponse, self._get("/recall", params))

    def format_recall_context(
        self,
        recall: RecallResponse,
        *,
        include_metrics: bool = True,
        max_items: Optional[int] = None,
    ) -> str:
        """
        Build a prompt-ready context string from recall payloads.

        Content is always prioritized: each returned excerpt is preserved as
        primary context. Retrieval telemetry is appended as a compact trailing
        line, never substituted for the memory text.
        """
        entries: list[str] = []
        results = recall.get("results") or []
        limit = len(results) if max_items is None else max(0, max_items)
        for index, item in enumerate(results):
            if index >= limit:
                break
            excerpt = str(item.get("excerpt", "")).strip()
            if not excerpt:
                continue
            source = str(item.get("source", "")).strip()
            method = str(item.get("method", "")).strip()
            label_parts: list[str] = []
            if source:
                label_parts.append(f"source={source}")
            if method:
                label_parts.append(f"method={method}")
            suffix = f" ({', '.join(label_parts)})" if label_parts else ""
            entries.append(f"## Memory {len(entries) + 1}{suffix}\n{excerpt}")
        if include_metrics:
            metrics = {
                key: recall.get(key)
                for key in ("budget", "spent", "saved", "count", "mode", "tier", "cached", "latencyMs")
                if recall.get(key) is not None
            }
            if metrics:
                entries.append(f"[retrieval-metrics] {json.dumps(metrics, ensure_ascii=False)}")
        return "\n\n".join(entries).strip()

    def recall_for_prompt(
        self,
        query: str,
        budget: int = 200,
        k: int = 10,
        agent: Optional[str] = None,
        *,
        include_metrics: bool = True,
        max_items: Optional[int] = None,
    ) -> str:
        """Convenience helper: run recall and return prompt-ready context."""
        payload = self.recall(query=query, budget=budget, k=k, agent=agent)
        return self.format_recall_context(
            payload,
            include_metrics=include_metrics,
            max_items=max_items,
        )

    def peek(self, query: str, k: int = 10) -> PeekResponse:
        return cast(PeekResponse, self._get("/peek", {"q": query, "k": k}))

    def store(
        self,
        decision: str,
        context: Optional[str] = None,
        source_agent: Optional[str] = None,
        source_model: Optional[str] = None,
        confidence: Optional[float] = None,
        reasoning_depth: Optional[str] = None,
        ttl_seconds: Optional[int] = None,
        entry_type: Optional[str] = None,
    ) -> StoreResponse:
        normalized_source_agent = (source_agent or self.source_agent).strip() or self.source_agent
        body: dict[str, object] = {"decision": decision, "source_agent": normalized_source_agent}
        if context is not None:
            body["context"] = context
        if source_model is not None:
            body["source_model"] = source_model
        if confidence is not None:
            body["confidence"] = confidence
        if reasoning_depth is not None:
            body["reasoning_depth"] = reasoning_depth
        if ttl_seconds is not None:
            body["ttl_seconds"] = ttl_seconds
        if entry_type is not None:
            body["type"] = entry_type
        return cast(StoreResponse, self._post("/store", body))

    def diary(self, text: str, agent: Optional[str] = None) -> DiaryResponse:
        normalized_agent = (agent or self.source_agent).strip() or self.source_agent
        return cast(DiaryResponse, self._post("/diary", {"text": text, "agent": normalized_agent}))

    def boot(self, agent: Optional[str] = None, budget: int = 600) -> BootResponse:
        normalized_agent = (agent or self.source_agent).strip() or self.source_agent
        return cast(BootResponse, self._get("/boot", {"agent": normalized_agent, "budget": budget}))

    def export(self, fmt: str = "json") -> ExportResponse:
        return cast(ExportResponse, self._get("/export", {"format": fmt}))

    def import_data(self, data: ImportPayload) -> ImportResponse:
        return cast(ImportResponse, self._post("/import", data))

    def forget(self, source: str) -> ForgetResponse:
        return cast(ForgetResponse, self._post("/forget", {"source": source}))

    def shutdown(self) -> ShutdownResponse:
        return cast(ShutdownResponse, self._post("/shutdown"))

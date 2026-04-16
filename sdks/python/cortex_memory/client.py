"""Core Cortex client using httpx for async/sync HTTP calls."""

# SPDX-License-Identifier: MIT

from __future__ import annotations

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

    def _headers(self) -> dict[str, str]:
        h = dict(_CORTEX_HEADERS)
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

    def peek(self, query: str, k: int = 10) -> PeekResponse:
        return cast(PeekResponse, self._get("/peek", {"q": query, "k": k}))

    def store(
        self,
        decision: str,
        context: Optional[str] = None,
        source_agent: str = "python-sdk",
        source_model: Optional[str] = None,
        confidence: Optional[float] = None,
        reasoning_depth: Optional[str] = None,
        ttl_seconds: Optional[int] = None,
        entry_type: Optional[str] = None,
    ) -> StoreResponse:
        body: dict[str, object] = {"decision": decision, "source_agent": source_agent}
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

    def diary(self, text: str, agent: str = "python-sdk") -> DiaryResponse:
        return cast(DiaryResponse, self._post("/diary", {"text": text, "agent": agent}))

    def boot(self, agent: str = "python-sdk", budget: int = 600) -> BootResponse:
        return cast(BootResponse, self._get("/boot", {"agent": agent, "budget": budget}))

    def export(self, fmt: str = "json") -> ExportResponse:
        return cast(ExportResponse, self._get("/export", {"format": fmt}))

    def import_data(self, data: ImportPayload) -> ImportResponse:
        return cast(ImportResponse, self._post("/import", data))

    def forget(self, source: str) -> ForgetResponse:
        return cast(ForgetResponse, self._post("/forget", {"source": source}))

    def shutdown(self) -> ShutdownResponse:
        return cast(ShutdownResponse, self._post("/shutdown"))

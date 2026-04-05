"""Core Cortex client using httpx for async/sync HTTP calls."""

# SPDX-License-Identifier: AGPL-3.0-only
# This file is part of Cortex.
#
# Cortex is free software: you can redistribute it and/or modify
# it under the terms of the GNU Affero General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
# GNU Affero General Public License for more details.
#
# You should have received a copy of the GNU Affero General Public License
# along with this program. If not, see <https://www.gnu.org/licenses/>.

from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Optional

import httpx

_DEFAULT_BASE = "http://127.0.0.1:7437"
_CORTEX_HEADERS = {"X-Cortex-Request": "true"}


def _read_token() -> Optional[str]:
    home = Path(os.environ.get("USERPROFILE", os.environ.get("HOME", ".")))
    token_path = home / ".cortex" / "cortex.token"
    try:
        return token_path.read_text().strip() or None
    except (FileNotFoundError, PermissionError):
        return None


class CortexClient:
    """Synchronous + async Python client for the Cortex daemon.

    Usage::

        from cortex_memory import CortexClient

        client = CortexClient()
        health = client.health()
        results = client.recall("What is Cortex?", budget=200)
        client.store("New fact", source="my-script")
    """

    def __init__(
        self,
        base_url: str = _DEFAULT_BASE,
        token: Optional[str] = None,
        timeout: float = 10.0,
    ):
        self.base_url = base_url.rstrip("/")
        self.token = token or _read_token()
        self.timeout = timeout

    def _headers(self) -> dict[str, str]:
        h = dict(_CORTEX_HEADERS)
        if self.token:
            h["Authorization"] = f"Bearer {self.token}"
        return h

    def _get(self, path: str, params: Optional[dict] = None) -> Any:
        with httpx.Client(timeout=self.timeout) as c:
            resp = c.get(
                f"{self.base_url}{path}",
                headers=self._headers(),
                params=params,
            )
            resp.raise_for_status()
            return resp.json()

    def _post(self, path: str, json: Optional[dict] = None) -> Any:
        with httpx.Client(timeout=self.timeout) as c:
            resp = c.post(
                f"{self.base_url}{path}",
                headers=self._headers(),
                json=json or {},
            )
            resp.raise_for_status()
            return resp.json()

    # ── Public API ──────────────────────────────────────────────────

    def health(self) -> dict:
        """Check daemon health (no auth required)."""
        with httpx.Client(timeout=self.timeout) as c:
            resp = c.get(f"{self.base_url}/health")
            resp.raise_for_status()
            return resp.json()

    def recall(
        self,
        query: str,
        budget: int = 200,
        k: int = 10,
        agent: Optional[str] = None,
    ) -> dict:
        params: dict[str, Any] = {"q": query, "budget": budget, "k": k}
        if agent:
            params["agent"] = agent
        return self._get("/recall", params)

    def peek(self, query: str, k: int = 10) -> dict:
        return self._get("/peek", {"q": query, "k": k})

    def store(
        self,
        text: str,
        source: Optional[str] = None,
        source_agent: str = "python-sdk",
        **kwargs: Any,
    ) -> dict:
        body: dict[str, Any] = {"text": text, "source_agent": source_agent}
        if source:
            body["source"] = source
        body.update(kwargs)
        return self._post("/store", body)

    def diary(self, text: str, agent: str = "python-sdk") -> dict:
        return self._post("/diary", {"text": text, "agent": agent})

    def boot(self, agent: str = "python-sdk", budget: int = 600) -> dict:
        return self._get("/boot", {"agent": agent, "budget": budget})

    def export(self, fmt: str = "json") -> Any:
        return self._get("/export", {"format": fmt})

    def import_data(self, data: dict) -> dict:
        return self._post("/import", data)

    def forget(self, source: str) -> dict:
        return self._post("/forget", {"source": source})

    def shutdown(self) -> dict:
        return self._post("/shutdown")

"""Shared dataclasses for Cortex HTTP benchmark adapters."""
from __future__ import annotations

from dataclasses import dataclass


@dataclass
class CortexStoredDocument:
    id: str
    content: str
    user_id: str | None = None
    timestamp: str | None = None
    context: str | None = None

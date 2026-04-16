from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from cortex_http_client import CortexHTTPClient


@dataclass
class FunctionCallResult:
    name: str
    payload: dict[str, Any]


class CortexFunctionAdapter:
    """OpenAI function-call adapter over Cortex HTTP endpoints."""

    def __init__(self, client: CortexHTTPClient | None = None) -> None:
        self.client = client or CortexHTTPClient()

    def close(self) -> None:
        self.client.close()

    def openai_tools(self) -> list[dict[str, Any]]:
        return [
            {
                "type": "function",
                "function": {
                    "name": "cortex_health",
                    "description": "Check Cortex daemon health and readiness.",
                    "parameters": {"type": "object", "properties": {}},
                },
            },
            {
                "type": "function",
                "function": {
                    "name": "cortex_store_memory",
                    "description": "Store a memory/decision in Cortex.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "decision": {"type": "string"},
                            "context": {"type": "string"},
                            "entry_type": {"type": "string"},
                            "confidence": {"type": "number"},
                            "source_agent": {"type": "string"},
                        },
                        "required": ["decision"],
                    },
                },
            },
            {
                "type": "function",
                "function": {
                    "name": "cortex_recall_memory",
                    "description": "Recall top matching memories from Cortex.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": {"type": "string"},
                            "k": {"type": "integer"},
                            "budget": {"type": "integer"},
                            "agent": {"type": "string"},
                        },
                        "required": ["query"],
                    },
                },
            },
        ]

    def execute_function_call(self, function_name: str, arguments_json: str) -> FunctionCallResult:
        args = self._parse_arguments(arguments_json)
        if function_name == "cortex_health":
            return FunctionCallResult(
                name=function_name,
                payload=self.client.request("GET", "/health", auth_required=False),
            )
        if function_name == "cortex_store_memory":
            decision = str(args.get("decision", "")).strip()
            if not decision:
                raise ValueError("cortex_store_memory requires a non-empty `decision` argument.")
            payload = {
                "decision": decision,
                "context": str(args.get("context", "")).strip(),
                "type": str(args.get("entry_type", "note")).strip() or "note",
                "source_agent": str(
                    args.get("source_agent", "openai-function-adapter")
                ).strip(),
            }
            if "confidence" in args:
                payload["confidence"] = float(args["confidence"])
            return FunctionCallResult(
                name=function_name,
                payload=self.client.request("POST", "/store", json=payload),
            )
        if function_name == "cortex_recall_memory":
            query = str(args.get("query", "")).strip()
            if not query:
                raise ValueError("cortex_recall_memory requires a non-empty `query` argument.")
            params: dict[str, str] = {"q": query}
            if "k" in args:
                params["k"] = str(max(1, int(args["k"])))
            if "budget" in args:
                params["budget"] = str(max(1, int(args["budget"])))
            if "agent" in args:
                agent = str(args.get("agent", "")).strip()
                if agent:
                    params["agent"] = agent
            return FunctionCallResult(
                name=function_name,
                payload=self.client.request("GET", "/recall", params=params),
            )
        raise ValueError(f"Unsupported function call: {function_name}")

    @staticmethod
    def _parse_arguments(arguments_json: str) -> dict[str, Any]:
        if not arguments_json.strip():
            return {}
        try:
            value = json.loads(arguments_json)
        except json.JSONDecodeError as exc:
            raise ValueError(f"Invalid JSON function arguments: {exc}") from exc
        if not isinstance(value, dict):
            raise ValueError("Function arguments must decode to a JSON object.")
        return value

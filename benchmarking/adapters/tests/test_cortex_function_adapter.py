from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

ADAPTERS_DIR = Path(__file__).resolve().parents[1]
if str(ADAPTERS_DIR) not in sys.path:
    sys.path.insert(0, str(ADAPTERS_DIR))

from cortex_function_adapter import CortexFunctionAdapter  # noqa: E402


class _FakeClient:
    def __init__(self, queued_payloads: list[dict[str, object]]) -> None:
        self.calls: list[dict[str, object]] = []
        self._queued_payloads = queued_payloads

    def request(
        self,
        method: str,
        path: str,
        *,
        auth_required: bool = True,
        **kwargs: object,
    ) -> dict[str, object]:
        self.calls.append(
            {
                "method": method,
                "path": path,
                "auth_required": auth_required,
                "kwargs": kwargs,
            }
        )
        if self._queued_payloads:
            return self._queued_payloads.pop(0)
        return {"ok": True}

    def close(self) -> None:
        return None


def test_openai_tools_expose_health_store_recall_functions() -> None:
    adapter = CortexFunctionAdapter(client=_FakeClient([]))
    names = [item["function"]["name"] for item in adapter.openai_tools()]
    assert names == ["cortex_health", "cortex_store_memory", "cortex_recall_memory"]


def test_execute_health_omits_auth_header_path() -> None:
    client = _FakeClient([{"status": "ok", "ready": True}])
    adapter = CortexFunctionAdapter(client=client)

    result = adapter.execute_function_call("cortex_health", "{}")

    assert result.payload["ready"] is True
    assert client.calls[0]["method"] == "GET"
    assert client.calls[0]["path"] == "/health"
    assert client.calls[0]["auth_required"] is False


def test_execute_store_memory_serializes_payload_defaults() -> None:
    client = _FakeClient([{"ok": True, "id": "mem-1"}])
    adapter = CortexFunctionAdapter(client=client)

    result = adapter.execute_function_call(
        "cortex_store_memory",
        json.dumps({"decision": "Ship sqlite-vec canary", "context": "phase-2a"}),
    )

    assert result.payload["ok"] is True
    call = client.calls[0]
    assert call["method"] == "POST"
    assert call["path"] == "/store"
    payload = call["kwargs"]["json"]
    assert payload["decision"] == "Ship sqlite-vec canary"
    assert payload["context"] == "phase-2a"
    assert payload["type"] == "note"
    assert payload["source_agent"] == "openai-function-adapter"


def test_execute_recall_memory_maps_query_params() -> None:
    client = _FakeClient([{"results": [{"source": "m1", "excerpt": "x"}]}])
    adapter = CortexFunctionAdapter(client=client)

    result = adapter.execute_function_call(
        "cortex_recall_memory",
        json.dumps({"query": "daemon lock", "k": 7, "budget": 320, "agent": "chatgpt"}),
    )

    assert result.payload["results"][0]["source"] == "m1"
    call = client.calls[0]
    assert call["method"] == "GET"
    assert call["path"] == "/recall"
    params = call["kwargs"]["params"]
    assert params["q"] == "daemon lock"
    assert params["k"] == "7"
    assert params["budget"] == "320"
    assert params["agent"] == "chatgpt"


def test_execute_function_call_rejects_unknown_name() -> None:
    adapter = CortexFunctionAdapter(client=_FakeClient([]))
    with pytest.raises(ValueError, match="Unsupported function call"):
        adapter.execute_function_call("cortex_unknown", "{}")


def test_execute_function_call_rejects_invalid_json_arguments() -> None:
    adapter = CortexFunctionAdapter(client=_FakeClient([]))
    with pytest.raises(ValueError, match="Invalid JSON function arguments"):
        adapter.execute_function_call("cortex_health", "{bad-json")

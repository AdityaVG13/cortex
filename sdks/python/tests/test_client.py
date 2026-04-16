# SPDX-License-Identifier: MIT

import json

from cortex_memory.client import CortexClient


def test_recall_sends_auth_and_ssrf_headers(httpx_mock):
    httpx_mock.add_response(json={"items": []})
    client = CortexClient(base_url="http://127.0.0.1:7437", token="ctx_test_token")

    client.recall("deploy gate", budget=222, k=7, agent="codex")

    requests = httpx_mock.get_requests()
    assert len(requests) == 1
    req = requests[0]
    assert req.url.path == "/recall"
    assert req.url.params["q"] == "deploy gate"
    assert req.url.params["budget"] == "222"
    assert req.url.params["k"] == "7"
    assert req.url.params["agent"] == "codex"
    assert req.headers["X-Cortex-Request"] == "true"
    assert req.headers["Authorization"] == "Bearer ctx_test_token"


def test_store_serializes_optional_fields(httpx_mock):
    httpx_mock.add_response(json={"ok": True})
    client = CortexClient(base_url="http://127.0.0.1:7437", token="ctx_store_token")

    client.store(
        "Prefer vector fallback",
        context="Canary trials",
        source_agent="py-suite",
        source_model="gpt-5.4",
        confidence=0.93,
        reasoning_depth="high",
        ttl_seconds=3600,
        entry_type="decision",
    )

    requests = httpx_mock.get_requests()
    assert len(requests) == 1
    req = requests[0]
    payload = json.loads(req.read().decode("utf-8"))
    assert payload["decision"] == "Prefer vector fallback"
    assert payload["context"] == "Canary trials"
    assert payload["type"] == "decision"
    assert payload["source_agent"] == "py-suite"
    assert payload["source_model"] == "gpt-5.4"
    assert payload["confidence"] == 0.93
    assert payload["reasoning_depth"] == "high"
    assert payload["ttl_seconds"] == 3600


def test_health_uses_health_endpoint_without_auth_header(httpx_mock):
    httpx_mock.add_response(json={"ok": True})
    client = CortexClient(base_url="http://127.0.0.1:7437", token="ctx_health_token")

    client.health()

    requests = httpx_mock.get_requests()
    assert len(requests) == 1
    req = requests[0]
    assert req.url.path == "/health"
    assert "Authorization" not in req.headers
    assert "X-Cortex-Request" not in req.headers

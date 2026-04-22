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
    assert req.headers["X-Source-Agent"] == "python-sdk"
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
    assert req.headers["X-Source-Agent"] == "python-sdk"
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


def test_remote_base_url_requires_explicit_token():
    try:
        CortexClient(base_url="https://team.example.com")
    except ValueError as exc:
        assert "requires explicit token" in str(exc).lower()
    else:
        raise AssertionError("Expected remote base_url without token to fail")


def test_format_recall_context_is_content_first_with_optional_metrics():
    client = CortexClient(base_url="http://127.0.0.1:7437", token="ctx_format_token")
    payload = {
        "results": [
            {
                "source": "memory::1",
                "method": "keyword",
                "excerpt": "Business Administration",
                "relevance": 0.91,
            },
            {
                "source": "memory::2",
                "method": "semantic",
                "excerpt": "Valentine's Day volunteer event",
                "relevance": 0.83,
            },
        ],
        "budget": 300,
        "spent": 214,
        "saved": 86,
        "mode": "balanced",
    }
    context = client.format_recall_context(payload, include_metrics=True, max_items=1)
    assert "Business Administration" in context
    assert "Valentine's Day" not in context
    assert "[retrieval-metrics]" in context
    assert '"budget": 300' in context


def test_recall_for_prompt_uses_recall_response(httpx_mock):
    httpx_mock.add_response(
        json={
            "results": [
                {
                    "source": "memory::1",
                    "method": "keyword",
                    "excerpt": "Prompt-ready excerpt",
                    "relevance": 0.8,
                }
            ],
            "budget": 200,
            "spent": 100,
            "saved": 100,
        }
    )
    client = CortexClient(base_url="http://127.0.0.1:7437", token="ctx_prompt_token")
    context = client.recall_for_prompt("what happened", include_metrics=False)
    assert "Prompt-ready excerpt" in context
    assert "[retrieval-metrics]" not in context

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

import pytest

ADAPTERS_DIR = Path(__file__).resolve().parents[1]
if str(ADAPTERS_DIR) not in sys.path:
    sys.path.insert(0, str(ADAPTERS_DIR))

from cortex_http_client import CortexHTTPClient, CortexStoredDocument  # noqa: E402


class _FakeResponse:
    def __init__(self, payload: dict[str, object] | None = None, status_code: int = 200) -> None:
        self._payload = payload
        self.status_code = status_code
        self.content = b"" if payload is None else json.dumps(payload, ensure_ascii=True).encode("utf-8")

    def raise_for_status(self) -> None:
        if self.status_code >= 400:
            raise RuntimeError(f"HTTP {self.status_code}")

    def json(self) -> dict[str, object]:
        if self._payload is None:
            raise RuntimeError("json() called with empty payload")
        return self._payload


class _FakeHTTPXClient:
    def __init__(self, responses: list[_FakeResponse]) -> None:
        self._responses = responses
        self.calls: list[dict[str, object]] = []
        self.closed = False

    def request(self, method: str, url: str, headers: dict[str, str], **kwargs: object) -> _FakeResponse:
        self.calls.append(
            {
                "method": method,
                "url": url,
                "headers": headers,
                "kwargs": kwargs,
            }
        )
        if not self._responses:
            raise RuntimeError(f"No fake response queued for {method} {url}")
        return self._responses.pop(0)

    def close(self) -> None:
        self.closed = True


@pytest.fixture
def configured_env(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "ctx_test_token")
    monkeypatch.setenv("CORTEX_BASE_URL", "http://127.0.0.1:9001")
    monkeypatch.setenv("CORTEX_TIMEOUT_SECONDS", "5")
    monkeypatch.setenv("CORTEX_BENCHMARK_NAMESPACE", "Test Suite Namespace")
    metrics_file = tmp_path / "metrics" / "retrieval-metrics.jsonl"
    monkeypatch.setenv("CORTEX_BENCHMARK_METRICS_FILE", str(metrics_file))
    return metrics_file


def test_healthcheck_omits_auth_header(configured_env: Path) -> None:
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient([_FakeResponse({"status": "ok", "ready": True})])
    client.client = fake

    payload = client.healthcheck()

    assert payload["status"] == "ok"
    assert len(fake.calls) == 1
    call = fake.calls[0]
    assert call["method"] == "GET"
    assert call["url"] == "http://127.0.0.1:9001/health"
    headers = call["headers"]
    assert headers["X-Cortex-Request"] == "true"
    assert headers["X-Source-Agent"] == "amb-cortex"
    assert "Authorization" not in headers


def test_store_documents_serializes_metadata_and_context_key(configured_env: Path) -> None:
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient([_FakeResponse({})])
    client.client = fake
    document = CortexStoredDocument(
        id="doc-1",
        content="daemon lock prevents duplicate startup",
        user_id="user-7",
        timestamp="2026-04-16T08:30:00Z",
        context="startup-failure",
    )

    client.store_documents([document])

    assert len(fake.calls) == 1
    call = fake.calls[0]
    assert call["url"] == "http://127.0.0.1:9001/store"
    assert call["headers"]["Authorization"] == "Bearer ctx_test_token"
    body = call["kwargs"]["json"]
    assert body["context"] == "amb::test-suite-namespace::user::user-7::doc::doc-1"
    assert "[timestamp] 2026-04-16T08:30:00Z" in body["decision"]
    assert "[user] user-7" in body["decision"]
    assert "[context] startup-failure" in body["decision"]
    assert "daemon lock prevents duplicate startup" in body["decision"]


def test_recall_documents_filters_user_dedupes_and_respects_k(configured_env: Path) -> None:
    client = CortexHTTPClient()
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::d1"] = CortexStoredDocument(
        id="d1",
        content="Primary user memory",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-2::doc::d2"] = CortexStoredDocument(
        id="d2",
        content="Other user memory",
        user_id="user-2",
    )

    payload = {
        "results": [
            {"source": "amb::test-suite-namespace::user::user-1::doc::d1", "excerpt": "Primary user memory"},
            {"source": "amb::test-suite-namespace::user::user-1::doc::d1", "excerpt": "Duplicate result"},
            {"source": "amb::test-suite-namespace::user::user-2::doc::d2", "excerpt": "Other user memory"},
            {"source": "recall::unknown", "excerpt": "Recovered from excerpt only"},
        ],
        "budget": 300,
        "spent": 210,
        "saved": 90,
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, raw_payload = client.recall_documents("daemon startup", k=2, user_id="user-1")

    assert raw_payload["spent"] == 210
    assert len(docs) == 2
    assert docs[0].id == "d1"
    assert docs[0].user_id == "user-1"
    assert docs[1].id == "recall::unknown"
    assert docs[1].content == "Recovered from excerpt only"
    assert docs[1].user_id == "user-1"

    call = fake.calls[0]
    params = call["kwargs"]["params"]
    assert params["q"] == "daemon startup"
    # user-scoped queries fan out recall depth for better hit coverage.
    assert params["k"] == "50"
    assert params["budget"] == "300"
    assert params["source_prefix"] == "amb::test-suite-namespace::"


def test_recall_documents_writes_metrics_jsonl(configured_env: Path) -> None:
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient(
        [
            _FakeResponse(
                {
                    "results": [
                        {"source": "memory::a", "excerpt": "A", "tokens": 11},
                        {"source": "memory::b", "excerpt": "B", "tokens": 13},
                    ],
                    "budget": 300,
                    "spent": 100,
                    "saved": 200,
                }
            )
        ]
    )
    client.client = fake

    docs, _payload = client.recall_documents("lock state", k=2)

    assert len(docs) == 2
    metrics_file = configured_env
    assert metrics_file.exists()
    lines = metrics_file.read_text(encoding="utf-8").strip().splitlines()
    assert len(lines) == 1
    record = json.loads(lines[0])
    assert record["query"] == "lock state"
    assert record["budget"] == 300
    assert record["result_count"] == 2
    assert record["token_estimate"] == 24


def test_reset_namespace_clears_context_map(configured_env: Path) -> None:
    client = CortexHTTPClient()
    client.docs_by_context["x"] = CortexStoredDocument(id="x", content="stale")
    client.reset_namespace("release candidate")

    assert client.namespace == "release-candidate"
    assert client.docs_by_context == {}

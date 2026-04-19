from __future__ import annotations

import json
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
BENCHMARKING_DIR = REPO_ROOT / "benchmarking"
if str(BENCHMARKING_DIR) not in sys.path:
    sys.path.insert(0, str(BENCHMARKING_DIR))
AMB_SRC = BENCHMARKING_DIR / "tools" / "agent-memory-benchmark" / "src"
if str(AMB_SRC) not in sys.path:
    sys.path.insert(0, str(AMB_SRC))
ADAPTERS_DIR = BENCHMARKING_DIR / "adapters"
if str(ADAPTERS_DIR) not in sys.path:
    sys.path.insert(0, str(ADAPTERS_DIR))

from cortex_http_base_provider import CortexHTTPBaseMemoryProvider  # noqa: E402
from memory_bench.models import Document  # noqa: E402


class _FakeResponse:
    def __init__(self, payload: dict[str, object], status_code: int = 200) -> None:
        self._payload = payload
        self.status_code = status_code
        self.content = b"{}"

    def raise_for_status(self) -> None:
        if self.status_code >= 400:
            raise RuntimeError(f"http error: {self.status_code}")

    def json(self) -> dict[str, object]:
        return self._payload


class _FakeHttpClient:
    def __init__(self) -> None:
        self.calls: list[dict[str, object]] = []
        self.recall_payload: dict[str, object] = {
            "results": [],
            "budget": 300,
            "spent": 0,
            "saved": 300,
        }

    def request(
        self,
        method: str,
        url: str,
        *,
        headers: dict[str, str] | None = None,
        **kwargs: object,
    ) -> _FakeResponse:
        self.calls.append(
            {
                "method": method,
                "url": url,
                "headers": headers or {},
                "kwargs": kwargs,
            }
        )
        if method == "GET" and url.endswith("/health"):
            return _FakeResponse({"status": "ok", "ready": True})
        if method == "GET" and url.endswith("/recall"):
            return _FakeResponse(self.recall_payload)
        return _FakeResponse({})

    def close(self) -> None:
        return None


def _set_base_env(monkeypatch) -> None:
    monkeypatch.setenv("CORTEX_BASE_URL", "http://127.0.0.1:7437")
    monkeypatch.setenv("CORTEX_AUTH_TOKEN", "test-token")
    monkeypatch.setenv("CORTEX_BENCHMARK_NAMESPACE", "bench-ns")
    monkeypatch.setenv("CORTEX_RECALL_BUDGET", "300")


def test_base_provider_ingest_uses_direct_store_calls(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="d1",
                content="I studied in Melbourne, Australia.",
                user_id="u1",
            )
        ]
    )
    provider.cleanup()

    store_calls = [call for call in fake_client.calls if call["url"].endswith("/store")]
    assert len(store_calls) == 1
    body = store_calls[0]["kwargs"]["json"]
    assert isinstance(body, dict)
    assert body["context"] == "amb::bench-ns::user::u1::doc::d1"
    assert "Melbourne" in str(body["decision"])


def test_base_provider_ingest_skips_unchanged_documents(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)

    provider = CortexHTTPBaseMemoryProvider()
    doc = Document(
        id="d1",
        content="I studied in Melbourne, Australia.",
        user_id="u1",
    )
    provider.ingest([doc])
    provider.ingest([doc])
    provider.cleanup()

    store_calls = [call for call in fake_client.calls if call["url"].endswith("/store")]
    assert len(store_calls) == 1


def test_base_provider_retrieve_runs_single_recall_call(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "2")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::user::u1::doc::d1",
                "excerpt": "University of Melbourne in Australia",
                "relevance": 0.88,
                "method": "bm25",
                "tokens": 12,
            }
        ],
        "budget": 300,
        "spent": 12,
        "saved": 288,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="d1",
                content="I completed my degree at the University of Melbourne in Australia.",
                user_id="u1",
            )
        ]
    )
    docs, payload = provider.retrieve("Where did I study abroad?", k=1, user_id="u1")
    provider.cleanup()

    recall_calls = [call for call in fake_client.calls if call["url"].endswith("/recall")]
    assert len(recall_calls) == 1
    params = recall_calls[0]["kwargs"]["params"]
    assert isinstance(params, dict)
    assert params["source_prefix"] == "amb::bench-ns::user::u1::"
    assert len(docs) == 1
    assert docs[0].id == "d1"
    assert "University of Melbourne" in docs[0].content
    assert payload["spent"] == 12


def test_base_provider_writes_recall_metrics(monkeypatch, tmp_path: Path) -> None:
    fake_client = _FakeHttpClient()
    metrics_file = tmp_path / "retrieval-metrics.jsonl"
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_METRICS_FILE", str(metrics_file))
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::doc::d1",
                "excerpt": "sample",
                "relevance": 0.7,
                "method": "bm25",
                "tokens": 20,
            }
        ],
        "budget": 300,
        "spent": 20,
        "saved": 280,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.retrieve("sample query", k=1, user_id=None)
    provider.cleanup()

    rows = [line for line in metrics_file.read_text(encoding="utf-8").splitlines() if line.strip()]
    assert len(rows) == 1
    payload = json.loads(rows[0])
    assert payload["query"] == "sample query"
    assert payload["token_estimate"] == 20
    assert payload["recall_call_count"] == 1


def test_base_provider_detail_rerank_prefers_fact_variant(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "2")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::user::u1::doc::d1",
                "excerpt": "I graduated from university.",
                "relevance": 0.91,
                "tokens": 10,
            },
            {
                "source": "amb::bench-ns::user::u1::doc::d1::fact::1",
                "excerpt": "I graduated in 2018 from the University of Melbourne.",
                "relevance": 0.73,
                "tokens": 9,
            },
        ],
        "budget": 300,
        "spent": 19,
        "saved": 281,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="d1",
                content="I graduated from university.",
                user_id="u1",
            ),
            Document(
                id="d1::fact::1",
                content="[user] I graduated in 2018 from the University of Melbourne.",
                user_id="u1",
            ),
        ]
    )
    docs, _payload = provider.retrieve("What year did I graduate?", k=2, user_id="u1")
    provider.cleanup()

    assert len(docs) == 2
    assert docs[0].id == "d1::fact::1"
    recall_calls = [call for call in fake_client.calls if call["url"].endswith("/recall")]
    assert len(recall_calls) == 1


def test_base_provider_detail_query_expands_fact_family_siblings(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "2")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_DETAIL_SIBLINGS_PER_SEED", "2")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_DETAIL_MAX_ADDED_SIBLINGS", "4")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::user::u1::doc::trip::fact::1",
                "excerpt": "I bought it at a market.",
                "relevance": 0.9,
                "tokens": 8,
            }
        ],
        "budget": 300,
        "spent": 8,
        "saved": 292,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="trip::fact::1",
                content="[user] I bought it at a market.",
                user_id="u1",
            ),
            Document(
                id="trip::fact::2",
                content="[user] I bought it at a market in Melbourne, Australia.",
                user_id="u1",
            ),
        ]
    )
    docs, _payload = provider.retrieve("Which country was the market in?", k=2, user_id="u1")
    provider.cleanup()

    assert len(docs) == 2
    assert docs[0].id == "trip::fact::2"
    assert {doc.id for doc in docs} == {"trip::fact::1", "trip::fact::2"}
    recall_calls = [call for call in fake_client.calls if call["url"].endswith("/recall")]
    assert len(recall_calls) == 1


def test_base_provider_profile_query_rerank_prefers_occupation_fact(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "2")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::user::u1::doc::career",
                "excerpt": "I used Trello before.",
                "relevance": 0.95,
                "tokens": 7,
            },
            {
                "source": "amb::bench-ns::user::u1::doc::career::fact::1",
                "excerpt": "I worked as a marketing specialist at a small startup.",
                "relevance": 0.72,
                "tokens": 12,
            },
        ],
        "budget": 300,
        "spent": 19,
        "saved": 281,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="career",
                content="[user] I used Trello before.",
                user_id="u1",
            ),
            Document(
                id="career::fact::1",
                content="[user] I worked as a marketing specialist at a small startup.",
                user_id="u1",
            ),
        ]
    )
    docs, _payload = provider.retrieve("What was my previous occupation?", k=2, user_id="u1")
    provider.cleanup()

    assert len(docs) == 2
    assert docs[0].id == "career::fact::1"


def test_base_provider_query_terms_expand_profile_and_education_signals(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)

    provider = CortexHTTPBaseMemoryProvider()
    terms = provider._query_terms("What was my previous occupation after I graduated?")
    assert "occupation" in terms
    assert "job" in terms
    assert "role" in terms
    assert "graduated" in terms
    assert "graduate" in terms

    overlap = provider._term_overlap_count(
        terms,
        "I worked as a marketing specialist and later completed my degree.",
    )
    assert overlap >= 2

    profile = provider._build_query_profile("What play did I attend at the local theater?")
    assert bool(profile["wants_event"]) is True
    assert bool(profile["is_detail_query"]) is True
    non_profile = provider._build_query_profile("What was my previous stance on spirituality?")
    assert bool(non_profile["wants_profile"]) is False
    assert bool(non_profile["wants_previous_role"]) is False
    assert bool(non_profile["wants_belief"]) is True
    assert bool(non_profile["is_detail_query"]) is True

    provider.cleanup()


def test_base_provider_detail_query_uses_variant_recall_query(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "1")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::user::u1::doc::career::fact::1",
                "excerpt": "I worked as a marketing specialist at a startup.",
                "relevance": 0.8,
                "tokens": 10,
            }
        ],
        "budget": 300,
        "spent": 10,
        "saved": 290,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="career::fact::1",
                content="[user] I worked as a marketing specialist at a startup.",
                user_id="u1",
            )
        ]
    )
    provider.retrieve("What was my previous occupation?", k=1, user_id="u1")
    provider.cleanup()

    recall_calls = [call for call in fake_client.calls if call["url"].endswith("/recall")]
    assert len(recall_calls) == 1
    params = recall_calls[0]["kwargs"]["params"]
    assert isinstance(params, dict)
    assert params["q"] != "What was my previous occupation?"
    assert "occupation" in str(params["q"])
    assert "worked as" in str(params["q"])


def test_base_provider_belief_detail_query_uses_variant_recall_query(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "1")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::user::u1::doc::belief::fact::1",
                "excerpt": "I used to be a staunch atheist.",
                "relevance": 0.82,
                "tokens": 8,
            }
        ],
        "budget": 420,
        "spent": 8,
        "saved": 412,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="belief::fact::1",
                content="[user] I used to be a staunch atheist.",
                user_id="u1",
            )
        ]
    )
    provider.retrieve("What was my previous stance on spirituality?", k=1, user_id="u1")
    provider.cleanup()

    recall_calls = [call for call in fake_client.calls if call["url"].endswith("/recall")]
    assert len(recall_calls) == 1
    params = recall_calls[0]["kwargs"]["params"]
    assert isinstance(params, dict)
    assert params["q"] != "What was my previous stance on spirituality?"
    assert "atheist" in str(params["q"])
    assert "spirituality" in str(params["q"])
    assert params["budget"] == "420"


def test_base_provider_abroad_query_adds_country_qualifier(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "2")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::user::u1::doc::trip::fact::1",
                "excerpt": "I attended my study abroad program at the University of Melbourne.",
                "relevance": 0.92,
                "tokens": 14,
            },
            {
                "source": "amb::bench-ns::user::u1::doc::trip::fact::2",
                "excerpt": "It was in Australia.",
                "relevance": 0.7,
                "tokens": 6,
            },
        ],
        "budget": 300,
        "spent": 20,
        "saved": 280,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="trip::fact::1",
                content="[user] I attended my study abroad program at the University of Melbourne.",
                user_id="u1",
            ),
            Document(
                id="trip::fact::2",
                content="[user] I attended it in Australia.",
                user_id="u1",
            ),
        ]
    )
    docs, _payload = provider.retrieve(
        "Where did I attend for my study abroad program?",
        k=2,
        user_id="u1",
    )
    provider.cleanup()

    assert len(docs) == 2
    assert "[location-qualifier] in Australia." in docs[0].content


def test_base_provider_location_only_query_keeps_primary_query(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "1")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)

    provider = CortexHTTPBaseMemoryProvider()
    provider.retrieve("Where did I attend for my study abroad program?", k=1, user_id="u1")
    provider.cleanup()

    recall_calls = [call for call in fake_client.calls if call["url"].endswith("/recall")]
    assert len(recall_calls) == 1
    params = recall_calls[0]["kwargs"]["params"]
    assert isinstance(params, dict)
    assert params["q"] == "Where did I attend for my study abroad program?"


def test_base_provider_item_location_query_promotes_same_family_store_complement(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "3")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::user::u1::doc::coupon::fact::1",
                "excerpt": "I redeemed a $5 coupon on coffee creamer.",
                "relevance": 0.93,
                "tokens": 9,
            },
            {
                "source": "amb::bench-ns::user::u1::doc::other::fact::1",
                "excerpt": "I redeemed a coupon on groceries at the local store.",
                "relevance": 0.9,
                "tokens": 10,
            },
            {
                "source": "amb::bench-ns::user::u1::doc::coupon::fact::2",
                "excerpt": "I shop at Target pretty frequently.",
                "relevance": 0.65,
                "tokens": 7,
            },
        ],
        "budget": 300,
        "spent": 26,
        "saved": 274,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="coupon::fact::1",
                content="[user] I redeemed a $5 coupon on coffee creamer.",
                user_id="u1",
            ),
            Document(
                id="other::fact::1",
                content="[user] I redeemed a coupon on groceries at the local store.",
                user_id="u1",
            ),
            Document(
                id="coupon::fact::2",
                content="[user] I shop at Target pretty frequently.",
                user_id="u1",
            ),
        ]
    )
    docs, _payload = provider.retrieve(
        "Where did I redeem a $5 coupon on coffee creamer?",
        k=2,
        user_id="u1",
    )
    provider.cleanup()

    assert len(docs) == 2
    assert docs[0].id == "coupon::fact::1"
    assert docs[1].id == "coupon::fact::2"
    assert "[location-qualifier] at Target." in docs[0].content


def test_base_provider_previous_occupation_rerank_penalizes_market_activity(monkeypatch) -> None:
    fake_client = _FakeHttpClient()
    _set_base_env(monkeypatch)
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MULTIPLIER", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_BASE_FANOUT_MIN", "2")
    monkeypatch.setattr("cortex_http_base_provider.httpx.Client", lambda timeout: fake_client)
    fake_client.recall_payload = {
        "results": [
            {
                "source": "amb::bench-ns::user::u1::doc::market::fact::1",
                "excerpt": "I had good sales at the weekly farmers' market.",
                "relevance": 0.96,
                "tokens": 9,
            },
            {
                "source": "amb::bench-ns::user::u1::doc::career::fact::1",
                "excerpt": "I've used Trello in my previous role as a marketing specialist at a small startup.",
                "relevance": 0.73,
                "tokens": 14,
            },
        ],
        "budget": 300,
        "spent": 23,
        "saved": 277,
    }

    provider = CortexHTTPBaseMemoryProvider()
    provider.ingest(
        [
            Document(
                id="market::fact::1",
                content="[user] I had good sales at the weekly farmers' market.",
                user_id="u1",
            ),
            Document(
                id="career::fact::1",
                content="[user] I've used Trello in my previous role as a marketing specialist at a small startup.",
                user_id="u1",
            ),
        ]
    )
    docs, _payload = provider.retrieve("What was my previous occupation?", k=2, user_id="u1")
    provider.cleanup()

    assert len(docs) == 2
    assert docs[0].id == "career::fact::1"

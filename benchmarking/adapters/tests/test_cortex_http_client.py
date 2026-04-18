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
    def __init__(
        self,
        payload: dict[str, object] | None = None,
        status_code: int = 200,
        headers: dict[str, str] | None = None,
    ) -> None:
        self._payload = payload
        self.status_code = status_code
        self.headers = headers or {}
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
            {"source": "amb::test-suite-namespace::user::user-1::doc::d1", "excerpt": "Focused excerpt"},
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
    assert docs[0].content == "Focused excerpt"
    assert docs[1].id == "recall::unknown"
    assert docs[1].content == "Recovered from excerpt only"
    assert docs[1].user_id == "user-1"

    call = fake.calls[0]
    params = call["kwargs"]["params"]
    assert params["q"] == "daemon startup"
    # user-scoped queries fan out recall depth for better hit coverage.
    assert params["k"] == "80"
    assert params["budget"] == "300"
    assert params["source_prefix"] == "amb::test-suite-namespace::user::user-1::"


def test_recall_documents_detail_queries_use_higher_user_fanout(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {"source": "recall::location", "excerpt": "[user] Target."},
        ],
        "budget": 300,
        "spent": 120,
        "saved": 180,
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I redeem the coupon?", k=1, user_id="user-1")

    assert len(docs) == 1
    params = fake.calls[0]["kwargs"]["params"]
    assert params["k"] == "120"


def test_recall_documents_prefers_fact_extract_variant_within_same_family(configured_env: Path) -> None:
    client = CortexHTTPClient()
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::d1"] = CortexStoredDocument(
        id="d1",
        content="Long, noisy base memory content",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::d1::fact::1"] = CortexStoredDocument(
        id="d1::fact::1",
        content="[user] I upgraded to 500 Mbps last week.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": "amb::test-suite-namespace::user::user-1::doc::d1",
                "excerpt": "base excerpt",
            },
            {
                "source": "amb::test-suite-namespace::user::user-1::doc::d1::fact::1",
                "excerpt": "[user] I upgraded to 500 Mbps last week.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("internet speed", k=1, user_id="user-1")

    assert len(docs) == 1
    assert docs[0].id == "d1::fact::1"
    assert "500 Mbps" in docs[0].content


def test_recall_documents_selects_best_fact_variant_by_query_signal(configured_env: Path) -> None:
    client = CortexHTTPClient()
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::d2::fact::1"] = CortexStoredDocument(
        id="d2::fact::1",
        content="[user] I bought a Nintendo Switch in 2024.",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::d2::fact::2"] = CortexStoredDocument(
        id="d2::fact::2",
        content="[user] My commute to work takes 45 minutes each way.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": "amb::test-suite-namespace::user::user-1::doc::d2::fact::1",
                "excerpt": "[user] I bought a Nintendo Switch in 2024.",
            },
            {
                "source": "amb::test-suite-namespace::user::user-1::doc::d2::fact::2",
                "excerpt": "[user] My commute to work takes 45 minutes each way.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("How long is my commute to work?", k=1, user_id="user-1")

    assert len(docs) == 1
    assert docs[0].id == "d2::fact::2"
    assert "45 minutes each way" in docs[0].content


def test_recall_documents_detail_queries_expand_fact_family_candidates(configured_env: Path) -> None:
    client = CortexHTTPClient()
    family_source = "amb::test-suite-namespace::user::user-1::doc::d9::fact::1"
    client.docs_by_context[family_source] = CortexStoredDocument(
        id="d9::fact::1",
        content=(
            "[assistant-question] What did I buy for my sister's birthday gift? "
            "[user-answer] I bought gifts for my sister's birthday."
        ),
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::d9::fact::2"] = CortexStoredDocument(
        id="d9::fact::2",
        content=(
            "[assistant-question] What did I buy for my sister's birthday gift? "
            "[user-answer] A yellow dress."
        ),
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": family_source,
                "excerpt": "[user] I bought gifts for my sister's birthday.",
            }
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("What did I buy for my sister's birthday gift?", k=2, user_id="user-1")

    assert len(docs) == 2
    assert {doc.id for doc in docs} == {"d9::fact::1", "d9::fact::2"}
    assert any("yellow dress" in doc.content.lower() for doc in docs)


def test_recall_documents_detail_queries_expand_fact_family_from_base_seed(configured_env: Path) -> None:
    client = CortexHTTPClient()
    base_source = "amb::test-suite-namespace::user::user-1::doc::d10"
    client.docs_by_context[base_source] = CortexStoredDocument(
        id="d10",
        content="[user] I recently changed my internet plan and it feels better.",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::d10::fact::1"] = CortexStoredDocument(
        id="d10::fact::1",
        content="[user] I upgraded my internet plan.",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::d10::fact::2"] = CortexStoredDocument(
        id="d10::fact::2",
        content="[user-answer] 500 Mbps.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": base_source,
                "excerpt": "[user] I recently changed my internet plan.",
            }
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("What speed is my new internet plan?", k=3, user_id="user-1")

    assert len(docs) == 3
    assert any(doc.id == "d10::fact::2" for doc in docs)
    assert any("500 Mbps" in doc.content for doc in docs)


def test_recall_documents_reranks_by_query_overlap(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "recall::a",
                "excerpt": "Daily podcast recommendations and music trends.",
            },
            {
                "source": "recall::b",
                "excerpt": "My daily commute takes 45 minutes each way to work.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("How long is my daily commute to work?", k=1)

    assert len(docs) == 1
    assert "45 minutes each way" in docs[0].content


def test_recall_documents_extracts_query_window_with_numeric_fact(configured_env: Path) -> None:
    client = CortexHTTPClient()
    long_content = (
        "I often listen to audiobooks during my daily commute into the city. "
        + ("filler " * 120)
        + "Recently I tracked that my daily commute to work takes 45 minutes each way by train. "
        + ("tail " * 80)
    )
    source = "amb::test-suite-namespace::user::user-1::doc::d-commute"
    client.docs_by_context[source] = CortexStoredDocument(
        id="d-commute",
        content=long_content,
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": source,
                "excerpt": "I often listen to audiobooks during my daily commute.",
            }
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("How long is my daily commute to work?", k=1, user_id="user-1")

    assert len(docs) == 1
    assert "45 minutes each way" in docs[0].content


def test_recall_documents_deprioritizes_assistant_advice_noise(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "recall::assistant-advice",
                "excerpt": "[assistant] Here are tips for internet performance. You should restart your router weekly.",
            },
            {
                "source": "recall::user-fact",
                "excerpt": "[user] I upgraded my home internet to 500 Mbps last week.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("What internet speed did I upgrade to?", k=1)

    assert len(docs) == 1
    assert docs[0].id == "recall::user-fact"
    assert "500 Mbps" in docs[0].content


def test_recall_documents_uses_full_context_for_date_location_item_details(configured_env: Path) -> None:
    client = CortexHTTPClient()
    source = "amb::test-suite-namespace::user::user-1::doc::d-switch"
    client.docs_by_context[source] = CortexStoredDocument(
        id="d-switch",
        content="I bought a Nintendo Switch in Seattle on March 5, 2024 from a local store.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": source,
                "excerpt": "I bought a console recently.",
            }
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("When did I buy my Nintendo Switch in Seattle?", k=1, user_id="user-1")

    assert len(docs) == 1
    assert "Nintendo Switch" in docs[0].content
    assert "Seattle" in docs[0].content
    assert "2024" in docs[0].content


def test_recall_documents_prefers_user_location_fact_over_assistant_generalities(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "recall::assistant-summary",
                "excerpt": "[assistant] Here are recommendations for moving in 2021. You should plan your commute early.",
            },
            {
                "source": "recall::user-location",
                "excerpt": "[user] I moved to Denver in 2021.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I move in 2021?", k=1)

    assert len(docs) == 1
    assert docs[0].id == "recall::user-location"
    assert "Denver" in docs[0].content


def test_recall_documents_prefers_specific_location_over_generic_home(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "recall::location-generic",
                "excerpt": "[user] I take yoga classes at home.",
            },
            {
                "source": "recall::location-specific",
                "excerpt": "[user] I take yoga classes at Serenity Yoga.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where do I take yoga classes?", k=1)

    assert len(docs) == 1
    assert docs[0].id == "recall::location-specific"
    assert "Serenity Yoga" in docs[0].content


def test_recall_documents_handles_short_location_abbreviations(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "recall::location-generic",
                "excerpt": "[user] I moved from home to work recently.",
            },
            {
                "source": "recall::location-abbrev",
                "excerpt": "[user] I moved to LA last summer.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I move?", k=1)

    assert len(docs) == 1
    assert docs[0].id == "recall::location-abbrev"
    assert "LA" in docs[0].content


def test_text_has_location_detail_accepts_short_standalone_place(configured_env: Path) -> None:
    client = CortexHTTPClient()

    assert client._text_has_location_detail("Target.")
    assert client._text_has_location_detail("Australia")
    assert not client._text_has_location_detail("thanks")


def test_text_has_speed_detail_accepts_megabits_per_second(configured_env: Path) -> None:
    client = CortexHTTPClient()

    assert client._text_has_speed_detail("I upgraded my internet plan to 500 megabits per second.")


def test_text_has_item_detail_accepts_short_user_answer_phrase(configured_env: Path) -> None:
    client = CortexHTTPClient()

    assert client._text_has_item_detail("[user-answer] A yellow dress.")
    assert not client._text_has_item_detail("[user-answer] Not sure.")


def test_recall_documents_penalizes_answer_sources_for_detail_queries(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "amb::test-suite-namespace::user::user-42::doc::user-42_answer_abcd1234::fact::1",
                "excerpt": "[user] I noticed my internet feels better recently.",
            },
            {
                "source": "amb::test-suite-namespace::user::user-42::doc::user-42_turn_1::fact::1",
                "excerpt": "[user] I upgraded my internet plan to 500 Mbps.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("What speed is my new internet plan?", k=1, user_id="user-42")

    assert len(docs) == 1
    assert docs[0].id.endswith("user-42_turn_1::fact::1")
    assert "500 Mbps" in docs[0].content


def test_source_quality_adjustment_only_penalizes_schema_answer_ids(configured_env: Path) -> None:
    client = CortexHTTPClient()
    penalized = "amb::test-suite-namespace::user::user-42::doc::user-42_answer_abcd1234::fact::1"
    non_penalized = "amb::test-suite-namespace::user::user-42::doc::my-answer-notes::fact::1"

    assert client._source_quality_adjustment(penalized) == (2 - client.answer_source_penalty)
    assert client._source_quality_adjustment(non_penalized) == 2


def test_recall_documents_uses_whole_word_overlap_not_substrings(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "recall::noise",
                "excerpt": "[user] My daily workout takes 20 minutes before breakfast.",
            },
            {
                "source": "recall::commute",
                "excerpt": "[user] My daily commute takes 45 minutes each way.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("How long is my daily commute to work?", k=1)

    assert len(docs) == 1
    assert docs[0].id == "recall::commute"
    assert "45 minutes each way" in docs[0].content


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


def test_recall_documents_detail_query_variants_use_split_budget_and_aggregate_tokens(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO", "0.4")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET", "96")
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient(
        [
            _FakeResponse(
                {
                    "results": [
                        {"source": "memory::primary", "excerpt": "[user] internet feels faster", "tokens": 40},
                    ],
                    "budget": 180,
                    "spent": 180,
                    "saved": 0,
                }
            ),
            _FakeResponse(
                {
                    "results": [
                        {"source": "memory::variant", "excerpt": "[user] I upgraded to 500 Mbps.", "tokens": 60},
                    ],
                    "budget": 120,
                    "spent": 120,
                    "saved": 0,
                }
            ),
        ]
    )
    client.client = fake

    docs, payload = client.recall_documents("What speed is my new internet plan?", k=2, user_id="user-1")

    assert len(fake.calls) == 2
    primary_params = fake.calls[0]["kwargs"]["params"]
    variant_params = fake.calls[1]["kwargs"]["params"]
    assert primary_params["budget"] == "180"
    assert variant_params["budget"] == "120"
    assert primary_params["q"] == "What speed is my new internet plan?"
    assert variant_params["q"] != primary_params["q"]
    assert payload["budget"] == 300
    assert payload["spent"] == 300
    assert len(docs) == 2
    assert {doc.id for doc in docs} == {"memory::primary", "memory::variant"}

    record = json.loads(configured_env.read_text(encoding="utf-8").strip().splitlines()[-1])
    assert record["query"] == "What speed is my new internet plan?"
    assert record["recall_call_count"] == 2
    assert record["token_estimate"] == 100
    assert record["combined_token_estimate"] == 100
    assert record["recall_variant_queries"]


def test_recall_documents_detail_query_variants_trigger_for_occupation_queries(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO", "0.35")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET", "96")
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient(
        [
            _FakeResponse(
                {
                    "results": [
                        {"source": "memory::generic", "excerpt": "[user] I changed jobs recently.", "tokens": 30},
                    ],
                    "budget": 204,
                    "spent": 204,
                    "saved": 0,
                }
            ),
            _FakeResponse(
                {
                    "results": [
                        {
                            "source": "memory::occupation",
                            "excerpt": "[user] I worked as a marketing specialist at a small startup.",
                            "tokens": 45,
                        },
                    ],
                    "budget": 96,
                    "spent": 96,
                    "saved": 0,
                }
            ),
        ]
    )
    client.client = fake

    docs, _payload = client.recall_documents("What was my previous occupation?", k=2, user_id="user-1")

    assert len(fake.calls) == 2
    variant_params = fake.calls[1]["kwargs"]["params"]
    assert variant_params["q"] != "What was my previous occupation?"
    assert "occupation" in variant_params["q"]
    assert any("marketing specialist" in doc.content.lower() for doc in docs)


def test_build_query_profile_avoids_birthday_date_bias_for_item_queries(configured_env: Path) -> None:
    client = CortexHTTPClient()
    profile = client._build_query_profile("What did I buy for my sister's birthday gift?")

    assert profile["wants_item"] is True
    assert profile["wants_date"] is False


def test_build_query_context_extracts_user_answer_detail_candidates(configured_env: Path) -> None:
    client = CortexHTTPClient()
    context = client._build_query_context_text(
        query="What speed is my new internet plan?",
        full_content=(
            "[user] [assistant-question] What speed is your new internet plan after the upgrade? "
            "[user-answer] I upgraded to 500 Mbps."
        ),
        excerpt="[user] [assistant-question] What speed is your new internet plan?",
    )

    assert "500 Mbps" in context


def test_build_query_context_prefers_full_content_for_exact_date_detail(configured_env: Path) -> None:
    client = CortexHTTPClient()
    context = client._build_query_context_text(
        query="When did I volunteer at the fundraising dinner?",
        full_content=(
            "[user] I volunteered at the \"Love is in the Air\" fundraising dinner on "
            "February 14th and stayed for the full event."
        ),
        excerpt="[user] I volunteered at the fundraising dinner back in February.",
    )

    assert "February 14th" in context


def test_build_query_context_prefers_full_content_for_location_qualifier(configured_env: Path) -> None:
    client = CortexHTTPClient()
    context = client._build_query_context_text(
        query="Where did I attend for my study abroad program?",
        full_content=(
            "[user] For my study abroad program, I attended the University of Melbourne in Australia."
        ),
        excerpt="[user] For my study abroad program, I attended the University of Melbourne.",
    )

    assert "Australia" in context


def test_reset_namespace_clears_context_map(configured_env: Path) -> None:
    client = CortexHTTPClient()
    client.docs_by_context["x"] = CortexStoredDocument(id="x", content="stale")
    client.reset_namespace("release candidate")

    assert client.namespace == "release-candidate"
    assert client.docs_by_context == {}


def test_request_retries_transient_429(configured_env: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient(
        [
            _FakeResponse({"error": "rate limited"}, status_code=429, headers={"Retry-After": "0"}),
            _FakeResponse({"status": "ok"}),
        ]
    )
    client.client = fake
    sleeps: list[float] = []
    monkeypatch.setattr("cortex_http_client.time.sleep", lambda seconds: sleeps.append(seconds))

    payload = client.request("GET", "/health", auth_required=False)

    assert payload["status"] == "ok"
    assert len(fake.calls) == 2
    assert sleeps


def test_store_documents_normalizes_nullable_content(configured_env: Path) -> None:
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient([_FakeResponse({})])
    client.client = fake
    document = CortexStoredDocument(
        id="doc-null",
        content=None,  # type: ignore[arg-type]
        user_id="user-null",
        timestamp=None,
        context=None,
    )

    client.store_documents([document])

    assert len(fake.calls) == 1
    body = fake.calls[0]["kwargs"]["json"]
    assert body["context"] == "amb::test-suite-namespace::user::user-null::doc::doc-null"
    assert body["decision"] == "[user] user-null"


def test_recall_documents_clips_long_content(configured_env: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_CONTEXT_MAX_CHARS", "16")
    client = CortexHTTPClient()
    client.docs_by_context["amb::test-suite-namespace::doc::long"] = CortexStoredDocument(
        id="long",
        content="this is a very long memory body that should be clipped",
        user_id=None,
    )
    payload = {
        "results": [
            {
                "source": "amb::test-suite-namespace::doc::long",
                "excerpt": "this is an even longer excerpt payload that should be clipped",
            }
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("clip", k=1, user_id=None)

    assert len(docs) == 1
    assert docs[0].content == "this ... lipped"

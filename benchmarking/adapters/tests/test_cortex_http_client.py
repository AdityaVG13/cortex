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


def test_store_documents_skips_unchanged_payloads(configured_env: Path) -> None:
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient([_FakeResponse({})])
    client.client = fake
    document = CortexStoredDocument(
        id="doc-1",
        content="stable payload",
        user_id="user-7",
        timestamp="2026-04-16T08:30:00Z",
        context="startup-failure",
    )

    client.store_documents([document])
    client.store_documents([document])

    assert len(fake.calls) == 1


def test_store_documents_dedupes_identical_payloads_across_contexts(configured_env: Path) -> None:
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient([_FakeResponse({})])
    client.client = fake
    first = CortexStoredDocument(
        id="doc-1",
        content="identical payload",
        user_id="user-1",
        timestamp="2026-04-16T08:30:00Z",
        context="context-a",
    )
    second = CortexStoredDocument(
        id="doc-2",
        content="identical payload",
        user_id="user-1",
        timestamp="2026-04-16T08:30:00Z",
        context="context-a",
    )

    client.store_documents([first, second])

    assert len(fake.calls) == 1


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


def test_recall_documents_location_queries_add_richer_family_sibling(configured_env: Path) -> None:
    client = CortexHTTPClient()
    seed_source = "amb::test-suite-namespace::user::user-1::doc::dloc::fact::1"
    client.docs_by_context[seed_source] = CortexStoredDocument(
        id="dloc::fact::1",
        content="[user] I redeemed a $5 coupon on coffee creamer last Sunday.",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::dloc::fact::2"] = CortexStoredDocument(
        id="dloc::fact::2",
        content="[user] I've been using the Cartwheel app from Target for savings.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": seed_source,
                "excerpt": "[user] I redeemed a $5 coupon on coffee creamer last Sunday.",
            }
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I redeem a $5 coupon on coffee creamer?", k=2, user_id="user-1")

    assert len(docs) == 2
    assert any(doc.id == "dloc::fact::2" for doc in docs)
    assert any("target" in doc.content.lower() for doc in docs)


def test_recall_documents_location_queries_keep_specific_sibling_when_seed_location_is_generic(
    configured_env: Path,
) -> None:
    client = CortexHTTPClient()
    seed_source = "amb::test-suite-namespace::user::user-1::doc::dloc2::fact::1"
    client.docs_by_context[seed_source] = CortexStoredDocument(
        id="dloc2::fact::1",
        content=(
            "[user] I actually redeemed a $5 coupon on coffee creamer last Sunday, "
            "which was a nice surprise since I didn't know I had it in my email inbox."
        ),
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::dloc2::fact::2"] = CortexStoredDocument(
        id="dloc2::fact::2",
        content="[user] I shop at Target pretty frequently, maybe every other week.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": seed_source,
                "excerpt": (
                    "[user] I actually redeemed a $5 coupon on coffee creamer last Sunday, "
                    "which was a nice surprise since I didn't know I had it in my email inbox."
                ),
            }
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I redeem a $5 coupon on coffee creamer?", k=2, user_id="user-1")

    assert len(docs) == 2
    assert any(doc.id == "dloc2::fact::2" for doc in docs)
    assert any("target" in doc.content.lower() for doc in docs)


def test_recall_documents_location_queries_keep_target_when_non_place_phrase_is_present(
    configured_env: Path,
) -> None:
    client = CortexHTTPClient()
    prefix = "amb::test-suite-namespace::user::user-1::doc::"

    def add_document(doc_id: str, content: str) -> str:
        source = f"{prefix}{doc_id}"
        client.docs_by_context[source] = CortexStoredDocument(
            id=doc_id,
            content=content,
            user_id="user-1",
        )
        return source

    # Primary coupon/location family where Target must stay in top-k.
    seed_source = add_document(
        "dloc3::fact::1",
        "[user] I actually redeemed a $5 coupon on coffee creamer last Sunday, "
        "which was a nice surprise since I didn't know I had it in my email inbox.",
    )
    add_document(
        "dloc3::fact::2",
        "[user] I shop at Target pretty frequently, maybe every other week.",
    )
    add_document(
        "dloc3::fact::3",
        "[user] I've been using the Cartwheel app from Target and it's been really helpful.",
    )
    add_document(
        "dloc3::fact::4",
        "[user] I think the Cartwheel app is really user-friendly and easy to navigate.",
    )
    add_document(
        "dloc3::fact::10",
        "[user] One thing I wish they would add is a way to sort offers by expiration date.",
    )

    # Competing families to mimic realistic high-noise top-k pressure.
    movie_seed = add_document("dmovie::fact::1", "[user] I've been watching a lot of movies lately.")
    add_document("dmovie::fact::2", "[user] I'm rewatching Marvel movies in chronological order.")
    add_document("dmovie::fact::3", "[user] I'm trying to figure out what to watch next.")
    add_document("dmovie::fact::5", "[user] I'm planning to watch more movies before Endgame.")
    add_document("dmovie::fact::6", "[user] I've got a long way to go before Endgame.")

    story_seed = add_document("dstory::fact::1", "[user] I know I typed a lot and would love suggestions.")
    add_document("dstory::fact::4", "[user] I'm hopeful the maps can stay ambiguous until players piece clues together.")
    add_document("dstory::fact::9", "[user] Just to make sure, I'm talking about the Jugernaut and Hydra paths.")

    payload = {
        "results": [
            {
                "source": seed_source,
                "excerpt": (
                    "[user] I actually redeemed a $5 coupon on coffee creamer last Sunday, "
                    "which was a nice surprise since I didn't know I had it in my email inbox."
                ),
            },
            {"source": movie_seed, "excerpt": "[user] I've been watching a lot of movies lately."},
            {"source": story_seed, "excerpt": "[user] I know I typed a lot and would love suggestions."},
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I redeem a $5 coupon on coffee creamer?", k=10, user_id="user-1")

    returned_ids = {doc.id for doc in docs}
    assert "dloc3::fact::2" in returned_ids
    assert any("target" in doc.content.lower() for doc in docs)
    assert client._location_term_set(
        "[user] I think the Cartwheel app is really user-friendly and easy to navigate."
    ) == set()


def test_recall_documents_location_queries_promote_same_family_country_qualifier_into_top_k(
    configured_env: Path,
) -> None:
    client = CortexHTTPClient()
    seed_source = "amb::test-suite-namespace::user::user-1::doc::dstudy::fact::1"
    client.docs_by_context[seed_source] = CortexStoredDocument(
        id="dstudy::fact::1",
        content="[user] I attended the University of Melbourne during my study abroad program.",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::dstudy::fact::2"] = CortexStoredDocument(
        id="dstudy::fact::2",
        content="[user] I've been to the Great Ocean Road before, and it's definitely a must-see in Australia.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": seed_source,
                "excerpt": "[user] I attended the University of Melbourne during my study abroad program.",
            },
            {
                "source": "recall::noise-1",
                "excerpt": "[user] Study abroad planning tips for university assignments and deadlines.",
            },
            {
                "source": "recall::noise-2",
                "excerpt": "[user] I stayed organized with class schedules during my study abroad semester.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I attend for my study abroad program?", k=2, user_id="user-1")

    assert len(docs) == 2
    assert any(doc.id == "dstudy::fact::2" for doc in docs)
    assert any("australia" in doc.content.lower() for doc in docs)


def test_recall_documents_location_queries_promote_country_qualifier_when_all_docs_fit_within_k(
    configured_env: Path,
) -> None:
    client = CortexHTTPClient()
    seed_source = "amb::test-suite-namespace::user::user-1::doc::dstudy::fact::1"
    client.docs_by_context[seed_source] = CortexStoredDocument(
        id="dstudy::fact::1",
        content="[user] I attended the University of Melbourne during my study abroad program.",
        user_id="user-1",
    )
    qualifier_source = "amb::test-suite-namespace::user::user-1::doc::dstudy::fact::2"
    client.docs_by_context[qualifier_source] = CortexStoredDocument(
        id="dstudy::fact::2",
        content="[user] I've been to the Great Ocean Road before, and it's definitely a must-see in Australia.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": seed_source,
                "excerpt": "[user] I attended the University of Melbourne during my study abroad program.",
            },
            {"source": "recall::noise-1", "excerpt": "[user] I stayed organized with class schedules."},
            {"source": qualifier_source, "excerpt": "[user] ...must-see in Australia."},
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I attend for my study abroad program?", k=10, user_id="user-1")

    assert len(docs) == 3
    assert docs[0].id == "dstudy::fact::1"
    assert docs[1].id == "dstudy::fact::2"
    assert "australia" in docs[1].content.lower()


def test_recall_documents_location_queries_promote_abroad_country_qualifier_across_families(
    configured_env: Path,
) -> None:
    client = CortexHTTPClient()
    seed_source = "amb::test-suite-namespace::user::user-1::doc::dstudyx::fact::1"
    qualifier_source = "amb::test-suite-namespace::user::user-1::doc::dtripx::fact::2"
    client.docs_by_context[seed_source] = CortexStoredDocument(
        id="dstudyx::fact::1",
        content="[user] I attended the University of Melbourne during my study abroad program.",
        user_id="user-1",
    )
    client.docs_by_context[qualifier_source] = CortexStoredDocument(
        id="dtripx::fact::2",
        content="[user] I've been to the Great Ocean Road before, and it's definitely a must-see in Australia.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": seed_source,
                "excerpt": "[user] I attended the University of Melbourne during my study abroad program.",
            },
            {
                "source": "recall::noise-study",
                "excerpt": "[user] I stayed organized with class schedules during my study abroad semester.",
            },
            {
                "source": qualifier_source,
                "excerpt": "[user] ...must-see in Australia.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I attend for my study abroad program?", k=2, user_id="user-1")

    assert len(docs) == 2
    assert docs[0].id == "dstudyx::fact::1"
    assert docs[1].id == "dtripx::fact::2"
    assert "australia" in docs[1].content.lower()


def test_recall_documents_location_queries_promote_abroad_country_qualifier_when_within_k_window(
    configured_env: Path,
) -> None:
    client = CortexHTTPClient()
    seed_source = "amb::test-suite-namespace::user::user-1::doc::dstudyw::fact::1"
    qualifier_source = "amb::test-suite-namespace::user::user-1::doc::dtripw::fact::2"
    client.docs_by_context[seed_source] = CortexStoredDocument(
        id="dstudyw::fact::1",
        content="[user] I attended the University of Melbourne during my study abroad program.",
        user_id="user-1",
    )
    client.docs_by_context[qualifier_source] = CortexStoredDocument(
        id="dtripw::fact::2",
        content="[user] I've been to the Great Ocean Road before, and it's definitely a must-see in Australia.",
        user_id="user-1",
    )

    results: list[dict[str, str]] = [
        {
            "source": seed_source,
            "excerpt": "[user] I attended the University of Melbourne during my study abroad program.",
        }
    ]
    for idx in range(1, 9):
        source = f"amb::test-suite-namespace::user::user-1::doc::dnoisew::{idx}"
        client.docs_by_context[source] = CortexStoredDocument(
            id=f"dnoisew::{idx}",
            content=f"[user] Noise memory {idx} about meal prep and scheduling.",
            user_id="user-1",
        )
        results.append({"source": source, "excerpt": f"[user] Noise memory {idx} about meal prep."})
    results.append({"source": qualifier_source, "excerpt": "[user] ...must-see in Australia."})
    for idx in range(9, 24):
        source = f"amb::test-suite-namespace::user::user-1::doc::dnoisew::{idx}"
        client.docs_by_context[source] = CortexStoredDocument(
            id=f"dnoisew::{idx}",
            content=f"[user] Additional filler memory {idx}.",
            user_id="user-1",
        )
        results.append({"source": source, "excerpt": f"[user] Additional filler memory {idx}."})

    payload = {"results": results}
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I attend for my study abroad program?", k=20, user_id="user-1")

    assert len(docs) >= 2
    assert docs[0].id == "dstudyw::fact::1"
    assert docs[1].id == "dtripw::fact::2"
    assert "australia" in docs[1].content.lower()


def test_recall_documents_location_queries_append_abroad_country_qualifier_to_primary_context(
    configured_env: Path,
) -> None:
    client = CortexHTTPClient()
    seed_source = "amb::test-suite-namespace::user::user-1::doc::dstudyq::fact::1"
    qualifier_source = "amb::test-suite-namespace::user::user-1::doc::dtripq::fact::2"
    kyoto_source = "amb::test-suite-namespace::user::user-1::doc::dtripq::fact::3"
    client.docs_by_context[seed_source] = CortexStoredDocument(
        id="dstudyq::fact::1",
        content="[user] I attended the University of Melbourne during my study abroad program.",
        user_id="user-1",
    )
    client.docs_by_context[qualifier_source] = CortexStoredDocument(
        id="dtripq::fact::2",
        content="[user] I've been to the Great Ocean Road before, and it's definitely a must-see in Australia.",
        user_id="user-1",
    )
    client.docs_by_context[kyoto_source] = CortexStoredDocument(
        id="dtripq::fact::3",
        content="[user] What are some good places to try kaiseki, the multi-course meal I fell in love with in Kyoto?",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": seed_source,
                "excerpt": "[user] I attended the University of Melbourne during my study abroad program.",
            },
            {
                "source": kyoto_source,
                "excerpt": "[user] What are some good places to try kaiseki in Kyoto?",
            },
            {
                "source": qualifier_source,
                "excerpt": "[user] ...must-see in Australia.",
            },
        ]
    }
    client.client = _FakeHTTPXClient([_FakeResponse(payload)])

    docs, _ = client.recall_documents("Where did I attend for my study abroad program?", k=2, user_id="user-1")

    assert len(docs) == 2
    assert docs[0].id == "dstudyq::fact::1"
    assert "[location-qualifier] in Australia." in docs[0].content
    assert "Kyoto" not in docs[0].content


def test_recall_documents_location_item_queries_skip_weak_seed_expansion_and_keep_store_inference(
    configured_env: Path,
) -> None:
    client = CortexHTTPClient()
    coupon_seed = "amb::test-suite-namespace::user::user-1::doc::dloc4::fact::1"
    noise_seed = "amb::test-suite-namespace::user::user-1::doc::dnoise4::fact::1"
    client.docs_by_context[coupon_seed] = CortexStoredDocument(
        id="dloc4::fact::1",
        content="[user] I actually redeemed a $5 coupon on coffee creamer last Sunday.",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::dloc4::fact::2"] = CortexStoredDocument(
        id="dloc4::fact::2",
        content="[user] I shop at Target pretty frequently, maybe every other week.",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::dloc4::fact::3"] = CortexStoredDocument(
        id="dloc4::fact::3",
        content="[user] I've been using the Cartwheel app from Target and it's been really helpful for savings.",
        user_id="user-1",
    )
    client.docs_by_context[noise_seed] = CortexStoredDocument(
        id="dnoise4::fact::1",
        content="[user] I know I typed a lot and would love suggestions.",
        user_id="user-1",
    )
    client.docs_by_context["amb::test-suite-namespace::user::user-1::doc::dnoise4::fact::2"] = CortexStoredDocument(
        id="dnoise4::fact::2",
        content=(
            "[user] Just to make sure you remember, I'm talking about the Jugernaut, "
            "the Hydra, the Skullcrawler, the Beserker, the Lasher, the Hoplite, and the Fissionator."
        ),
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": coupon_seed,
                "excerpt": "[user] I actually redeemed a $5 coupon on coffee creamer last Sunday.",
            },
            {
                "source": noise_seed,
                "excerpt": "[user] I know I typed a lot and would love suggestions.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I redeem a $5 coupon on coffee creamer?", k=6, user_id="user-1")

    returned_ids = {doc.id for doc in docs}
    assert "dloc4::fact::2" in returned_ids
    assert "dloc4::fact::3" in returned_ids
    assert "dnoise4::fact::2" not in returned_ids
    assert any("target" in doc.content.lower() for doc in docs)


def test_location_item_affinity_bonus_boosts_store_location_purchase_context(configured_env: Path) -> None:
    client = CortexHTTPClient()
    query_profile = client._build_query_profile("Where did I redeem a $5 coupon on coffee creamer?")

    store_bonus = client._location_item_affinity_bonus(
        query_profile=query_profile,
        text="[user] I shop at Target pretty frequently, maybe every other week.",
    )
    generic_bonus = client._location_item_affinity_bonus(
        query_profile=query_profile,
        text="[user] I redeemed a coupon while at home last weekend.",
    )
    no_location_bonus = client._location_item_affinity_bonus(
        query_profile=query_profile,
        text="[user] I redeemed a coupon last weekend.",
    )

    assert store_bonus > generic_bonus
    assert generic_bonus > no_location_bonus


def test_recall_documents_high_detail_prefers_user_answer_sources_for_item_queries(
    configured_env: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "high-detail")
    monkeypatch.setenv("CORTEX_BENCHMARK_ANSWER_SOURCE_PENALTY", "26")
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "amb::test-suite-namespace::user::user-1::doc::d9::answer_fea2e4d3::fact::8",
                "excerpt": "[assistant-question] What did I buy for my sister's birthday gift? [user-answer] A yellow dress.",
            },
            {
                "source": "amb::test-suite-namespace::user::user-1::doc::d9::fact::3",
                "excerpt": "[user] I bought gifts for my sister's birthday.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("What did I buy for my sister's birthday gift?", k=1, user_id="user-1")

    assert len(docs) == 1
    assert docs[0].id.endswith("answer_fea2e4d3::fact::8")
    assert "yellow dress" in docs[0].content.lower()


def test_answer_source_detail_relief_requires_high_detail_policy(
    configured_env: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "standard")
    client = CortexHTTPClient()
    query_profile = client._build_query_profile("What did I buy for my sister's birthday gift?")
    doc = CortexStoredDocument(
        id="d9::answer_fea2e4d3::fact::8",
        content="[assistant-question] What did I buy for my sister's birthday gift? [user-answer] A yellow dress.",
        user_id="user-1",
    )

    relief = client._answer_source_detail_relief(
        query_profile=query_profile,
        document=doc,
        text=doc.content,
        overlap=3,
        detail_bonus=8,
    )

    assert relief == 0


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


def test_recall_documents_prefers_previous_occupation_over_current_role_for_previous_query(
    configured_env: Path,
) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "memory::current-role",
                "excerpt": "[user] I currently work as a product manager.",
            },
            {
                "source": "memory::previous-role",
                "excerpt": "[user] I previously worked as a marketing specialist.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("What was my previous occupation?", k=1, user_id="user-1")

    assert len(docs) == 1
    assert docs[0].id == "memory::previous-role"
    assert "marketing specialist" in docs[0].content.lower()


def test_recall_documents_prefers_concrete_user_answer_for_sister_gift_query(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "memory::gift-generic",
                "excerpt": (
                    "[assistant-question] What did I buy for my sister's birthday gift? "
                    "[user-answer] I bought gifts for my sister's birthday."
                ),
            },
            {
                "source": "memory::gift-concrete",
                "excerpt": (
                    "[assistant-question] What did I buy for my sister's birthday gift? "
                    "[user-answer] A yellow dress."
                ),
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("What did I buy for my sister's birthday gift?", k=1, user_id="user-1")

    assert len(docs) == 1
    assert docs[0].id == "memory::gift-concrete"
    assert "yellow dress" in docs[0].content.lower()


def test_recall_documents_runs_variant_when_primary_relations_conflict(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "standard")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO", "0.4")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET", "96")
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient(
        [
            _FakeResponse(
                {
                    "results": [
                        {
                            "source": "memory::gift-primary",
                            "excerpt": (
                                "[assistant-question] What did I buy for my sister's birthday gift? "
                                "[user-answer] I bought gifts for my sister's birthday."
                            ),
                            "tokens": 60,
                        },
                        {
                            "source": "memory::gift-conflict",
                            "excerpt": (
                                "[user] I bought a customized phone case for my brother's birthday last month."
                            ),
                            "tokens": 55,
                        },
                    ],
                    "budget": 180,
                    "spent": 180,
                    "saved": 0,
                }
            ),
            _FakeResponse(
                {
                    "results": [
                        {
                            "source": "memory::gift-concrete",
                            "excerpt": (
                                "[assistant-question] What did I buy for my sister's birthday gift? "
                                "[user-answer] A yellow dress."
                            ),
                            "tokens": 45,
                        },
                    ],
                    "budget": 120,
                    "spent": 120,
                    "saved": 0,
                }
            ),
        ]
    )
    client.client = fake

    docs, payload = client.recall_documents("What did I buy for my sister's birthday gift?", k=1, user_id="user-1")

    assert len(fake.calls) == 2
    variant_params = fake.calls[1]["kwargs"]["params"]
    assert variant_params["q"] != "What did I buy for my sister's birthday gift?"
    assert "sister" in variant_params["q"]
    assert len(docs) == 1
    assert docs[0].id == "memory::gift-concrete"
    assert "yellow dress" in docs[0].content.lower()
    assert payload["budget"] == 300


def test_recall_documents_skips_detail_variant_when_primary_already_has_required_detail(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "standard")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO", "0.4")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET", "96")
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient(
        [
            _FakeResponse(
                {
                    "results": [
                        {
                            "source": "memory::primary",
                            "excerpt": "[user] I upgraded to 500 Mbps about three weeks ago.",
                            "tokens": 40,
                        },
                    ],
                    "budget": 180,
                    "spent": 180,
                    "saved": 0,
                }
            )
        ]
    )
    client.client = fake

    docs, payload = client.recall_documents("What speed is my new internet plan?", k=1, user_id="user-1")

    assert len(fake.calls) == 1
    assert payload["budget"] == 180
    assert len(docs) == 1
    record = json.loads(configured_env.read_text(encoding="utf-8").strip().splitlines()[-1])
    assert record["recall_call_count"] == 1
    assert record["recall_variant_queries"] == []


def test_recall_documents_high_detail_policy_keeps_variant_for_detail_queries(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "high-detail")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO", "0.4")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET", "96")
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient(
        [
            _FakeResponse(
                {
                    "results": [
                        {
                            "source": "memory::primary",
                            "excerpt": "[user] I attended the University of Melbourne during study abroad.",
                            "tokens": 40,
                        },
                    ],
                    "budget": 180,
                    "spent": 180,
                    "saved": 0,
                }
            ),
            _FakeResponse(
                {
                    "results": [
                        {
                            "source": "memory::variant",
                            "excerpt": "[user] I attended the University of Melbourne in Australia.",
                            "tokens": 30,
                        }
                    ],
                    "budget": 120,
                    "spent": 120,
                    "saved": 0,
                }
            ),
        ]
    )
    client.client = fake

    docs, payload = client.recall_documents(
        "Where did I attend for my study abroad program?",
        k=1,
        user_id="user-1",
    )

    assert len(fake.calls) == 2
    assert len(docs) == 1
    assert docs[0].id in {"memory::primary", "memory::variant"}
    assert payload["budget"] == 300
    record = json.loads(configured_env.read_text(encoding="utf-8").strip().splitlines()[-1])
    assert record["recall_call_count"] == 2
    assert len(record["recall_variant_queries"]) == 1


def test_recall_documents_high_detail_location_prefers_user_answer_with_country_qualifier(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "high-detail")
    client = CortexHTTPClient()
    answer_source = "amb::test-suite-namespace::user::user-1::doc::memory_answer_ab12cd34::fact::1"
    fact_source = "amb::test-suite-namespace::user::user-1::doc::memory_fact::fact::1"
    client.docs_by_context[fact_source] = CortexStoredDocument(
        id="memory_fact::fact::1",
        content="[user] I attended the University of Melbourne during my study abroad program.",
        user_id="user-1",
    )
    client.docs_by_context[answer_source] = CortexStoredDocument(
        id="memory_answer_ab12cd34::fact::1",
        content=(
            "[assistant-question] Where did I attend for my study abroad program? "
            "[user-answer] University of Melbourne in Australia."
        ),
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": fact_source,
                "excerpt": "[user] I attended the University of Melbourne during my study abroad program.",
            },
            {
                "source": answer_source,
                "excerpt": "[user] I attended the University of Melbourne during my study abroad program.",
            },
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I attend for my study abroad program?", k=1, user_id="user-1")

    assert len(docs) == 1
    assert docs[0].id == "memory_answer_ab12cd34::fact::1"
    assert "australia" in docs[0].content.lower()


def test_recall_documents_high_detail_policy_skips_variant_when_primary_has_specific_location_detail(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_ENABLE_DETAIL_QUERY_VARIANTS", "1")
    monkeypatch.setenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "high-detail")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_BUDGET_RATIO", "0.4")
    monkeypatch.setenv("CORTEX_BENCHMARK_DETAIL_QUERY_MIN_BUDGET", "96")
    client = CortexHTTPClient()
    fake = _FakeHTTPXClient(
        [
            _FakeResponse(
                {
                    "results": [
                        {
                            "source": "memory::primary",
                            "excerpt": "[user] I attended the University of Melbourne in Australia during my study abroad program.",
                            "tokens": 40,
                        },
                    ],
                    "budget": 180,
                    "spent": 180,
                    "saved": 0,
                }
            )
        ]
    )
    client.client = fake

    docs, payload = client.recall_documents(
        "Where did I attend for my study abroad program?",
        k=1,
        user_id="user-1",
    )

    assert len(fake.calls) == 1
    assert len(docs) == 1
    assert payload["budget"] == 180
    record = json.loads(configured_env.read_text(encoding="utf-8").strip().splitlines()[-1])
    assert record["recall_call_count"] == 1
    assert record["recall_variant_queries"] == []


def test_recall_documents_drops_out_of_scope_sources_when_user_scoped(configured_env: Path) -> None:
    client = CortexHTTPClient()
    payload = {
        "results": [
            {
                "source": "amb::test-suite-namespace::user::user-1::doc::d1",
                "excerpt": "In-scope detail",
            },
            {
                "source": "self-improvement::session-log",
                "excerpt": "Out-of-scope noise",
            },
            {
                "source": "recall::unknown",
                "excerpt": "Recovered from excerpt only",
            },
        ],
        "budget": 300,
        "spent": 210,
        "saved": 90,
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _raw_payload = client.recall_documents("where did I redeem the coupon?", k=3, user_id="user-1")

    assert len(docs) == 2
    assert all(doc.id != "self-improvement::session-log" for doc in docs)
    assert any(doc.id == "recall::unknown" for doc in docs)


def test_build_query_profile_avoids_birthday_date_bias_for_item_queries(configured_env: Path) -> None:
    client = CortexHTTPClient()
    profile = client._build_query_profile("What did I buy for my sister's birthday gift?")

    assert profile["wants_item"] is True
    assert profile["wants_date"] is False


def test_query_terms_include_cjk_overlap_tokens(configured_env: Path) -> None:
    client = CortexHTTPClient()
    query_terms = client._query_terms("我最喜欢的城市是什么？")

    assert "城市" in query_terms
    overlap = client._term_overlap_count(query_terms, "我最喜欢的城市是东京。")
    assert overlap >= 1


def test_build_query_profile_detects_mcq_and_extracts_stem(configured_env: Path) -> None:
    client = CortexHTTPClient()
    profile = client._build_query_profile(
        "Where did I move last year?\nA. Boston\nB. Seattle\nC. Denver\nD. Austin"
    )

    assert profile["is_mcq_query"] is True
    assert str(profile["term_query"]).startswith("where did i move last year")


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


def test_build_query_context_prefers_location_user_answer_country_qualifier(configured_env: Path) -> None:
    client = CortexHTTPClient()
    context = client._build_query_context_text(
        query="Where did I attend for my study abroad program?",
        full_content=(
            "[user] I attended the University of Melbourne during my study abroad program. "
            "[assistant-question] Where did I attend for my study abroad program? "
            "[user-answer] University of Melbourne in Australia."
        ),
        excerpt="[user] I attended the University of Melbourne during my study abroad program.",
    )

    assert "[user-answer]" in context
    assert "Australia" in context


def test_reset_namespace_clears_context_map(configured_env: Path) -> None:
    client = CortexHTTPClient()
    client.docs_by_context["x"] = CortexStoredDocument(id="x", content="stale")
    client._serialized_by_context["x"] = "serialized"
    client._content_digest_by_context["x"] = "digest"
    client._stored_content_digests.add("digest")
    client.reset_namespace("release candidate")

    assert client.namespace == "release-candidate"
    assert client.docs_by_context == {}
    assert client._serialized_by_context == {}
    assert client._content_digest_by_context == {}
    assert client._stored_content_digests == set()


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


def test_recall_documents_mcq_queries_use_larger_context_window(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_CONTEXT_MAX_CHARS", "16")
    monkeypatch.setenv("CORTEX_BENCHMARK_MCQ_CONTEXT_MAX_CHARS", "48")
    client = CortexHTTPClient()
    source = "amb::test-suite-namespace::user::user-1::doc::mcq-detail"
    client.docs_by_context[source] = CortexStoredDocument(
        id="mcq-detail",
        content="I moved to Seattle in 2024 and changed apartments near downtown.",
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": source,
                "excerpt": "I moved to Seattle in 2024 and changed apartments near downtown.",
            }
        ]
    }
    client.client = _FakeHTTPXClient([_FakeResponse(payload)])

    docs, _ = client.recall_documents(
        "Where did I move in 2024?\nA. Boston\nB. Seattle\nC. Austin\nD. Denver",
        k=1,
        user_id="user-1",
    )

    assert len(docs) == 1
    assert len(docs[0].content) <= 48
    assert len(docs[0].content) > 16
    assert "Seattle" in docs[0].content


def test_retrieval_policy_defaults_to_standard(configured_env: Path) -> None:
    client = CortexHTTPClient()

    assert client.retrieval_policy == "standard"


def test_high_detail_retrieval_policy_preserves_precise_fact_snippet_near_budget_limit(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_CONTEXT_MAX_CHARS", "64")
    monkeypatch.setenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "high-detail")
    client = CortexHTTPClient()
    source = "amb::test-suite-namespace::user::user-1::doc::detail-fact"
    client.docs_by_context[source] = CortexStoredDocument(
        id="detail-fact",
        content=(
            "Background notes before the fact. "
            "I moved to Seattle on March 5, 2024 (ID ZX-91Q). "
            "Additional unrelated tail notes after the fact."
        ),
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": source,
                "excerpt": "I moved recently.",
            }
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents(
        "When did I move to Seattle and what was the ID?",
        k=1,
        user_id="user-1",
    )

    assert len(docs) == 1
    assert len(docs[0].content) <= 64
    assert "Seattle" in docs[0].content
    assert "March 5, 2024" in docs[0].content
    assert "ZX-91Q" in docs[0].content


def test_high_detail_retrieval_policy_preserves_study_abroad_country_qualifier_under_budget(
    configured_env: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("CORTEX_BENCHMARK_CONTEXT_MAX_CHARS", "72")
    monkeypatch.setenv("CORTEX_BENCHMARK_RETRIEVAL_POLICY", "high-detail")
    client = CortexHTTPClient()
    source = "amb::test-suite-namespace::user::user-1::doc::study-abroad"
    client.docs_by_context[source] = CortexStoredDocument(
        id="study-abroad",
        content=(
            "Background detail. "
            "For my study abroad program, I attended the University of Melbourne in Australia. "
            "More unrelated trailing text."
        ),
        user_id="user-1",
    )
    payload = {
        "results": [
            {
                "source": source,
                "excerpt": "For my study abroad program, I attended the University of Melbourne.",
            }
        ]
    }
    fake = _FakeHTTPXClient([_FakeResponse(payload)])
    client.client = fake

    docs, _ = client.recall_documents("Where did I attend for my study abroad program?", k=1, user_id="user-1")

    assert len(docs) == 1
    assert len(docs[0].content) <= 72
    assert "Melbourne" in docs[0].content
    assert "Australia" in docs[0].content

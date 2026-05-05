# SPDX-License-Identifier: MIT

import json
from pathlib import Path

from cortex_memory.client import CortexClient


SPEC_PATH = Path(__file__).resolve().parents[3] / "specs" / "cortex-adapter-contract.yaml"


def load_contract():
    return json.loads(SPEC_PATH.read_text(encoding="utf-8"))


def scenario(contract, scenario_id):
    for item in contract["scenarios"]:
        if item["id"] == scenario_id:
            return item
    raise AssertionError(f"missing scenario {scenario_id}")


def test_contract_spec_has_required_scenarios():
    contract = load_contract()
    assert contract["schema"] == "cortex.adapter.contract"
    assert contract["version"] == "0.6.0"
    ids = {item["id"] for item in contract["scenarios"]}
    assert len(ids) >= 10
    assert {
        "health-public",
        "store-decision",
        "recall-get",
        "boot",
        "export-json",
    }.issubset(ids)


def test_python_sdk_matches_http_contract_shapes(httpx_mock):
    contract = load_contract()
    client = CortexClient(base_url="http://127.0.0.1:7437", token="ctx_contract_token")

    httpx_mock.add_response(json={"status": "ok", "runtime": {}, "stats": {}})
    client.health()
    health_req = httpx_mock.get_requests()[-1]
    health = scenario(contract, "health-public")
    assert health_req.method == health["http"]["method"]
    assert health_req.url.path == health["http"]["path"]
    assert "Authorization" not in health_req.headers
    assert "X-Cortex-Request" not in health_req.headers

    store = scenario(contract, "store-decision")
    httpx_mock.add_response(json={"stored": True, "entry": {}})
    body = store["request"]["json"]
    client.store(
        body["decision"],
        context=body["context"],
        source_agent=body["source_agent"],
        source_model=body["source_model"],
        confidence=body["confidence"],
        reasoning_depth=body["reasoning_depth"],
        ttl_seconds=body["ttl_seconds"],
        entry_type=body["type"],
    )
    store_req = httpx_mock.get_requests()[-1]
    assert store_req.method == store["http"]["method"]
    assert store_req.url.path == store["http"]["path"]
    assert store_req.headers["X-Cortex-Request"] == "true"
    assert store_req.headers["Authorization"] == "Bearer ctx_contract_token"
    assert json.loads(store_req.read().decode("utf-8")) == body

    recall = scenario(contract, "recall-get")
    httpx_mock.add_response(json={"results": [], "budget": 200, "spent": 0, "saved": 200})
    query = recall["request"]["query"]
    client.recall(
        query["q"],
        budget=query["budget"],
        k=query["k"],
        agent=query["agent"],
    )
    recall_req = httpx_mock.get_requests()[-1]
    assert recall_req.method == recall["http"]["method"]
    assert recall_req.url.path == recall["http"]["path"]
    for key, value in query.items():
        assert recall_req.url.params[key] == str(value)

    boot = scenario(contract, "boot")
    httpx_mock.add_response(json={"prompt": "", "tokenEstimate": 0, "savings": {}})
    boot_query = boot["request"]["query"]
    client.boot(agent=boot_query["agent"], budget=boot_query["budget"])
    boot_req = httpx_mock.get_requests()[-1]
    assert boot_req.method == boot["http"]["method"]
    assert boot_req.url.path == boot["http"]["path"]
    for key, value in boot_query.items():
        assert boot_req.url.params[key] == str(value)

    export = scenario(contract, "export-json")
    httpx_mock.add_response(json={"memories": [], "decisions": []})
    client.export(fmt=export["request"]["query"]["format"])
    export_req = httpx_mock.get_requests()[-1]
    assert export_req.method == export["http"]["method"]
    assert export_req.url.path == export["http"]["path"]
    assert export_req.url.params["format"] == "json"

// SPDX-License-Identifier: MIT
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { CortexClient } from "../dist/index.js";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const SPEC_PATH = join(ROOT, "specs", "cortex-adapter-contract.yaml");

function loadContract() {
  return JSON.parse(readFileSync(SPEC_PATH, "utf8"));
}

function scenario(contract, scenarioId) {
  const item = contract.scenarios.find((entry) => entry.id === scenarioId);
  assert.ok(item, `missing scenario ${scenarioId}`);
  return item;
}

function okJson(payload) {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

test("contract spec has required scenarios", () => {
  const contract = loadContract();
  assert.equal(contract.schema, "cortex.adapter.contract");
  assert.equal(contract.version, "0.6.0");
  const ids = new Set(contract.scenarios.map((entry) => entry.id));
  assert.ok(ids.size >= 10);
  for (const id of ["health-public", "store-decision", "recall-get", "boot", "export-json"]) {
    assert.ok(ids.has(id), `missing scenario ${id}`);
  }
});

test("typescript sdk matches HTTP contract shapes", async () => {
  const contract = loadContract();
  const calls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input, init = {}) => {
    calls.push({ input, init });
    return okJson({ status: "ok", runtime: {}, stats: {}, stored: true, entry: {}, results: [], memories: [], decisions: [] });
  };

  try {
    const client = new CortexClient({
      baseUrl: "http://127.0.0.1:7437",
      token: "ctx_contract_token",
    });

    const health = scenario(contract, "health-public");
    await client.health();
    const healthCall = calls.at(-1);
    assert.equal(new URL(String(healthCall.input)).pathname, health.http.path);
    assert.equal(healthCall.init.headers, undefined);

    const store = scenario(contract, "store-decision");
    const body = store.request.json;
    await client.store(body.decision, {
      context: body.context,
      entryType: body.type,
      sourceAgent: body.source_agent,
      sourceModel: body.source_model,
      confidence: body.confidence,
      reasoningDepth: body.reasoning_depth,
      ttlSeconds: body.ttl_seconds,
    });
    const storeCall = calls.at(-1);
    assert.equal(storeCall.init.method, store.http.method);
    assert.equal(new URL(String(storeCall.input)).pathname, store.http.path);
    assert.equal(storeCall.init.headers["X-Cortex-Request"], "true");
    assert.equal(storeCall.init.headers.Authorization, "Bearer ctx_contract_token");
    assert.deepEqual(JSON.parse(String(storeCall.init.body)), body);

    const recall = scenario(contract, "recall-get");
    const query = recall.request.query;
    await client.recall(query.q, {
      budget: query.budget,
      k: query.k,
      agent: query.agent,
    });
    const recallCall = calls.at(-1);
    const recallUrl = new URL(String(recallCall.input));
    assert.equal(recallUrl.pathname, recall.http.path);
    for (const [key, value] of Object.entries(query)) {
      assert.equal(recallUrl.searchParams.get(key), String(value));
    }

    const boot = scenario(contract, "boot");
    const bootQuery = boot.request.query;
    await client.boot(bootQuery.agent, bootQuery.budget);
    const bootUrl = new URL(String(calls.at(-1).input));
    assert.equal(bootUrl.pathname, boot.http.path);
    for (const [key, value] of Object.entries(bootQuery)) {
      assert.equal(bootUrl.searchParams.get(key), String(value));
    }

    const exportScenario = scenario(contract, "export-json");
    await client.export(exportScenario.request.query.format);
    const exportUrl = new URL(String(calls.at(-1).input));
    assert.equal(exportUrl.pathname, exportScenario.http.path);
    assert.equal(exportUrl.searchParams.get("format"), "json");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

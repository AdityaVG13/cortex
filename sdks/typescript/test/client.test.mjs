// SPDX-License-Identifier: MIT
import assert from "node:assert/strict";
import test from "node:test";

import { CortexClient } from "../dist/index.js";

function okJson(payload) {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

test("recall sends auth + cortex headers and query params", async () => {
  const calls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input, init = {}) => {
    calls.push({ input, init });
    return okJson({ items: [] });
  };

  try {
    const client = new CortexClient({
      baseUrl: "http://127.0.0.1:7437/",
      token: "ctx_test_token",
      timeout: 5_000,
    });
    await client.recall("deploy gate", { budget: 222, k: 7, agent: "codex" });
  } finally {
    globalThis.fetch = originalFetch;
  }

  assert.equal(calls.length, 1);
  const [{ input, init }] = calls;
  const requestUrl = new URL(String(input));
  assert.equal(requestUrl.pathname, "/recall");
  assert.equal(requestUrl.searchParams.get("q"), "deploy gate");
  assert.equal(requestUrl.searchParams.get("budget"), "222");
  assert.equal(requestUrl.searchParams.get("k"), "7");
  assert.equal(requestUrl.searchParams.get("agent"), "codex");

  const headers = init.headers;
  assert.equal(headers["X-Cortex-Request"], "true");
  assert.equal(headers.Authorization, "Bearer ctx_test_token");
});

test("store serializes optional fields to daemon schema", async () => {
  const calls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input, init = {}) => {
    calls.push({ input, init });
    return okJson({ ok: true });
  };

  try {
    const client = new CortexClient({
      baseUrl: "http://127.0.0.1:7437",
      token: "ctx_store_token",
    });
    await client.store("Prefer vector fallback", {
      context: "Canary trials",
      entryType: "decision",
      sourceAgent: "ts-suite",
      sourceModel: "gpt-5.4",
      confidence: 0.93,
      reasoningDepth: "high",
      ttlSeconds: 3600,
    });
  } finally {
    globalThis.fetch = originalFetch;
  }

  assert.equal(calls.length, 1);
  const [{ init }] = calls;
  assert.equal(init.method, "POST");
  const parsedBody = JSON.parse(String(init.body));
  assert.equal(parsedBody.decision, "Prefer vector fallback");
  assert.equal(parsedBody.context, "Canary trials");
  assert.equal(parsedBody.type, "decision");
  assert.equal(parsedBody.source_agent, "ts-suite");
  assert.equal(parsedBody.source_model, "gpt-5.4");
  assert.equal(parsedBody.confidence, 0.93);
  assert.equal(parsedBody.reasoning_depth, "high");
  assert.equal(parsedBody.ttl_seconds, 3600);
});

test("health uses health endpoint without cortex auth headers", async () => {
  const calls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input, init = {}) => {
    calls.push({ input, init });
    return okJson({ ok: true });
  };

  try {
    const client = new CortexClient({
      baseUrl: "http://127.0.0.1:7437",
      token: "ctx_health_token",
    });
    await client.health();
  } finally {
    globalThis.fetch = originalFetch;
  }

  assert.equal(calls.length, 1);
  const [{ input, init }] = calls;
  const requestUrl = new URL(String(input));
  assert.equal(requestUrl.pathname, "/health");
  assert.equal(init.headers, undefined);
});

test("remote baseUrl requires explicit token", () => {
  assert.throws(
    () => new CortexClient({ baseUrl: "https://team.example.com" }),
    /requires explicit token/i
  );
});

test("formatRecallContext keeps memory text and appends compact metrics", () => {
  const client = new CortexClient({
    baseUrl: "http://127.0.0.1:7437",
    token: "ctx_format_token",
  });
  const context = client.formatRecallContext(
    {
      results: [
        { source: "memory::1", method: "keyword", excerpt: "Business Administration", relevance: 0.9 },
        { source: "memory::2", method: "semantic", excerpt: "Volunteer event was Feb 14", relevance: 0.8 },
      ],
      budget: 300,
      spent: 210,
      saved: 90,
      mode: "balanced",
    },
    { maxItems: 1, includeMetrics: true },
  );
  assert.match(context, /Business Administration/);
  assert.doesNotMatch(context, /Volunteer event/);
  assert.match(context, /\[retrieval-metrics\]/);
  assert.match(context, /"budget":300/);
});

test("recallForPrompt reuses recall payload", async () => {
  const calls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (input, init = {}) => {
    calls.push({ input, init });
    return okJson({
      results: [
        { source: "memory::1", method: "keyword", excerpt: "Prompt-ready excerpt", relevance: 0.8 },
      ],
      budget: 200,
      spent: 100,
      saved: 100,
    });
  };
  try {
    const client = new CortexClient({
      baseUrl: "http://127.0.0.1:7437",
      token: "ctx_prompt_token",
    });
    const context = await client.recallForPrompt("what happened", { includeMetrics: false });
    assert.match(context, /Prompt-ready excerpt/);
    assert.doesNotMatch(context, /\[retrieval-metrics\]/);
  } finally {
    globalThis.fetch = originalFetch;
  }
  assert.equal(calls.length, 1);
});

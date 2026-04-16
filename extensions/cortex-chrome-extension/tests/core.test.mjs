import test from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_AGENT,
  DEFAULT_CORTEX_URL,
  isLoopbackUrl,
  normalizeAgent,
  normalizeCortexUrl,
  normalizeLocalCortexUrl,
  normalizePositiveInteger,
  originPatternForUrl,
  sanitizeDecision
} from "../src/core.js";

test("normalizeCortexUrl trims paths/search/hash and enforces protocol", () => {
  const normalized = normalizeCortexUrl(" http://127.0.0.1:7437/api/v1/?x=1#frag ");
  assert.equal(normalized, "http://127.0.0.1:7437/api/v1");
});

test("normalizeCortexUrl rejects non-http protocols", () => {
  assert.throws(
    () => normalizeCortexUrl("ftp://127.0.0.1:7437"),
    /must use http or https/i
  );
});

test("normalizeLocalCortexUrl accepts loopback and rejects remote hosts", () => {
  assert.equal(
    normalizeLocalCortexUrl("http://127.0.0.1:7437/api"),
    "http://127.0.0.1:7437/api"
  );
  assert.throws(
    () => normalizeLocalCortexUrl("https://team.example.com"),
    /only supports http loopback cortex urls/i
  );
});

test("originPatternForUrl returns chrome permissions origin format", () => {
  assert.equal(originPatternForUrl("https://team.example.com/base"), "https://team.example.com/*");
});

test("isLoopbackUrl recognizes localhost and 127.0.0.1", () => {
  assert.equal(isLoopbackUrl("http://localhost:7437"), true);
  assert.equal(isLoopbackUrl("http://127.0.0.1:7437"), true);
  assert.equal(isLoopbackUrl("https://team.example.com"), false);
});

test("normalizePositiveInteger returns fallback for invalid values", () => {
  assert.equal(normalizePositiveInteger("7", 2), 7);
  assert.equal(normalizePositiveInteger("0", 2), 2);
  assert.equal(normalizePositiveInteger("nan", 2), 2);
});

test("sanitizeDecision requires non-empty trimmed text", () => {
  assert.equal(sanitizeDecision(" ship this "), "ship this");
  assert.throws(() => sanitizeDecision("   "), /cannot be empty/i);
});

test("normalizeAgent falls back to default", () => {
  assert.equal(normalizeAgent("codex"), "codex");
  assert.equal(normalizeAgent(" "), DEFAULT_AGENT);
});

test("default URL stays parseable as extension boot fallback", () => {
  assert.equal(normalizeCortexUrl(DEFAULT_CORTEX_URL), DEFAULT_CORTEX_URL);
});

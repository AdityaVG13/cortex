// SPDX-License-Identifier: MIT

import {
  DEFAULT_AGENT,
  DEFAULT_CORTEX_URL,
  DEFAULT_RECALL_BUDGET,
  DEFAULT_RECALL_K,
  DEFAULT_TIMEOUT_MS,
  normalizeAgent,
  normalizeCortexUrl,
  normalizePositiveInteger
} from "./core.js";

const STORAGE_KEY = "cortex.extension.settings.v1";

export async function loadSettings() {
  const wrapped = await chrome.storage.local.get(STORAGE_KEY);
  const raw = wrapped?.[STORAGE_KEY] ?? {};
  const cortexUrl = safeNormalizeUrl(raw.cortexUrl, DEFAULT_CORTEX_URL);
  return {
    cortexUrl,
    apiKey: String(raw.apiKey ?? "").trim(),
    agent: normalizeAgent(raw.agent),
    recallBudget: normalizePositiveInteger(raw.recallBudget, DEFAULT_RECALL_BUDGET),
    recallK: normalizePositiveInteger(raw.recallK, DEFAULT_RECALL_K),
    timeoutMs: normalizePositiveInteger(raw.timeoutMs, DEFAULT_TIMEOUT_MS)
  };
}

export async function saveSettings(input) {
  const next = {
    cortexUrl: normalizeCortexUrl(input.cortexUrl ?? DEFAULT_CORTEX_URL),
    apiKey: String(input.apiKey ?? "").trim(),
    agent: normalizeAgent(input.agent),
    recallBudget: normalizePositiveInteger(input.recallBudget, DEFAULT_RECALL_BUDGET),
    recallK: normalizePositiveInteger(input.recallK, DEFAULT_RECALL_K),
    timeoutMs: normalizePositiveInteger(input.timeoutMs, DEFAULT_TIMEOUT_MS)
  };
  await chrome.storage.local.set({ [STORAGE_KEY]: next });
  return next;
}

function safeNormalizeUrl(rawValue, fallback) {
  try {
    return normalizeCortexUrl(rawValue);
  } catch (_error) {
    return fallback;
  }
}

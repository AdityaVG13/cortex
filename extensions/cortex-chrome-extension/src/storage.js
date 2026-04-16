// SPDX-License-Identifier: MIT

import {
  DEFAULT_AGENT,
  DEFAULT_CORTEX_URL,
  DEFAULT_RECALL_BUDGET,
  DEFAULT_RECALL_K,
  DEFAULT_TIMEOUT_MS,
  normalizeAgent,
  normalizeLocalCortexUrl,
  normalizePositiveInteger
} from "./core.js";

const STORAGE_KEY = "cortex.extension.settings.v1";
const API_KEY_SESSION_KEY = "cortex.extension.settings.apiKey.session.v1";

export async function loadSettings() {
  const wrapped = await chrome.storage.local.get(STORAGE_KEY);
  const sessionWrapped = await chrome.storage.session.get(API_KEY_SESSION_KEY);
  const raw = wrapped?.[STORAGE_KEY] ?? {};
  const cortexUrl = safeNormalizeLocalUrl(raw.cortexUrl, DEFAULT_CORTEX_URL);
  const persistedApiKey = String(raw.apiKey ?? "").trim();
  const rememberApiKey =
    raw.rememberApiKey === true ||
    (raw.rememberApiKey === undefined && persistedApiKey.length > 0);
  const sessionApiKey = String(sessionWrapped?.[API_KEY_SESSION_KEY] ?? "").trim();
  return {
    cortexUrl,
    apiKey: rememberApiKey ? persistedApiKey : sessionApiKey,
    rememberApiKey,
    includePageMetadata: raw.includePageMetadata === true,
    agent: normalizeAgent(raw.agent),
    recallBudget: normalizePositiveInteger(raw.recallBudget, DEFAULT_RECALL_BUDGET),
    recallK: normalizePositiveInteger(raw.recallK, DEFAULT_RECALL_K),
    timeoutMs: normalizePositiveInteger(raw.timeoutMs, DEFAULT_TIMEOUT_MS)
  };
}

export async function saveSettings(input) {
  const normalizedApiKey = String(input.apiKey ?? "").trim();
  const rememberApiKey = Boolean(input.rememberApiKey);
  const next = {
    cortexUrl: normalizeLocalCortexUrl(input.cortexUrl ?? DEFAULT_CORTEX_URL),
    apiKey: rememberApiKey ? normalizedApiKey : "",
    rememberApiKey,
    includePageMetadata: input.includePageMetadata === true,
    agent: normalizeAgent(input.agent),
    recallBudget: normalizePositiveInteger(input.recallBudget, DEFAULT_RECALL_BUDGET),
    recallK: normalizePositiveInteger(input.recallK, DEFAULT_RECALL_K),
    timeoutMs: normalizePositiveInteger(input.timeoutMs, DEFAULT_TIMEOUT_MS)
  };
  await chrome.storage.local.set({ [STORAGE_KEY]: next });
  if (rememberApiKey) {
    await chrome.storage.session.remove(API_KEY_SESSION_KEY);
  } else if (normalizedApiKey) {
    await chrome.storage.session.set({ [API_KEY_SESSION_KEY]: normalizedApiKey });
  } else {
    await chrome.storage.session.remove(API_KEY_SESSION_KEY);
  }
  return {
    ...next,
    apiKey: normalizedApiKey
  };
}

function safeNormalizeLocalUrl(rawValue, fallback) {
  try {
    return normalizeLocalCortexUrl(rawValue);
  } catch (_error) {
    return fallback;
  }
}

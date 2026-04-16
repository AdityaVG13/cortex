// SPDX-License-Identifier: MIT

import {
  DEFAULT_AGENT,
  DEFAULT_CORTEX_URL,
  DEFAULT_RECALL_BUDGET,
  DEFAULT_RECALL_K,
  DEFAULT_TIMEOUT_MS
} from "../src/core.js";

const elements = {
  url: document.getElementById("cortex-url"),
  apiKey: document.getElementById("api-key"),
  rememberApiKey: document.getElementById("remember-api-key"),
  includePageMetadata: document.getElementById("include-page-metadata"),
  agent: document.getElementById("agent-name"),
  recallBudget: document.getElementById("recall-budget"),
  recallK: document.getElementById("recall-k"),
  timeoutMs: document.getElementById("timeout-ms"),
  status: document.getElementById("status")
};

document.getElementById("save-settings").addEventListener("click", async () => {
  try {
    const payload = collectPayload();
    const saved = await callBackground("settings:save", payload);
    const persistence = saved.rememberApiKey ? "persisted" : "session-only";
    setStatus(`Saved settings for ${saved.cortexUrl} (API key: ${persistence}).`, false);
  } catch (error) {
    setStatus(String(error.message ?? error), true);
  }
});

document.getElementById("test-connection").addEventListener("click", async () => {
  try {
    const health = await callBackground("cortex:health", {});
    const status = String(health?.status ?? "unknown");
    const ready = String(health?.ready ?? "n/a");
    setStatus(`Health OK. status=${status} ready=${ready}`, false);
  } catch (error) {
    setStatus(String(error.message ?? error), true);
  }
});

void initialize();

async function initialize() {
  try {
    const settings = await callBackground("settings:get", {});
    elements.url.value = settings.cortexUrl ?? DEFAULT_CORTEX_URL;
    elements.apiKey.value = settings.apiKey ?? "";
    elements.rememberApiKey.checked = settings.rememberApiKey === true;
    elements.includePageMetadata.checked = settings.includePageMetadata === true;
    elements.agent.value = settings.agent ?? DEFAULT_AGENT;
    elements.recallBudget.value = String(settings.recallBudget ?? DEFAULT_RECALL_BUDGET);
    elements.recallK.value = String(settings.recallK ?? DEFAULT_RECALL_K);
    elements.timeoutMs.value = String(settings.timeoutMs ?? DEFAULT_TIMEOUT_MS);
    if (settings.hasOriginPermission) {
      setStatus("Settings loaded. Loopback Cortex URL is ready.", false);
    } else {
      setStatus(
        "Settings loaded. This build only supports local loopback Cortex endpoints.",
        true
      );
    }
  } catch (error) {
    setStatus(String(error.message ?? error), true);
  }
}

function collectPayload() {
  return {
    cortexUrl: elements.url.value.trim() || DEFAULT_CORTEX_URL,
    apiKey: elements.apiKey.value.trim(),
    rememberApiKey: elements.rememberApiKey.checked,
    includePageMetadata: elements.includePageMetadata.checked,
    agent: elements.agent.value.trim() || DEFAULT_AGENT,
    recallBudget: Number.parseInt(elements.recallBudget.value, 10) || DEFAULT_RECALL_BUDGET,
    recallK: Number.parseInt(elements.recallK.value, 10) || DEFAULT_RECALL_K,
    timeoutMs: Number.parseInt(elements.timeoutMs.value, 10) || DEFAULT_TIMEOUT_MS
  };
}

function callBackground(action, payload) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage({ action, payload }, (response) => {
      if (chrome.runtime.lastError) {
        reject(new Error(chrome.runtime.lastError.message));
        return;
      }
      if (!response?.ok) {
        reject(new Error(response?.error ?? "Request failed."));
        return;
      }
      resolve(response.result);
    });
  });
}

function setStatus(message, isError) {
  elements.status.textContent = message;
  elements.status.classList.toggle("error", Boolean(isError));
  elements.status.classList.toggle("ok", !isError);
}

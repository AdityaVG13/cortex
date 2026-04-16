// SPDX-License-Identifier: MIT

import { DEFAULT_RECALL_K } from "../src/core.js";

const statusEl = document.getElementById("status");
const decisionEl = document.getElementById("store-decision");
const contextEl = document.getElementById("store-context");
const queryEl = document.getElementById("recall-query");
const recallKEl = document.getElementById("recall-k");
const resultsEl = document.getElementById("recall-results");

document.getElementById("open-options").addEventListener("click", () => {
  chrome.runtime.openOptionsPage();
});

document.getElementById("store-submit").addEventListener("click", async () => {
  const decision = decisionEl.value.trim();
  const context = contextEl.value.trim();
  if (!decision) {
    setStatus("Memory text cannot be empty.", true);
    return;
  }
  try {
    await callBackground("cortex:store", {
      decision,
      context,
      entryType: "note"
    });
    decisionEl.value = "";
    contextEl.value = "";
    setStatus("Stored in Cortex.", false);
  } catch (error) {
    setStatus(String(error.message ?? error), true);
  }
});

document.getElementById("recall-submit").addEventListener("click", async () => {
  const query = queryEl.value.trim();
  if (!query) {
    setStatus("Recall query cannot be empty.", true);
    return;
  }
  const k = Number.parseInt(recallKEl.value, 10) || DEFAULT_RECALL_K;
  try {
    const payload = await callBackground("cortex:recall", { query, k });
    renderResults(payload?.results ?? []);
    setStatus("Recall complete.", false);
  } catch (error) {
    setStatus(String(error.message ?? error), true);
  }
});

void initialize();

async function initialize() {
  recallKEl.value = String(DEFAULT_RECALL_K);
  try {
    const settings = await callBackground("settings:get", {});
    if (!settings?.hasOriginPermission) {
      setStatus(
        "Origin permission is missing for configured Cortex URL. Open Options and re-save.",
        true
      );
      return;
    }
    await callBackground("cortex:health", {});
    setStatus(`Connected: ${settings.cortexUrl}`, false);
  } catch (error) {
    setStatus(String(error.message ?? error), true);
  }
}

function renderResults(results) {
  resultsEl.replaceChildren();
  if (!Array.isArray(results) || results.length === 0) {
    const empty = document.createElement("li");
    empty.textContent = "No results.";
    resultsEl.appendChild(empty);
    return;
  }
  for (const item of results) {
    const row = document.createElement("li");
    const meta = document.createElement("div");
    meta.className = "meta";
    const source = String(item?.source ?? "unknown");
    const relevance = typeof item?.relevance === "number" ? item.relevance.toFixed(3) : "n/a";
    meta.textContent = `${source} | relevance ${relevance}`;
    const excerpt = document.createElement("div");
    excerpt.textContent = String(item?.excerpt ?? "");
    row.append(meta, excerpt);
    resultsEl.appendChild(row);
  }
}

function setStatus(message, isError) {
  statusEl.textContent = message;
  statusEl.classList.toggle("error", Boolean(isError));
  statusEl.classList.toggle("ok", !isError);
}

function callBackground(action, payload) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage({ action, payload }, (response) => {
      if (chrome.runtime.lastError) {
        reject(new Error(chrome.runtime.lastError.message));
        return;
      }
      if (!response?.ok) {
        reject(new Error(response?.error ?? "Extension request failed."));
        return;
      }
      resolve(response.result);
    });
  });
}

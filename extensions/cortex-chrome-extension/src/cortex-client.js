// SPDX-License-Identifier: MIT

import {
  normalizeAgent,
  normalizePositiveInteger,
  sanitizeDecision
} from "./core.js";

export async function healthcheck(settings) {
  return requestJson(settings, {
    method: "GET",
    path: "/health",
    includeAuth: false,
    includeCortexRequestHeader: false
  });
}

export async function storeDecision(settings, payload) {
  const decision = sanitizeDecision(payload.decision);
  const context = String(payload.context ?? "").trim();
  const entryType = String(payload.entryType ?? "note").trim() || "note";
  return requestJson(settings, {
    method: "POST",
    path: "/store",
    includeAuth: true,
    body: {
      decision,
      context,
      type: entryType,
      source_agent: normalizeAgent(payload.sourceAgent ?? settings.agent)
    }
  });
}

export async function recall(settings, payload) {
  const q = String(payload.query ?? "").trim();
  if (!q) {
    throw new Error("Recall query cannot be empty.");
  }
  const budget = normalizePositiveInteger(payload.budget, settings.recallBudget);
  const k = normalizePositiveInteger(payload.k, settings.recallK);
  return requestJson(settings, {
    method: "POST",
    path: "/recall",
    includeAuth: true,
    body: {
      q,
      budget,
      k,
      agent: normalizeAgent(payload.agent ?? settings.agent)
    }
  });
}

async function requestJson(settings, config) {
  const controller = new AbortController();
  const timeoutId = setTimeout(
    () => controller.abort("timeout"),
    normalizePositiveInteger(settings.timeoutMs, 8_000)
  );
  const url = `${settings.cortexUrl}${config.path}`;
  try {
    const response = await fetch(url, {
      method: config.method,
      headers: requestHeaders(settings, config),
      body: config.body ? JSON.stringify(config.body) : undefined,
      signal: controller.signal
    });
    if (!response.ok) {
      throw new Error(`Cortex request failed (${response.status})`);
    }
    const text = await response.text();
    return text ? JSON.parse(text) : {};
  } catch (error) {
    if (error?.name === "AbortError") {
      throw new Error("Cortex request timed out.");
    }
    throw error;
  } finally {
    clearTimeout(timeoutId);
  }
}

function requestHeaders(settings, config) {
  const headers = {
    "Content-Type": "application/json"
  };
  if (config.includeCortexRequestHeader !== false) {
    headers["X-Cortex-Request"] = "true";
  }
  if (config.includeAuth) {
    const apiKey = String(settings.apiKey ?? "").trim();
    if (!apiKey) {
      throw new Error("Cortex API key is missing. Set it in extension options.");
    }
    headers["Authorization"] = `Bearer ${apiKey}`;
  }
  return headers;
}

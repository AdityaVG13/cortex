/**
 * Cortex Memory SDK -- TypeScript fetch client for the Cortex daemon REST API.
 *
 * Usage:
 *   import { CortexClient } from "@cortex-memory/client";
 *   const client = new CortexClient();
 *   const health = await client.health();
 *   const results = await client.recall("What is Cortex?");
 */

// SPDX-License-Identifier: MIT

import { readFileSync } from "fs";
import { join } from "path";
import type {
  CortexBootResult as BootResult,
  CortexDiaryResult as DiaryResult,
  CortexExportResult as ExportResult,
  CortexForgetResult as ForgetResult,
  CortexHealthResult as HealthResult,
  CortexImportPayload as ImportPayload,
  CortexImportResult as ImportResult,
  CortexPeekResult as PeekResult,
  CortexRecallResult as RecallResult,
  CortexShutdownResult as ShutdownResult,
  CortexStoreRequest as StoreRequest,
  CortexStoreResult as StoreResult,
  JsonObject,
  JsonPrimitive,
  JsonValue,
} from "./types.js";

export type {
  JsonPrimitive,
  JsonValue,
  JsonObject,
  CortexRecallItem as RecallItem,
  CortexRecallResult as RecallResult,
  CortexPeekItem as PeekMatch,
  CortexPeekResult as PeekResult,
  CortexStoreConflict as StoreConflict,
  CortexStoreEntry as StoreEntry,
  CortexStoreRequest as StoreRequest,
  CortexStoreResult as StoreResult,
  CortexHealthStats as HealthStats,
  CortexHealthRuntime as HealthRuntime,
  CortexHealthResult as HealthResult,
  CortexExportMemoryRow as ExportMemoryRow,
  CortexExportDecisionRow as ExportDecisionRow,
  CortexExportResult as ExportResult,
  CortexBootCapsule as BootCapsule,
  CortexBootSavings as BootSavings,
  CortexBootResult as BootResult,
  CortexImportMemory as ImportMemory,
  CortexImportDecision as ImportDecision,
  CortexImportPayload as ImportPayload,
  CortexImportResult as ImportResult,
  CortexDiaryResult as DiaryResult,
  CortexForgetResult as ForgetResult,
  CortexShutdownResult as ShutdownResult,
} from "./types.js";

const DEFAULT_BASE = "http://127.0.0.1:7437";

function readToken(): string | undefined {
  const home = process.env.USERPROFILE || process.env.HOME || ".";
  try {
    return readFileSync(join(home, ".cortex", "cortex.token"), "utf-8").trim() || undefined;
  } catch {
    return undefined;
  }
}

function isLoopbackBaseUrl(baseUrl: string): boolean {
  try {
    const parsed = new URL(baseUrl);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return false;
    }
    const host = parsed.hostname.toLowerCase();
    return host === "127.0.0.1" || host === "localhost" || host === "::1";
  } catch {
    return false;
  }
}

export class CortexClient {
  private baseUrl: string;
  private token?: string;
  private timeout: number;
  private sourceAgent: string;

  constructor(options?: { baseUrl?: string; token?: string; timeout?: number; sourceAgent?: string }) {
    this.baseUrl = (options?.baseUrl ?? DEFAULT_BASE).replace(/\/$/, "");
    if (options?.token) {
      this.token = options.token;
    } else if (isLoopbackBaseUrl(this.baseUrl)) {
      this.token = readToken();
    } else {
      throw new Error(
        "Remote Cortex baseUrl requires explicit token. Pass token when using non-loopback targets."
      );
    }
    this.timeout = options?.timeout ?? 10_000;
    this.sourceAgent = (options?.sourceAgent ?? "typescript-sdk").trim() || "typescript-sdk";
  }

  private headers(): Record<string, string> {
    const h: Record<string, string> = {
      "X-Cortex-Request": "true",
      "X-Source-Agent": this.sourceAgent,
    };
    if (this.token) h["Authorization"] = `Bearer ${this.token}`;
    return h;
  }

  private async get<T>(path: string, params?: Record<string, string | number>): Promise<T> {
    const url = new URL(`${this.baseUrl}${path}`);
    if (params) {
      for (const [k, v] of Object.entries(params)) {
        url.searchParams.set(k, String(v));
      }
    }
    const resp = await fetch(url.toString(), {
      headers: this.headers(),
      signal: AbortSignal.timeout(this.timeout),
    });
    if (!resp.ok) throw new Error(`Cortex ${path}: ${resp.status} ${resp.statusText}`);
    return resp.json() as Promise<T>;
  }

  private async post<T>(path: string, body?: JsonObject): Promise<T> {
    const resp = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { ...this.headers(), "Content-Type": "application/json" },
      body: JSON.stringify(body ?? {}),
      signal: AbortSignal.timeout(this.timeout),
    });
    if (!resp.ok) throw new Error(`Cortex ${path}: ${resp.status} ${resp.statusText}`);
    return resp.json() as Promise<T>;
  }

  // ── Public API ─────────────────────────────────────────────────────────────

  async health(): Promise<HealthResult> {
    const url = `${this.baseUrl}/health`;
    const resp = await fetch(url, { signal: AbortSignal.timeout(this.timeout) });
    if (!resp.ok) throw new Error(`Health check failed: ${resp.status}`);
    return resp.json() as Promise<HealthResult>;
  }

  async recall(query: string, options?: { budget?: number; k?: number; agent?: string }): Promise<RecallResult> {
    const params: Record<string, string | number> = { q: query };
    if (options?.budget !== undefined) params.budget = options.budget;
    if (options?.k !== undefined) params.k = options.k;
    if (options?.agent) params.agent = options.agent;
    return this.get("/recall", params);
  }

  formatRecallContext(
    recall: RecallResult,
    options?: { includeMetrics?: boolean; maxItems?: number },
  ): string {
    const includeMetrics = options?.includeMetrics ?? true;
    const maxItems = options?.maxItems;
    const results = Array.isArray(recall?.results) ? recall.results : [];
    const entries: string[] = [];
    const limit = maxItems === undefined ? results.length : Math.max(0, maxItems);
    for (const item of results.slice(0, limit)) {
      const excerpt = String(item?.excerpt ?? "").trim();
      if (!excerpt) continue;
      const source = String(item?.source ?? "").trim();
      const method = String(item?.method ?? "").trim();
      const labelParts: string[] = [];
      if (source) labelParts.push(`source=${source}`);
      if (method) labelParts.push(`method=${method}`);
      const suffix = labelParts.length ? ` (${labelParts.join(", ")})` : "";
      entries.push(`## Memory ${entries.length + 1}${suffix}\n${excerpt}`);
    }
    if (includeMetrics) {
      const metricsKeys = ["budget", "spent", "saved", "count", "mode", "tier", "cached", "latencyMs"] as const;
      const compactMetrics: Record<string, string | number | boolean> = {};
      for (const key of metricsKeys) {
        const value = recall[key];
        if (value !== undefined && value !== null) compactMetrics[key] = value;
      }
      if (Object.keys(compactMetrics).length > 0) {
        entries.push(`[retrieval-metrics] ${JSON.stringify(compactMetrics)}`);
      }
    }
    return entries.join("\n\n").trim();
  }

  async recallForPrompt(
    query: string,
    options?: { budget?: number; k?: number; agent?: string; includeMetrics?: boolean; maxItems?: number },
  ): Promise<string> {
    const recallPayload = await this.recall(query, {
      budget: options?.budget,
      k: options?.k,
      agent: options?.agent,
    });
    return this.formatRecallContext(recallPayload, {
      includeMetrics: options?.includeMetrics,
      maxItems: options?.maxItems,
    });
  }

  async peek(query: string, k = 10): Promise<PeekResult> {
    return this.get("/peek", { q: query, k });
  }

  async store(
    decision: string,
    options?: {
      context?: string;
      entryType?: string;
      sourceAgent?: string;
      sourceModel?: string;
      confidence?: number;
      reasoningDepth?: string;
      ttlSeconds?: number;
    },
  ): Promise<StoreResult> {
    const body: StoreRequest = {
      decision,
      context: options?.context,
      type: options?.entryType,
      source_agent: options?.sourceAgent ?? this.sourceAgent,
      source_model: options?.sourceModel,
      confidence: options?.confidence,
      reasoning_depth: options?.reasoningDepth,
      ttl_seconds: options?.ttlSeconds,
    };
    return this.post("/store", body);
  }

  async diary(text: string, agent = this.sourceAgent): Promise<DiaryResult> {
    return this.post("/diary", { text, agent });
  }

  async boot(agent = this.sourceAgent, budget = 600): Promise<BootResult> {
    return this.get("/boot", { agent, budget });
  }

  async export(format: "json" | "sql" = "json"): Promise<ExportResult> {
    return this.get("/export", { format });
  }

  async importData(data: ImportPayload): Promise<ImportResult> {
    return this.post("/import", data as JsonObject);
  }

  async forget(source: string): Promise<ForgetResult> {
    return this.post("/forget", { source });
  }

  async shutdown(): Promise<ShutdownResult> {
    return this.post("/shutdown");
  }
}

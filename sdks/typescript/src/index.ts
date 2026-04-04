/**
 * Cortex Memory SDK -- TypeScript fetch client for the Cortex daemon REST API.
 *
 * Usage:
 *   import { CortexClient } from "@cortex-memory/client";
 *   const client = new CortexClient();
 *   const health = await client.health();
 *   const results = await client.recall("What is Cortex?");
 */

import { readFileSync } from "fs";
import { join } from "path";

const DEFAULT_BASE = "http://127.0.0.1:7437";

function readToken(): string | undefined {
  const home = process.env.USERPROFILE || process.env.HOME || ".";
  try {
    return readFileSync(join(home, ".cortex", "cortex.token"), "utf-8").trim() || undefined;
  } catch {
    return undefined;
  }
}

export interface RecallResult {
  results: Array<{
    source: string;
    relevance: number;
    excerpt: string;
    method: string;
    tokens?: number;
  }>;
  budget: number;
  spent: number;
  saved: number;
  mode?: string;
}

export interface StoreResult {
  stored: boolean;
  id?: number;
}

export interface HealthResult {
  status: string;
  version: string;
  stats: Record<string, number>;
}

export interface ExportResult {
  version: number;
  exported_at: string;
  memories: unknown[];
  decisions: unknown[];
}

export class CortexClient {
  private baseUrl: string;
  private token?: string;
  private timeout: number;

  constructor(options?: { baseUrl?: string; token?: string; timeout?: number }) {
    this.baseUrl = (options?.baseUrl ?? DEFAULT_BASE).replace(/\/$/, "");
    this.token = options?.token ?? readToken();
    this.timeout = options?.timeout ?? 10_000;
  }

  private headers(): Record<string, string> {
    const h: Record<string, string> = { "X-Cortex-Request": "true" };
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

  private async post<T>(path: string, body?: Record<string, unknown>): Promise<T> {
    const resp = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { ...this.headers(), "Content-Type": "application/json" },
      body: JSON.stringify(body ?? {}),
      signal: AbortSignal.timeout(this.timeout),
    });
    if (!resp.ok) throw new Error(`Cortex ${path}: ${resp.status} ${resp.statusText}`);
    return resp.json() as Promise<T>;
  }

  // ── Public API ──────────────────────────────────────────────────

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

  async peek(query: string, k = 10): Promise<{ count: number; matches: unknown[] }> {
    return this.get("/peek", { q: query, k });
  }

  async store(text: string, options?: { source?: string; sourceAgent?: string }): Promise<StoreResult> {
    return this.post("/store", {
      text,
      source: options?.source,
      source_agent: options?.sourceAgent ?? "typescript-sdk",
    });
  }

  async diary(text: string, agent = "typescript-sdk"): Promise<unknown> {
    return this.post("/diary", { text, agent });
  }

  async boot(agent = "typescript-sdk", budget = 600): Promise<unknown> {
    return this.get("/boot", { agent, budget });
  }

  async export(format: "json" | "sql" = "json"): Promise<ExportResult> {
    return this.get("/export", { format });
  }

  async importData(data: { memories?: unknown[]; decisions?: unknown[] }): Promise<unknown> {
    return this.post("/import", data);
  }

  async forget(source: string): Promise<unknown> {
    return this.post("/forget", { source });
  }

  async shutdown(): Promise<unknown> {
    return this.post("/shutdown");
  }
}

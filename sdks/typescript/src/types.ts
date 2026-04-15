export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[] | undefined;

export interface JsonObject {
  [key: string]: JsonValue;
}

export interface CortexRecallItem {
  source: string;
  relevance: number;
  excerpt: string;
  method: string;
  tokens?: number;
}

export interface CortexRecallResult {
  results: CortexRecallItem[];
  budget: number;
  spent: number;
  saved: number;
  mode?: string;
  cached?: boolean;
  tier?: string;
  latencyMs?: number;
  count?: number;
  semanticAvailable?: boolean;
}

export interface CortexPeekItem {
  source: string;
  relevance: number;
  method: string;
}

export interface CortexPeekResult {
  count: number;
  matches: CortexPeekItem[];
}

export interface CortexStoreConflict extends JsonObject {
  status?: string;
  conflict_record_id?: number;
  classification?: string;
  source_decision_id?: number | null;
  target_decision_id?: number | null;
  resolution_strategy?: string | null;
  resolved_by?: string | null;
  resolved_at?: string | null;
  similarity_jaccard?: number;
  similarity_cosine?: number;
}

export interface CortexStoreEntry extends JsonObject {
  stored?: boolean;
  action?: string;
  id?: number;
  reason?: string;
  surprise?: number;
  quality?: number;
  status?: string;
  classification?: string;
  target_id?: number;
  conflictWith?: number;
  resolution_strategy?: string;
  supersedes?: number;
  conflict?: CortexStoreConflict;
}

export interface CortexStoreResult {
  stored: boolean;
  entry: CortexStoreEntry;
}

export interface CortexStoreRequest extends JsonObject {
  decision: string;
  context?: string;
  type?: string;
  source_agent?: string;
  source_model?: string;
  confidence?: number;
  reasoning_depth?: string;
  ttl_seconds?: number;
}

export interface CortexImportBase {
  source_agent?: string;
  source_client?: string;
  source_model?: string;
  confidence?: number;
  reasoning_depth?: string;
  trust_score?: number;
  score?: number;
}

export interface CortexImportMemory extends CortexImportBase {
  text: string;
  source?: string;
  type?: string;
  tags?: string;
}

export interface CortexImportDecision extends CortexImportBase {
  decision: string;
  context?: string;
  type?: string;
}

export interface CortexImportPayload {
  memories?: CortexImportMemory[];
  decisions?: CortexImportDecision[];
}

export interface CortexImportResult {
  imported: {
    memories: number;
    decisions: number;
  };
}

export interface CortexExportRowBase extends JsonObject {
  id: number;
  source_agent: string | null;
  source_client: string | null;
  source_model: string | null;
  confidence: number | null;
  reasoning_depth: string | null;
  trust_score: number | null;
  status: string | null;
  score: number | null;
  retrievals: number | null;
  pinned: number | null;
  created_at: string | null;
  updated_at: string | null;
}

export interface CortexExportMemoryRow extends CortexExportRowBase {
  text: string;
  source: string | null;
  type: string | null;
  tags: string | null;
}

export interface CortexExportDecisionRow extends CortexExportRowBase {
  decision: string;
  context: string | null;
  type: string | null;
}

export interface CortexExportResult {
  version: number;
  exported_at: string;
  memories: CortexExportMemoryRow[];
  decisions: CortexExportDecisionRow[];
  memories_count: number;
  decisions_count: number;
}

export interface CortexHealthStats extends JsonObject {
  memories: number;
  decisions: number;
  embeddings: number;
  events: number;
  home: string;
}

export interface CortexHealthRuntime extends JsonObject {
  version: string;
  mode: string;
  port: number;
  db_path: string;
  token_path: string;
  pid_path: string;
  ipc_endpoint: string | null;
  ipc_kind: string | null;
  executable: string;
  owner: string | null;
}

export interface CortexHealthResult extends JsonObject {
  status: string;
  ready: boolean;
  degraded: boolean;
  db_corrupted: boolean;
  embedding_status: string;
  team_mode: boolean;
  db_freelist_pages: number;
  db_size_bytes: number;
  db_soft_limit_bytes: number;
  db_hard_limit_bytes: number;
  db_pressure: string;
  db_soft_utilization: number;
  storage_bytes: number;
  backup_count: number;
  log_bytes: number;
  stats: CortexHealthStats;
  runtime: CortexHealthRuntime;
}

export interface CortexBootCapsule {
  name: string;
  tokens: number;
  priority: number;
  utility?: number;
  truncated?: boolean;
}

export interface CortexBootSavings {
  rawBaseline: number;
  served: number;
  saved: number;
  percent: number;
}

export interface CortexBootResult {
  bootPrompt: string;
  tokenEstimate: number;
  profile: string;
  capsules: CortexBootCapsule[];
  savings: CortexBootSavings;
}

export interface CortexDiaryResult {
  written: boolean;
  agent: string;
  path: string;
}

export interface CortexForgetResult {
  affected: number;
}

export interface CortexShutdownResult {
  shutdown: boolean;
}

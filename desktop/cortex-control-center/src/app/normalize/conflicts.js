export const CONFLICT_CLASSIFICATIONS = new Set(["AGREES", "CONTRADICTS", "REFINES", "UNRELATED"]);
export const CONFLICT_STATUS_FALLBACK = "OPEN";

export function pickDefined(...values) {
  for (const value of values) {
    if (value !== undefined && value !== null && value !== "") {
      return value;
    }
  }
  return null;
}

export function toFiniteNumber(value) {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : null;
}

export function normalizeConflictClassification(value) {
  const normalized = String(value || "").trim().toUpperCase();
  if (!normalized) return "UNSPECIFIED";
  return CONFLICT_CLASSIFICATIONS.has(normalized) ? normalized : normalized;
}

export function normalizeConflictStatus(value) {
  const normalized = String(value || "").trim().toUpperCase();
  if (!normalized) return CONFLICT_STATUS_FALLBACK;
  if (normalized === "IN_PROGRESS") return "OPEN";
  return normalized;
}

export function extractEntityId(value) {
  if (value && typeof value === "object") {
    return pickDefined(value.id, value.decision_id, value.memory_id);
  }
  return value;
}

export function extractEntityAgent(value) {
  if (!value || typeof value !== "object") return "";
  return String(
    pickDefined(
      value.source_agent,
      value.sourceAgent,
      value.agent,
      value.source_client,
      value.client_id,
      ""
    ) || ""
  );
}

export function normalizeConflictEntry(entry, fallbackId) {
  const sourceAgent = String(
    pickDefined(
      entry?.source_agent,
      entry?.sourceAgent,
      entry?.agent,
      entry?.source_client,
      entry?.client_id,
      "unknown"
    ) || "unknown"
  );
  const id = pickDefined(entry?.id, entry?.decision_id, entry?.memory_id, fallbackId);
  return {
    raw: entry || {},
    id,
    sourceAgent,
    decision: String(
      pickDefined(
        entry?.decision,
        entry?.text,
        entry?.content,
        entry?.memory,
        entry?.value,
        "(no decision text)"
      ) || "(no decision text)"
    ),
    context: String(pickDefined(entry?.context, entry?.scope, entry?.topic, "") || ""),
    confidence: toFiniteNumber(pickDefined(entry?.confidence, entry?.source_confidence, entry?.score)),
    trustScore: toFiniteNumber(pickDefined(entry?.trust_score, entry?.trustScore, entry?.trust)),
    createdAt: String(
      pickDefined(entry?.created_at, entry?.createdAt, entry?.detected_at, entry?.timestamp, "") || ""
    ),
    resolvedAt: String(pickDefined(entry?.resolved_at, entry?.resolvedAt, "") || ""),
  };
}

export function normalizeConflictResolution(rawResolution, pair, left, right) {
  const resolution = rawResolution && typeof rawResolution === "object" ? rawResolution : {};
  const winnerRaw = pickDefined(resolution.winner, pair?.winner, pair?.winning_entry);
  const loserRaw = pickDefined(resolution.loser, pair?.loser, pair?.losing_entry, pair?.superseded);
  const winnerId = pickDefined(
    resolution.winner_id,
    resolution.winnerId,
    pair?.winner_id,
    pair?.winnerId,
    extractEntityId(winnerRaw)
  );
  const loserId = pickDefined(
    resolution.loser_id,
    resolution.loserId,
    pair?.loser_id,
    pair?.loserId,
    pair?.superseded_id,
    pair?.supersededId,
    extractEntityId(loserRaw)
  );

  const winnerAgentFallback = winnerId === left?.id ? left.sourceAgent : winnerId === right?.id ? right.sourceAgent : "";
  const loserAgentFallback = loserId === left?.id ? left.sourceAgent : loserId === right?.id ? right.sourceAgent : "";

  const action = String(
    pickDefined(
      resolution.action,
      resolution.resolution,
      resolution.method,
      resolution.policy,
      pair?.resolution,
      pair?.resolution_action
    ) || ""
  ).toLowerCase();

  const method = String(
    pickDefined(
      resolution.method,
      resolution.policy,
      pair?.resolved_by,
      pair?.resolvedBy,
      ""
    ) || ""
  );

  const resolvedBy = String(
    pickDefined(
      resolution.resolved_by,
      resolution.resolvedBy,
      pair?.resolved_by,
      pair?.resolvedBy,
      ""
    ) || ""
  );

  const notes = String(
    pickDefined(
      resolution.notes,
      resolution.reason,
      pair?.resolution_reason,
      pair?.reason,
      ""
    ) || ""
  );

  const trustDelta = toFiniteNumber(
    pickDefined(
      resolution.trust_delta,
      resolution.trustDelta,
      pair?.trust_delta,
      pair?.trustDelta
    )
  );

  if (
    winnerId === null
    && loserId === null
    && !action
    && !method
    && !resolvedBy
    && !notes
    && trustDelta === null
  ) {
    return null;
  }

  return {
    winnerId,
    loserId,
    winnerAgent: String(
      pickDefined(resolution.winner_agent, resolution.winnerAgent, extractEntityAgent(winnerRaw), winnerAgentFallback, "")
      || ""
    ),
    loserAgent: String(
      pickDefined(resolution.loser_agent, resolution.loserAgent, extractEntityAgent(loserRaw), loserAgentFallback, "")
      || ""
    ),
    action,
    method,
    resolvedBy,
    notes,
    trustDelta,
  };
}

export function normalizeConflictPair(pair, index) {
  const leftRaw = pickDefined(
    pair?.left,
    pair?.memory_a,
    pair?.a,
    pair?.first,
    pair?.winner,
    pair?.entries?.[0]
  );
  const rightRaw = pickDefined(
    pair?.right,
    pair?.memory_b,
    pair?.b,
    pair?.second,
    pair?.loser,
    pair?.entries?.[1]
  );

  const left = normalizeConflictEntry(leftRaw, `left-${index}`);
  const right = normalizeConflictEntry(rightRaw, `right-${index}`);
  const conflictId = pickDefined(
    pair?.id,
    pair?.conflict_id,
    pair?.conflictId,
    pair?.pair_id,
    pair?.pairId
  );

  const classification = normalizeConflictClassification(
    pickDefined(
      pair?.classification,
      pair?.conflict_classification,
      pair?.relation,
      pair?.relationship,
      pair?.type,
      pair?.conflict_type
    )
  );
  const createdAt = String(
    pickDefined(
      pair?.created_at,
      pair?.createdAt,
      pair?.detected_at,
      left.createdAt,
      right.createdAt,
      ""
    ) || ""
  );
  const resolvedAt = String(
    pickDefined(pair?.resolved_at, pair?.resolvedAt, left.resolvedAt, right.resolvedAt, "") || ""
  );
  const status = normalizeConflictStatus(
    pickDefined(
      pair?.status,
      pair?.state,
      pair?.resolution_status,
      pair?.conflict_status,
      resolvedAt ? "resolved" : "open"
    )
  );
  const trustDelta = toFiniteNumber(pickDefined(pair?.trust_delta, pair?.trustDelta));
  const resolution = normalizeConflictResolution(
    pickDefined(pair?.resolution, pair?.resolution_detail, pair?.result, pair?.outcome),
    pair,
    left,
    right
  );
  const key = String(conflictId || `${left.id || "left"}-${right.id || "right"}-${index}`);

  return {
    raw: pair || {},
    key,
    conflictId,
    classification,
    status,
    createdAt,
    resolvedAt,
    trustDelta,
    left,
    right,
    resolution,
  };
}

export function normalizeConflictPairsPayload(payload) {
  const rawPairs = Array.isArray(payload?.pairs)
    ? payload.pairs
    : Array.isArray(payload?.conflicts)
      ? payload.conflicts
      : [];
  return rawPairs.map((pair, index) => normalizeConflictPair(pair, index));
}

export function formatConfidencePercent(value) {
  const numeric = toFiniteNumber(value);
  if (numeric === null) return "n/a";
  const normalized = numeric <= 1 ? numeric * 100 : numeric;
  return `${Math.max(0, normalized).toFixed(0)}%`;
}

export function formatTrustScore(value) {
  const numeric = toFiniteNumber(value);
  if (numeric === null) return "n/a";
  return numeric.toFixed(3);
}

export function formatTimestamp(iso) {
  if (!iso) return "unknown";
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) return String(iso);
  return parsed.toLocaleString();
}

export function conflictBadgeClass(prefix, value) {
  const suffix = String(value || "unspecified")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-");
  return `${prefix} ${prefix}-${suffix}`;
}

export function isRouteMissingError(error) {
  const message = String(error?.message || error || "");
  return message.includes("HTTP 404") || message.includes("HTTP 405");
}

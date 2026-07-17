import { sameAgent } from "../../live-surface.js";

export function normalizeSession(session, index) {
  const files = Array.isArray(session?.files)
    ? session.files
    : Array.isArray(session?.files_json)
      ? session.files_json
      : [];
  const startedAt = session?.startedAt ?? session?.started_at ?? null;
  const lastHeartbeat = session?.lastHeartbeat ?? session?.last_heartbeat ?? startedAt;
  const parsedLastHeartbeat = Date.parse(String(lastHeartbeat || ""));
  const lastHeartbeatMs = Number.isFinite(parsedLastHeartbeat) ? parsedLastHeartbeat : 0;
  const expiresAt = session?.expiresAt ?? session?.expires_at ?? null;
  const sessionId = session?.sessionId ?? session?.session_id ?? `${session?.agent || "agent"}-${index}`;

  return {
    ...session,
    files,
    sessionId,
    startedAt,
    lastHeartbeat,
    lastHeartbeatMs,
    expiresAt,
  };
}

export function normalizeSessionAgent(agent) {
  return String(agent || "")
    .replace(/\s*\([^)]*\)\s*$/, "")
    .trim()
    .toLowerCase();
}

export function sessionMatchesAgent(session, agent) {
  const rawSessionAgent = String(session?.agent || "").trim();
  const rawAgent = String(agent || "").trim();
  if (!rawSessionAgent || !rawAgent) return false;
  return sameAgent(rawSessionAgent, rawAgent) || normalizeSessionAgent(rawSessionAgent) === rawAgent.toLowerCase();
}

function normalizeTask(task) { return { ...task, status: { in_progress: "claimed", done: "completed" }[task?.status] || task?.status || "pending", };
}
function stripAgentModel(agent) { return String(agent || "")
    .replace(/\s*\([^)]*\)\s*$/, "")
    .trim()
    .toLowerCase();
}
function sameAgent(left, right) {
  const normalizedLeft = stripAgentModel(left), normalizedRight = stripAgentModel(right);
  return normalizedLeft.length > 0 && normalizedLeft === normalizedRight;
}
function resolveAgentName(agent, knownAgents = []) { const trimmed = String(agent || "").trim();
  if (!trimmed) return "";
  const canonical = knownAgents.find((knownAgent) => sameAgent(knownAgent, trimmed));
  return canonical ? String(canonical).trim() : trimmed;
}
function isTransportSession(session) { return stripAgentModel(session?.agent) === "mcp";
}
// Heartbeat-newest session per operator. Hook rows are `claude-code (opus)`
// while MCP is `claude-code`; a raw-string Map key listed them twice.
function dedupeNormalizedSessions(sessions = []) {
  const sorted = [...sessions].sort((a, b) => (b.lastHeartbeatMs || 0) - (a.lastHeartbeatMs || 0));
  const deduped = new Map();
  for (const session of sorted) {
    const agentRaw = String(session?.agent || "").trim();
    if (!agentRaw) {
      deduped.set(session.sessionId || `session-${deduped.size}`, session);
      continue;
    }
    const key = stripAgentModel(agentRaw) || agentRaw.toLowerCase();
    const existing = deduped.get(key);
    if (!existing) {
      deduped.set(key, session);
      continue;
    }
    const existingHasModel = /\([^)]+\)/.test(String(existing.agent || ""));
    if (/\([^)]+\)/.test(agentRaw) && !existingHasModel) {
      deduped.set(key, session);
    }
  }
  return Array.from(deduped.values()).filter((session) => !isTransportSession(session));
}
function buildKnownAgents(sessions = [], extras = []) { const allAgents = new Map(), registerAgent = (value) => { const agent = String(value || "").trim();
      if (!agent) return;
      const key = stripAgentModel(agent);
      if (!key) return;
      const existing = allAgents.get(key);
      if (!existing) { allAgents.set(key, agent);
        return;
      }
      const existingHasModel = /\([^)]+\)/.test(existing);
      /\([^)]+\)/.test(agent) && !existingHasModel && allAgents.set(key, agent);
    };
  for (const session of sessions) isTransportSession(session) || registerAgent(session?.agent);
  for (const extra of extras) registerAgent(extra);
  return Array.from(allAgents.values()).sort((left, right) => left.localeCompare(right));
}
function filterFeedEntries(entries = [], agentFilter = "") { const needle = String(agentFilter || "")
    .trim()
    .toLowerCase();
  if (!needle) return [...entries];
  return entries.filter((entry) => {
    const agent = String(entry?.agent || "").trim().toLowerCase();
    const stripped = stripAgentModel(entry?.agent);
    return agent === needle || stripped === needle || agent.startsWith(needle) || stripped.startsWith(needle);
  });
}
function canClaimTask(task, operator = "") { return normalizeTask(task).status === "pending" && String(operator || "").trim().length > 0;
}
function canFinalizeTask(task, operator = "") { const normalized = normalizeTask(task);
  return normalized.status === "claimed" && sameAgent(normalized.claimedBy, operator);
}
function canUnlockLock(lock, operator = "") { return !!lock?.path && sameAgent(lock?.agent, operator);
}
function nextFeedAckId(entries = [], operator = "") { const operatorName = String(operator || "").trim();
  return (operatorName && entries.find((entry) => entry?.id && !sameAgent(entry?.agent, operatorName))?.id) || "";
}
export {
  buildKnownAgents, canClaimTask, canFinalizeTask, canUnlockLock, dedupeNormalizedSessions, filterFeedEntries, isTransportSession, nextFeedAckId, normalizeTask,
  resolveAgentName, sameAgent, };

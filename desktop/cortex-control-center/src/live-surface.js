export function normalizeTask(task) {
  const statusMap = {
    in_progress: "claimed",
    done: "completed",
  };

  return {
    ...task,
    status: statusMap[task?.status] || task?.status || "pending",
  };
}

export function sameAgent(left, right) {
  const normalizedLeft = String(left || "").trim().toLowerCase();
  const normalizedRight = String(right || "").trim().toLowerCase();
  return normalizedLeft.length > 0 && normalizedLeft === normalizedRight;
}

export function resolveAgentName(agent, knownAgents = []) {
  const trimmed = String(agent || "").trim();
  if (!trimmed) return "";

  const canonical = knownAgents.find((knownAgent) => sameAgent(knownAgent, trimmed));
  return canonical ? String(canonical).trim() : trimmed;
}

function stripAgentModel(agent) {
  return String(agent || "")
    .replace(/\s*\([^)]*\)\s*$/, "")
    .trim()
    .toLowerCase();
}

export function isTransportSession(session) {
  return stripAgentModel(session?.agent) === "mcp";
}

export function buildKnownAgents(sessions = [], extras = []) {
  const allAgents = new Map();

  const registerAgent = (value) => {
    const agent = String(value || "").trim();
    if (!agent) return;
    const key = agent.toLowerCase();
    const existing = allAgents.get(key);
    if (!existing) {
      allAgents.set(key, agent);
      return;
    }

    const existingHasModel = /\([^)]+\)/.test(existing);
    const currentHasModel = /\([^)]+\)/.test(agent);
    if (currentHasModel && !existingHasModel) {
      allAgents.set(key, agent);
    }
  };

  for (const session of sessions) {
    if (isTransportSession(session)) continue;
    registerAgent(session?.agent);
  }

  for (const extra of extras) {
    registerAgent(extra);
  }

  return Array.from(allAgents.values()).sort((left, right) => left.localeCompare(right));
}

export function filterFeedEntries(entries = [], agentFilter = "") {
  const needle = String(agentFilter || "").trim().toLowerCase();
  if (!needle) return [...entries];

  return entries.filter((entry) => String(entry?.agent || "").toLowerCase().includes(needle));
}

export function canClaimTask(task, operator = "") {
  return normalizeTask(task).status === "pending" && String(operator || "").trim().length > 0;
}

export function canFinalizeTask(task, operator = "") {
  const normalized = normalizeTask(task);
  return normalized.status === "claimed" && sameAgent(normalized.claimedBy, operator);
}

export function canUnlockLock(lock, operator = "") {
  return Boolean(lock?.path) && sameAgent(lock?.agent, operator);
}

export function nextFeedAckId(entries = [], operator = "") {
  const operatorName = String(operator || "").trim();
  if (!operatorName) return "";

  const candidate = entries.find((entry) => entry?.id && !sameAgent(entry?.agent, operatorName));
  return candidate?.id || "";
}

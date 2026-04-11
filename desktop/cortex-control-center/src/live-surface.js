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

function stripAgentModel(agent) {
  return String(agent || "")
    .replace(/\s*\([^)]*\)\s*$/, "")
    .trim()
    .toLowerCase();
}

export function isTransportSession(session) {
  const baseAgent = stripAgentModel(session?.agent);
  if (baseAgent !== "mcp") return false;

  const description = String(session?.description || "").trim().toLowerCase();
  return !description || description.startsWith("mcp session");
}

export function buildKnownAgents(sessions = [], extras = []) {
  const allAgents = new Set();

  for (const session of sessions) {
    if (isTransportSession(session)) continue;
    const agent = String(session?.agent || "").trim();
    if (agent) allAgents.add(agent);
  }

  for (const extra of extras) {
    const agent = String(extra || "").trim();
    if (agent) allAgents.add(agent);
  }

  return Array.from(allAgents).sort((left, right) => left.localeCompare(right));
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

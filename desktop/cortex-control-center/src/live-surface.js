function normalizeTask(task) { return { ...task, status: { in_progress: "claimed", done: "completed" }[task?.status] || task?.status || "pending", };
}
function sameAgent(left, right) { const normalizedLeft = String(left || "")
      .trim()
      .toLowerCase(), normalizedRight = String(right || "")
      .trim()
      .toLowerCase();
  return normalizedLeft.length > 0 && normalizedLeft === normalizedRight;
}
function resolveAgentName(agent, knownAgents = []) { const trimmed = String(agent || "").trim();
  if (!trimmed) return "";
  const canonical = knownAgents.find((knownAgent) => sameAgent(knownAgent, trimmed));
  return canonical ? String(canonical).trim() : trimmed;
}
function stripAgentModel(agent) { return String(agent || "")
    .replace(/\s*\([^)]*\)\s*$/, "")
    .trim()
    .toLowerCase();
}
function isTransportSession(session) { return stripAgentModel(session?.agent) === "mcp";
}
function buildKnownAgents(sessions = [], extras = []) { const allAgents = new Map(), registerAgent = (value) => { const agent = String(value || "").trim();
      if (!agent) return;
      const key = agent.toLowerCase(), existing = allAgents.get(key);
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
  return needle
    ? entries.filter((entry) => String(entry?.agent || "")
          .toLowerCase()
          .includes(needle), )
    : [...entries];
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
  buildKnownAgents, canClaimTask, canFinalizeTask, canUnlockLock, filterFeedEntries, isTransportSession, nextFeedAckId, normalizeTask,
  resolveAgentName, sameAgent, };

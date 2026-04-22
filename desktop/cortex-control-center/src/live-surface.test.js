import { describe, expect, it } from "vitest";

import {
  buildKnownAgents,
  canClaimTask,
  canFinalizeTask,
  canUnlockLock,
  filterFeedEntries,
  isTransportSession,
  nextFeedAckId,
  normalizeTask,
  resolveAgentName,
  sameAgent,
} from "./live-surface.js";

describe("normalizeTask", () => {
  it("maps legacy in-progress status to claimed", () => {
    expect(normalizeTask({ taskId: "a", status: "in_progress" })).toEqual({
      taskId: "a",
      status: "claimed",
    });
  });

  it("maps legacy done status to completed", () => {
    expect(normalizeTask({ taskId: "a", status: "done" })).toEqual({
      taskId: "a",
      status: "completed",
    });
  });

  it("preserves modern task status values", () => {
    expect(normalizeTask({ taskId: "a", status: "pending" }).status).toBe("pending");
    expect(normalizeTask({ taskId: "b", status: "claimed" }).status).toBe("claimed");
    expect(normalizeTask({ taskId: "c", status: "completed" }).status).toBe("completed");
  });
});

describe("sameAgent", () => {
  it("matches names case-insensitively", () => {
    expect(sameAgent("Codex", "codex")).toBe(true);
  });

  it("rejects blank names", () => {
    expect(sameAgent("", "Codex")).toBe(false);
    expect(sameAgent("Codex", "")).toBe(false);
  });
});

describe("buildKnownAgents", () => {
  it("merges session agents and extras without duplicates", () => {
    expect(
      buildKnownAgents(
        [{ agent: "Codex" }, { agent: "Claude" }, { agent: "Codex" }],
        ["Factory Droid", "Claude"],
      ),
    ).toEqual(["Claude", "Codex", "Factory Droid"]);
  });

  it("deduplicates agent names case-insensitively", () => {
    expect(
      buildKnownAgents(
        [{ agent: "Codex" }],
        ["codex", "CLAUDE", "Claude"],
      ),
    ).toEqual(["CLAUDE", "Codex"]);
  });

  it("skips generic MCP transport sessions", () => {
    expect(
      buildKnownAgents(
        [
          { agent: "Codex" },
          { agent: "mcp", description: "MCP session" },
          { agent: "mcp (gpt-5.4)", description: "MCP session · gpt-5.4" },
        ],
        [],
      ),
    ).toEqual(["Codex"]);
  });
});

describe("isTransportSession", () => {
  it("flags the generic MCP proxy session", () => {
    expect(isTransportSession({ agent: "mcp", description: "MCP session" })).toBe(true);
    expect(isTransportSession({ agent: "mcp (gpt-5.4)", description: "MCP session · gpt-5.4" })).toBe(true);
    expect(isTransportSession({ agent: "mcp (gpt-5.4)", description: "MCP active session · gpt-5.4" })).toBe(true);
  });

  it("keeps named agent sessions visible", () => {
    expect(isTransportSession({ agent: "Codex", description: "MCP session · gpt-5.4" })).toBe(false);
  });
});

describe("filterFeedEntries", () => {
  const entries = [
    { id: "1", agent: "Codex" },
    { id: "2", agent: "Claude" },
    { id: "3", agent: "Factory Droid" },
  ];

  it("returns all entries when the filter is blank", () => {
    expect(filterFeedEntries(entries, "")).toEqual(entries);
  });

  it("filters entries by agent name", () => {
    expect(filterFeedEntries(entries, "cla")).toEqual([{ id: "2", agent: "Claude" }]);
  });
});

describe("task actions", () => {
  it("requires an operator to claim a pending task", () => {
    expect(canClaimTask({ taskId: "a", status: "pending" }, "Codex")).toBe(true);
    expect(canClaimTask({ taskId: "a", status: "pending" }, "")).toBe(false);
  });

  it("only allows the claiming operator to complete or abandon claimed tasks", () => {
    const task = { taskId: "a", status: "claimed", claimedBy: "Codex" };
    expect(canFinalizeTask(task, "Codex")).toBe(true);
    expect(canFinalizeTask(task, "Claude")).toBe(false);
  });
});

describe("lock actions", () => {
  it("only allows the lock holder to unlock a path", () => {
    const lock = { path: "src/App.jsx", agent: "Codex" };
    expect(canUnlockLock(lock, "Codex")).toBe(true);
    expect(canUnlockLock(lock, "Claude")).toBe(false);
  });
});

describe("nextFeedAckId", () => {
  it("acks the newest visible entry from another agent", () => {
    const entries = [
      { id: "feed-3", agent: "Claude" },
      { id: "feed-2", agent: "Codex" },
      { id: "feed-1", agent: "Factory Droid" },
    ];

    expect(nextFeedAckId(entries, "Codex")).toBe("feed-3");
  });

  it("returns blank when there is no operator or nothing to ack", () => {
    expect(nextFeedAckId([{ id: "feed-1", agent: "Codex" }], "Codex")).toBe("");
    expect(nextFeedAckId([{ id: "feed-1", agent: "Claude" }], "")).toBe("");
  });
});

describe("resolveAgentName", () => {
  it("returns the canonical known agent when casing differs", () => {
    expect(resolveAgentName("codex", ["Codex", "Claude"])).toBe("Codex");
  });

  it("falls back to the trimmed raw value when no known agent matches", () => {
    expect(resolveAgentName("Factory Droid", ["Codex", "Claude"])).toBe("Factory Droid");
  });
});

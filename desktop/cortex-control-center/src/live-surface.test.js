import { describe, expect, it } from "vitest";

import {
  buildKnownAgents,
  filterFeedEntries,
  isTransportSession,
  normalizeTask,
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

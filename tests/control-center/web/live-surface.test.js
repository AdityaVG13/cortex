import { describe, expect, it } from "vitest";
import {
  buildKnownAgents,
  canFinalizeTask,
  canUnlockLock,
  dedupeNormalizedSessions,
  nextFeedAckId,
  resolveAgentName,
  sameAgent,
} from "../../../desktop/cortex-control-center/src/live-surface.js";

describe("agent identity", () => {
  it("treats a trailing (model) suffix as the same agent", () => {
    expect(sameAgent("claude-code", "claude-code (opus)")).toBe(true);
    expect(sameAgent("Claude-Code (opus)", "claude-code")).toBe(true);
    expect(sameAgent("claude-code", "codex")).toBe(false);
  });

  it("lets the operator unlock a lock recorded with a model suffix", () => {
    expect(canUnlockLock({ path: "/repo", agent: "claude-code (opus)" }, "claude-code")).toBe(true);
    expect(canUnlockLock({ path: "/repo", agent: "peer" }, "claude-code")).toBe(false);
  });

  it("lets the operator finalize a task they claimed under a model suffix", () => {
    expect(
      canFinalizeTask({ status: "claimed", claimedBy: "claude-code (opus)" }, "claude-code"),
    ).toBe(true);
  });

  it("does not ack the operator's own model-suffixed feed row as a teammate", () => {
    expect(
      nextFeedAckId(
        [
          { id: "self", agent: "claude-code (opus)" },
          { id: "peer", agent: "peer-agent" },
        ],
        "claude-code",
      ),
    ).toBe("peer");
  });

  it("collapses model-suffixed aliases onto one known agent", () => {
    const agents = buildKnownAgents([{ agent: "claude-code" }], ["claude-code (opus)"]);
    expect(agents).toEqual(["claude-code (opus)"]);
    expect(resolveAgentName("claude-code", agents)).toBe("claude-code (opus)");
  });

  it("collapses hook and MCP sessions for the same operator into one row", () => {
    const rows = dedupeNormalizedSessions([
      { agent: "claude-code", lastHeartbeatMs: 20, sessionId: "mcp" },
      { agent: "claude-code (opus)", lastHeartbeatMs: 10, sessionId: "hook" },
      { agent: "droid (gpt-5)", lastHeartbeatMs: 5, sessionId: "droid-model" },
      { agent: "droid", lastHeartbeatMs: 4, sessionId: "droid-bare" },
    ]);
    expect(rows.map((row) => row.sessionId).sort()).toEqual(["droid-model", "hook"]);
  });
});

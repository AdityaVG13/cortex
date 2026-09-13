import { describe, it, expect } from "vitest";
import { brainHealthRows, nextCaptureAction, normalizeBrainHealth } from "../../../desktop/cortex-control-center/src/app/brain-health.js";

describe("brain health normalizer", () => {
  it("never shows a green light for unknown values", () => {
    const n = normalizeBrainHealth(null);
    expect(n.ackProfile.tone).toBe("idle");
    expect(n.lastVerifiedRestore.tone).toBe("warn");
    expect(n.reflex.tone).toBe("idle");
    expect(n.captureScope.tone).toBe("idle");
    const rows = brainHealthRows(n);
    expect(rows.find((r) => r[0] === "RESTORE")[1]).toBe("NEVER VERIFIED");
  });

  it("maps the daemon brain block to ack profile, receipts, debt, restore, heads, scope and reflex", () => {
    const n = normalizeBrainHealth({
      commit_durability: "power_loss_assumed",
      durability_profile: "durable",
      maintenance_debt: { pending_jobs: 2, failed_jobs: 0, projection_lag: 3, pressure: "soft" },
      projection_lag: 3,
      unresolved_heads: 1,
      open_contradictions: 0,
      last_verified_restore: { verified_at: "2026-09-04T00:00:00Z", ok: true },
      capture_policy: { global: "active", scopes: [{ scope: "/repo/a", state: "paused", reason: "review" }] },
      capture_receipts: { sources_owned: 12, sources_external_only: 1, sources_erased: 0, sources_unavailable: 0, hook_captures: 4 },
      reflex: { state: "expired", generation: 3, records: 120 },
    });
    expect(n.ackProfile).toEqual({ label: "power_loss_assumed", tone: "ok", profile: "durable" });
    expect(n.debt).toEqual({ pending: 2, failed: 0, projectionLag: 3, pressure: "soft", tone: "warn" });
    expect(n.unresolvedHeads).toEqual({ count: 1, tone: "warn" });
    expect(n.captureScope.pausedScopes).toEqual([{ scope: "/repo/a", state: "paused", reason: "review" }]);
    expect(n.reflex).toEqual({ state: "expired", generation: 3, records: 120, tone: "warn" });
    const rows = Object.fromEntries(brainHealthRows(n).map(([k, v, t]) => [k, [v, t]]));
    expect(rows.ACK).toEqual(["POWER LOSS ASSUMED", "ok"]);
    expect(rows.CAPTURE).toEqual(["12 OWNED / 4 HOOK", "ok"]);
    expect(rows.DEBT).toEqual(["LAG 3", "warn"]);
    expect(rows.RESTORE).toEqual(["VERIFIED", "ok"]);
    expect(rows.HEADS).toEqual(["1 UNRESOLVED", "warn"]);
    expect(rows.SCOPE).toEqual(["ACTIVE (+1)", "ok"]);
    expect(rows.REFLEX).toEqual(["EXPIRED", "warn"]);
  });

  it("schema pending collapses to one honest row and capture actions cycle", () => {
    expect(brainHealthRows(normalizeBrainHealth({ status: "schema_pending" }))).toEqual([["BRAIN", "SCHEMA PENDING", "idle"]]);
    expect(nextCaptureAction("active").state).toBe("paused");
    expect(nextCaptureAction("paused").state).toBe("stopped");
    expect(nextCaptureAction("stopped").state).toBe("active");
    expect(normalizeBrainHealth({ capture_policy: { global: "stopped" } }).captureScope.tone).toBe("bad");
  });

  it("treats a restore report as verified only when verified/ok is true", () => {
    const failed = normalizeBrainHealth({
      last_verified_restore: { verified_at: "2026-09-04T00:00:00Z", verified: false, integrity_ok: false },
    });
    expect(failed.lastVerifiedRestore.ok).toBe(false);
    expect(failed.lastVerifiedRestore.tone).toBe("bad");
    expect(brainHealthRows(failed).find((r) => r[0] === "RESTORE")).toEqual(["RESTORE", "FAILED", "bad"]);

    const passed = normalizeBrainHealth({
      last_verified_restore: { verified_at: "2026-09-04T00:00:00Z", verified: true },
    });
    expect(passed.lastVerifiedRestore.ok).toBe(true);
    expect(passed.lastVerifiedRestore.tone).toBe("ok");
    expect(brainHealthRows(passed).find((r) => r[0] === "RESTORE")).toEqual(["RESTORE", "VERIFIED", "ok"]);
  });
});

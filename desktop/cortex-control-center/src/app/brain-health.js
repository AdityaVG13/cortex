// Brain health = what the memory can still promise, not whether a process
// is up. Pure normalizer over the `/health` payload's `brain` block; every
// row is a claim with a tone so the panel never shows a green light for an
// unknown value.

const UNKNOWN = "unknown";

function pick(value, fallback = null) {
  return value === undefined ? fallback : value;
}

function toneForDurability(profile) {
  if (profile === "power_loss_assumed") return "ok";
  if (profile === "process_crash") return "warn";
  return "idle";
}

function normalizeBrainHealth(brain) {
  const b = brain && typeof brain === "object" ? brain : {};
  const debt = b.maintenance_debt && typeof b.maintenance_debt === "object" ? b.maintenance_debt : {};
  const receipts = b.capture_receipts && typeof b.capture_receipts === "object" ? b.capture_receipts : {};
  const policy = b.capture_policy && typeof b.capture_policy === "object" ? b.capture_policy : {};
  const reflex = b.reflex && typeof b.reflex === "object" ? b.reflex : {};
  const restore = b.last_verified_restore && typeof b.last_verified_restore === "object" ? b.last_verified_restore : null;
  const schemaPending = b.status === "schema_pending";
  const unresolvedHeads = Number(pick(b.unresolved_heads, 0)) || 0;
  const contradictions = Number(pick(b.open_contradictions, 0)) || 0;
  const lag = Number(pick(b.projection_lag, pick(debt.projection_lag, 0))) || 0;
  const pressure = String(pick(debt.pressure, UNKNOWN));
  const captureState = String(pick(policy.global, UNKNOWN));
  const scopes = Array.isArray(policy.scopes) ? policy.scopes : [];
  const pausedScopes = scopes.filter((s) => s && s.state && s.state !== "active");
  return {
    schemaPending,
    ackProfile: { label: String(pick(b.commit_durability, UNKNOWN)), tone: schemaPending ? "idle" : toneForDurability(b.commit_durability), profile: String(pick(b.durability_profile, UNKNOWN)) },
    captureReceipts: {
      owned: Number(pick(receipts.sources_owned, 0)) || 0,
      externalOnly: Number(pick(receipts.sources_external_only, 0)) || 0,
      erased: Number(pick(receipts.sources_erased, 0)) || 0,
      unavailable: Number(pick(receipts.sources_unavailable, 0)) || 0,
      hookCaptures: Number(pick(receipts.hook_captures, 0)) || 0,
    },
    debt: { pending: Number(pick(debt.pending_jobs, 0)) || 0, failed: Number(pick(debt.failed_jobs, 0)) || 0, projectionLag: lag, pressure, tone: pressure === "hard" ? "bad" : pressure === "soft" ? "warn" : lag > 0 ? "warn" : "ok" },
    lastVerifiedRestore: restore ? { at: String(pick(restore.verified_at, pick(restore.at, UNKNOWN))), ok: restore.ok !== false, tone: restore.ok === false ? "bad" : "ok" } : { at: "never", ok: false, tone: "warn" },
    unresolvedHeads: { count: unresolvedHeads, tone: unresolvedHeads > 0 ? "warn" : "ok" },
    contradictions: { count: contradictions, tone: contradictions > 0 ? "warn" : "ok" },
    captureScope: { state: captureState, tone: captureState === "active" ? "ok" : captureState === "paused" ? "warn" : captureState === "stopped" ? "bad" : "idle", pausedScopes: pausedScopes.map((s) => ({ scope: String(s.scope), state: String(s.state), reason: s.reason ? String(s.reason) : "" })) },
    reflex: { state: String(pick(reflex.state, "unavailable")), generation: pick(reflex.generation, null), records: Number(pick(reflex.records, 0)) || 0, tone: reflex.state === "fresh" ? "ok" : reflex.state === "expired" ? "warn" : "idle" },
  };
}

/** Rows for a status strip: [label, value, tone]. */
function brainHealthRows(normalized) {
  const n = normalized || normalizeBrainHealth(null);
  if (n.schemaPending) return [["BRAIN", "SCHEMA PENDING", "idle"]];
  return [
    ["ACK", n.ackProfile.label.toUpperCase().replace(/_/g, " "), n.ackProfile.tone],
    ["CAPTURE", `${n.captureReceipts.owned} OWNED / ${n.captureReceipts.hookCaptures} HOOK`, n.captureReceipts.unavailable > 0 ? "warn" : "ok"],
    ["DEBT", n.debt.projectionLag > 0 ? `LAG ${n.debt.projectionLag}` : n.debt.pending > 0 ? `${n.debt.pending} PENDING` : "CLEAR", n.debt.tone],
    ["RESTORE", n.lastVerifiedRestore.at === "never" ? "NEVER VERIFIED" : "VERIFIED", n.lastVerifiedRestore.tone],
    ["HEADS", n.unresolvedHeads.count > 0 ? `${n.unresolvedHeads.count} UNRESOLVED` : "RESOLVED", n.unresolvedHeads.tone],
    ["SCOPE", n.captureScope.state.toUpperCase() + (n.captureScope.pausedScopes.length ? ` (+${n.captureScope.pausedScopes.length})` : ""), n.captureScope.tone],
    ["REFLEX", n.reflex.state.toUpperCase(), n.reflex.tone],
  ];
}

/** The next capture-policy action an operator can take from the strip. */
function nextCaptureAction(state) {
  if (state === "active") return { state: "paused", label: "Pause capture" };
  if (state === "paused") return { state: "stopped", label: "Stop capture" };
  return { state: "active", label: "Resume capture" };
}

export { brainHealthRows, nextCaptureAction, normalizeBrainHealth };

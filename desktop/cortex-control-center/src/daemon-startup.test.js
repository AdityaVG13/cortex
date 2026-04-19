import { describe, expect, it } from "vitest";

import {
  computeStartupRetryStep,
  daemonStatusPill,
  daemonSystemStatus,
  daemonUtilityPill,
  isDaemonStartingState,
  isTransientDaemonFeedback,
} from "./daemon-startup.js";

describe("daemon startup state helpers", () => {
  it("classifies running-but-unreachable as starting", () => {
    const daemonState = {
      running: true,
      reachable: false,
    };

    expect(isDaemonStartingState(daemonState)).toBe(true);
    expect(daemonStatusPill(daemonState)).toEqual({
      className: "pill starting",
      label: "Starting",
    });
    expect(daemonUtilityPill(daemonState)).toEqual({
      className: "starting",
      label: "Boot",
    });
    expect(daemonSystemStatus(daemonState)).toEqual({
      toneClass: "sys-warn",
      daemonLabel: "STARTING",
      embeddingsLabel: "WARMING",
    });
  });

  it("keeps reachable daemon states online", () => {
    const daemonState = {
      running: true,
      reachable: true,
    };

    expect(isDaemonStartingState(daemonState)).toBe(false);
    expect(daemonStatusPill(daemonState)).toEqual({
      className: "pill online",
      label: "Online",
    });
    expect(daemonUtilityPill(daemonState)).toEqual({
      className: "online",
      label: "Live",
    });
    expect(daemonSystemStatus(daemonState)).toEqual({
      toneClass: "sys-ok",
      daemonLabel: "RUNNING",
      embeddingsLabel: "ONNX ACTIVE",
    });
  });

  it("keeps stopped daemon states offline", () => {
    const daemonState = {
      running: false,
      reachable: false,
    };

    expect(isDaemonStartingState(daemonState)).toBe(false);
    expect(daemonStatusPill(daemonState)).toEqual({
      className: "pill offline",
      label: "Offline",
    });
    expect(daemonUtilityPill(daemonState)).toEqual({
      className: "offline",
      label: "Wait",
    });
    expect(daemonSystemStatus(daemonState)).toEqual({
      toneClass: "sys-err",
      daemonLabel: "OFFLINE",
      embeddingsLabel: "OFFLINE",
    });
  });
});

describe("computeStartupRetryStep", () => {
  it("seeds startup window and backoff from an empty state", () => {
    const step = computeStartupRetryStep({}, 1000);
    expect(step).toEqual({
      startedAtMs: 1000,
      attempts: 1,
      elapsedMs: 0,
      exhausted: false,
      nextDelayMs: 750,
    });
  });

  it("backs off retries and eventually exhausts by attempt budget", () => {
    const step = computeStartupRetryStep(
      { startedAtMs: 1000, attempts: 7 },
      5000,
      { maxAttempts: 8, maxWindowMs: 600000 }
    );

    expect(step.attempts).toBe(8);
    expect(step.elapsedMs).toBe(4000);
    expect(step.exhausted).toBe(true);
    expect(step.nextDelayMs).toBeGreaterThanOrEqual(1500);
  });

  it("exhausts by elapsed startup window even with low attempts", () => {
    const step = computeStartupRetryStep(
      { startedAtMs: 1000, attempts: 2 },
      7000,
      { maxAttempts: 50, maxWindowMs: 5000 }
    );

    expect(step.attempts).toBe(3);
    expect(step.elapsedMs).toBe(6000);
    expect(step.exhausted).toBe(true);
  });

  it("uses a bounded default startup window to avoid long stalls", () => {
    const step = computeStartupRetryStep(
      { startedAtMs: 1000, attempts: 5 },
      47000
    );

    expect(step.elapsedMs).toBe(46000);
    expect(step.exhausted).toBe(true);
  });
});

describe("isTransientDaemonFeedback", () => {
  it("treats startup and warmup notices as transient", () => {
    expect(isTransientDaemonFeedback("Daemon is still starting. Reconnect will continue automatically.")).toBe(true);
    expect(isTransientDaemonFeedback("Daemon startup timed out after 46s. Check Cortex logs, then restart from Control Center.")).toBe(true);
    expect(isTransientDaemonFeedback("Waiting for daemon auth token to finish rotating...")).toBe(true);
    expect(isTransientDaemonFeedback("Daemon is reachable but still warming up. Retrying shortly...")).toBe(true);
  });

  it("keeps durable operator messages intact", () => {
    expect(isTransientDaemonFeedback("Connected (core ready).")).toBe(false);
    expect(isTransientDaemonFeedback("Restart failed: Existing daemon did not stop cleanly.")).toBe(false);
  });
});

import { describe, expect, it } from "vitest";

import {
  CONTROL_CENTER_SETTINGS_STORAGE_KEY,
  DEFAULT_CONTROL_CENTER_SETTINGS,
  normalizeControlCenterSettings,
  readControlCenterSettings,
  resolveEffectiveReducedMotion,
  summarizeBudgetStatus,
  writeControlCenterSettings,
} from "./settings-state.js";

function memoryStorage(initial = {}) {
  const entries = new Map(Object.entries(initial));
  return {
    getItem: (key) => entries.get(key) ?? null,
    setItem: (key, value) => entries.set(key, String(value)),
  };
}

describe("control center settings state", () => {
  it("normalizes invalid persisted settings to defaults", () => {
    expect(
      normalizeControlCenterSettings({
        reducedMotion: "fast",
        highContrast: 1,
        compactNavigation: 1,
      }),
    ).toEqual({
      ...DEFAULT_CONTROL_CENTER_SETTINGS,
      highContrast: true,
      compactNavigation: true,
    });
  });

  it("round-trips settings through storage", () => {
    const storage = memoryStorage();
    const settings = {
      reducedMotion: "reduce",
      highContrast: true,
      keyboardHints: false,
      compactNavigation: true,
    };

    writeControlCenterSettings(settings, storage);

    expect(readControlCenterSettings(storage)).toEqual(settings);
  });

  it("resolves effective reduced motion from settings and OS preference", () => {
    expect(resolveEffectiveReducedMotion("system", true)).toBe(true);
    expect(resolveEffectiveReducedMotion("system", false)).toBe(false);
    expect(resolveEffectiveReducedMotion("reduce", false)).toBe(true);
    expect(resolveEffectiveReducedMotion("full", true)).toBe(false);
    expect(resolveEffectiveReducedMotion("unexpected", true)).toBe(true);
  });
});

describe("summarizeBudgetStatus", () => {
  it("treats missing budgets as unlimited", () => {
    expect(summarizeBudgetStatus(null)).toMatchObject({
      configLoaded: false,
      enabled: false,
      statusLabel: "Unlimited",
      endpointRows: [],
    });
  });

  it("maps health budget fields into rows", () => {
    const summary = summarizeBudgetStatus({
      configLoaded: true,
      enabled: true,
      source: "~/.cortex/budgets.toml",
      endpoints: {
        recall: { limit: 300, window_seconds: 60 },
        store: { limit: 120, windowSeconds: 60 },
      },
      recent_denials: { recall: 2 },
    });

    expect(summary.statusLabel).toBe("Enforced");
    expect(summary.endpointRows).toEqual([
      { endpoint: "recall", limit: 300, windowSeconds: 60 },
      { endpoint: "store", limit: 120, windowSeconds: 60 },
    ]);
    expect(summary.denialRows).toEqual([{ endpoint: "recall", count: 2 }]);
    expect(summary.recentDenialsTotal).toBe(2);
  });

  it("maps aggregate daemon health denials", () => {
    expect(
      summarizeBudgetStatus({
        configLoaded: true,
        enabled: true,
        recentDenials: 5,
      }),
    ).toMatchObject({
      denialRows: [],
      recentDenialsTotal: 5,
    });
  });

  it("surfaces invalid config state", () => {
    expect(
      summarizeBudgetStatus({
        config_loaded: true,
        enabled: false,
        error: "unknown endpoint",
      }),
    ).toMatchObject({
      statusLabel: "Invalid",
      error: "unknown endpoint",
    });
  });

  it("uses the documented storage key", () => {
    expect(CONTROL_CENTER_SETTINGS_STORAGE_KEY).toBe(
      "cortex_control_center_settings",
    );
  });
});

export const CONTROL_CENTER_SETTINGS_STORAGE_KEY = "cortex_control_center_settings";

export const DEFAULT_CONTROL_CENTER_SETTINGS = Object.freeze({
  reducedMotion: "system",
  highContrast: false,
  keyboardHints: true,
  compactNavigation: false,
});

const REDUCED_MOTION_VALUES = new Set(["system", "reduce", "full"]);

export function normalizeControlCenterSettings(value = {}) {
  const raw = value && typeof value === "object" ? value : {};
  const reducedMotion = REDUCED_MOTION_VALUES.has(raw.reducedMotion)
    ? raw.reducedMotion
    : DEFAULT_CONTROL_CENTER_SETTINGS.reducedMotion;
  return {
    reducedMotion,
    highContrast: Boolean(raw.highContrast),
    keyboardHints:
      typeof raw.keyboardHints === "boolean"
        ? raw.keyboardHints
        : DEFAULT_CONTROL_CENTER_SETTINGS.keyboardHints,
    compactNavigation: Boolean(raw.compactNavigation),
  };
}

export function readControlCenterSettings(storage = globalThis?.localStorage) {
  if (!storage) return DEFAULT_CONTROL_CENTER_SETTINGS;
  try {
    const raw = storage.getItem(CONTROL_CENTER_SETTINGS_STORAGE_KEY);
    return raw
      ? normalizeControlCenterSettings(JSON.parse(raw))
      : DEFAULT_CONTROL_CENTER_SETTINGS;
  } catch {
    return DEFAULT_CONTROL_CENTER_SETTINGS;
  }
}

export function writeControlCenterSettings(
  settings,
  storage = globalThis?.localStorage,
) {
  if (!storage) return;
  const normalized = normalizeControlCenterSettings(settings);
  storage.setItem(CONTROL_CENTER_SETTINGS_STORAGE_KEY, JSON.stringify(normalized));
}

function readBool(value, fallback = false) {
  return typeof value === "boolean" ? value : fallback;
}

function readNumber(value) {
  return Number.isFinite(Number(value)) ? Number(value) : null;
}

function readDenials(value) {
  if (value && typeof value === "object") {
    const rows = Object.entries(value).map(([endpoint, count]) => ({
      endpoint,
      count: readNumber(count) ?? 0,
    }));
    return {
      rows,
      total: rows.reduce((sum, row) => sum + row.count, 0),
    };
  }

  const total = readNumber(value) ?? 0;
  return {
    rows: [],
    total,
  };
}

export function summarizeBudgetStatus(budgets) {
  if (!budgets || typeof budgets !== "object") {
    return {
      configLoaded: false,
      enabled: false,
      source: "",
      error: "",
      statusLabel: "Unlimited",
      endpointRows: [],
      denialRows: [],
      recentDenialsTotal: 0,
    };
  }

  const configLoaded = readBool(
    budgets.configLoaded ?? budgets.config_loaded,
    false,
  );
  const enabled = readBool(budgets.enabled, false);
  const error = String(budgets.error || "");
  const source = String(budgets.source || "");
  const endpoints = budgets.endpoints && typeof budgets.endpoints === "object"
    ? budgets.endpoints
    : {};
  const recentDenials = readDenials(
    budgets.recentDenials ?? budgets.recent_denials,
  );

  const endpointRows = Object.entries(endpoints).map(([endpoint, config]) => {
    const row = config && typeof config === "object" ? config : {};
    return {
      endpoint,
      limit: readNumber(row.limit),
      windowSeconds: readNumber(row.windowSeconds ?? row.window_seconds),
    };
  });

  const statusLabel = error
    ? "Invalid"
    : !configLoaded
      ? "Unlimited"
      : enabled
        ? "Enforced"
        : "Disabled";

  return {
    configLoaded,
    enabled,
    source,
    error,
    statusLabel,
    endpointRows,
    denialRows: recentDenials.rows,
    recentDenialsTotal: recentDenials.total,
  };
}

export const CONTROL_CENTER_SETTINGS_STORAGE_KEY = "cortex_control_center_settings";

export const DEFAULT_CONTROL_CENTER_SETTINGS = Object.freeze({
  reducedMotion: "system",
  highContrast: false,
  keyboardHints: true,
  compactNavigation: false,
});

const REDUCED_MOTION_VALUES = new Set(["system", "reduce", "full"]);

export const BUDGET_ENDPOINT_DEFINITIONS = Object.freeze([
  { key: "store", label: "Store", defaultLimit: 120, defaultWindowSeconds: 60 },
  { key: "recall", label: "Recall", defaultLimit: 300, defaultWindowSeconds: 60 },
  { key: "boot", label: "Boot", defaultLimit: 60, defaultWindowSeconds: 60 },
  { key: "mcp", label: "MCP", defaultLimit: 240, defaultWindowSeconds: 60 },
]);

export function resolveEffectiveReducedMotion(setting = "system", osPrefersReducedMotion = false) {
  const reducedMotion = REDUCED_MOTION_VALUES.has(setting)
    ? setting
    : DEFAULT_CONTROL_CENTER_SETTINGS.reducedMotion;
  if (reducedMotion === "reduce") return true;
  if (reducedMotion === "full") return false;
  return Boolean(osPrefersReducedMotion);
}

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

function readErrorMessage(value) {
  if (!value) return "";
  if (typeof value === "string") return value;
  if (typeof value === "object") {
    return String(value.message || value.code || "");
  }
  return String(value);
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
  const error = readErrorMessage(budgets.error);
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

function defaultBudgetEndpointDraft(definition) {
  return {
    enabled: false,
    limit: definition.defaultLimit,
    windowSeconds: definition.defaultWindowSeconds,
  };
}

export function createBudgetDraftFromStatus(budgets) {
  const summary = summarizeBudgetStatus(budgets);
  const byEndpoint = new Map(summary.endpointRows.map((row) => [row.endpoint, row]));
  const endpoints = Object.fromEntries(
    BUDGET_ENDPOINT_DEFINITIONS.map((definition) => {
      const row = byEndpoint.get(definition.key);
      return [
        definition.key,
        row
          ? {
              enabled: true,
              limit: row.limit ?? definition.defaultLimit,
              windowSeconds: row.windowSeconds ?? definition.defaultWindowSeconds,
            }
          : defaultBudgetEndpointDraft(definition),
      ];
    }),
  );

  return {
    enabled: summary.configLoaded ? summary.enabled : false,
    endpoints,
  };
}

export function normalizeBudgetDraft(value = {}) {
  const raw = value && typeof value === "object" ? value : {};
  const rawEndpoints = raw.endpoints && typeof raw.endpoints === "object"
    ? raw.endpoints
    : {};
  const endpoints = Object.fromEntries(
    BUDGET_ENDPOINT_DEFINITIONS.map((definition) => {
      const endpoint = rawEndpoints[definition.key] || {};
      return [
        definition.key,
        {
          enabled: Boolean(endpoint.enabled),
          limit: endpoint.limit ?? definition.defaultLimit,
          windowSeconds: endpoint.windowSeconds ?? definition.defaultWindowSeconds,
        },
      ];
    }),
  );

  return {
    enabled: Boolean(raw.enabled),
    endpoints,
  };
}

function isPositiveInteger(value) {
  const number = Number(value);
  return Number.isInteger(number) && number > 0;
}

export function validateBudgetDraft(value) {
  const draft = normalizeBudgetDraft(value);
  for (const definition of BUDGET_ENDPOINT_DEFINITIONS) {
    const endpoint = draft.endpoints[definition.key];
    if (!endpoint.enabled) continue;
    if (!isPositiveInteger(endpoint.limit)) {
      return `${definition.label} limit must be a positive integer.`;
    }
    if (!isPositiveInteger(endpoint.windowSeconds)) {
      return `${definition.label} window must be a positive integer.`;
    }
  }
  return "";
}

export function serializeBudgetDraftForSave(value) {
  const draft = normalizeBudgetDraft(value);
  return {
    enabled: draft.enabled,
    endpoints: BUDGET_ENDPOINT_DEFINITIONS
      .map((definition) => {
        const endpoint = draft.endpoints[definition.key];
        return {
          endpoint: definition.key,
          enabled: endpoint.enabled,
          limit: Number(endpoint.limit),
          windowSeconds: Number(endpoint.windowSeconds),
        };
      })
      .filter((endpoint) => endpoint.enabled),
  };
}

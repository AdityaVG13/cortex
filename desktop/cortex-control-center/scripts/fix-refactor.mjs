#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "src");

function read(p) {
  return fs.readFileSync(path.join(SRC, p), "utf8");
}

function write(p, content) {
  fs.writeFileSync(path.join(SRC, p), content.endsWith("\n") ? content : `${content}\n`);
}

function exportConsts(filePath) {
  let content = read(filePath);
  content = content.replace(/^\/\/ Matches daemon-rs.*\n\/\/ Matches daemon-rs.*\n/m, "// Matches daemon-rs/src/main.rs:DEFAULT_CORTEX_PORT. Bump both simultaneously.\n");
  content = content
    .replace(/^const /gm, "export const ")
    .replace(/^function /gm, "export function ");
  write(filePath, content);
}

function exportFunctions(filePath) {
  let content = read(filePath);
  content = content.replace(/^function /gm, "export function ").replace(/^async function /gm, "export async function ");
  write(filePath, content);
}

exportConsts("app/constants.js");
exportFunctions("app/browser-bootstrap.js");
exportFunctions("app/utils/format.js");
exportFunctions("app/utils/daemon.js");
exportFunctions("app/utils/agent-color.js");
exportFunctions("app/components/sparkline-utils.js");

// conflicts.js - export functions but keep internal pickDefined
{
  let content = read("app/normalize/conflicts.js");
  content = content.replace(/^import.*\n\n/, "");
  content = content
    .replace(/^function pickDefined/gm, "function pickDefined")
    .replace(/^function normalizeConflict/gm, "export function normalizeConflict")
    .replace(/^function extractEntity/gm, "export function extractEntity")
    .replace(/^function formatConfidence/gm, "export function formatConfidence")
    .replace(/^function formatTrust/gm, "export function formatTrust")
    .replace(/^function formatTimestamp/gm, "export function formatTimestamp")
    .replace(/^function conflictBadge/gm, "export function conflictBadge")
    .replace(/^function isRouteMissing/gm, "export function isRouteMissing")
    .replace(/^function toFinite/gm, "export function toFinite")
    .replace(/^const CONFLICT_/gm, "export const CONFLICT_");
  // Fix normalizeConflictEntry etc
  content = content.replace(/^function normalizeConflictEntry/gm, "export function normalizeConflictEntry");
  content = content.replace(/^function normalizeConflictResolution/gm, "export function normalizeConflictResolution");
  content = content.replace(/^function normalizeConflictPair/gm, "export function normalizeConflictPair");
  content = content.replace(/^function normalizeConflictPairsPayload/gm, "export function normalizeConflictPairsPayload");
  content = content.replace(/^function normalizeConflictClassification/gm, "export function normalizeConflictClassification");
  content = content.replace(/^function normalizeConflictStatus/gm, "export function normalizeConflictStatus");
  write("app/normalize/conflicts.js", content);
}

exportFunctions("app/normalize/permissions.js");
exportFunctions("app/normalize/sessions.js");

// Bundle CSS for tests + backward compat
const cssFiles = [
  "styles/base.css",
  "styles/layout.css",
  "styles/components.css",
  "styles/topbar.css",
  "styles/animations.css",
  "styles/charts.css",
  "styles/panels/analytics.css",
  "styles/panels/coming-soon.css",
  "styles/panels/brain.css",
  "styles/overrides-2026.css",
  "styles/sidebar-collapse.css",
  "styles/connection-dialog.css",
  "styles/panels/conflicts.css",
  "styles/accessibility.css",
];
const bundled = cssFiles.map((f) => read(f)).join("\n");
write("styles.css", bundled);

// Panel-stage: full destructuring from hook return keys
const returnBlock = read("app/hooks/useDashboardHooks.js").match(/return \{([\s\S]*?)\n  \};\n\}/)?.[1] ?? "";
const keys = [...returnBlock.matchAll(/^\s{4}(\w+),?$/gm)].map((m) => m[1]);
const destructure = `export function PanelStage(p) {\n  const {\n    ${keys.join(",\n    ")},\n  } = p;\n`;

let panelStage = read("app/panels/panel-stage.jsx");
panelStage = panelStage.replace(
  /export function PanelStage\(props\) \{[\s\S]*?return \(/,
  `${destructure}\n  return (`,
);
write("app/panels/panel-stage.jsx", panelStage);

// Hook imports fix
{
  let hook = read("app/hooks/useDashboardHooks.js");
  hook = hook.replace(
    'import {\n  normalizeConflictPairsPayload,\n} from "../normalize/conflicts.js";',
    `import {
  isRouteMissingError,
  normalizeConflictPairsPayload,
} from "../normalize/conflicts.js";`,
  );
  hook = hook.replace(
    'import {\n  readBrowserBootstrap,',
    `import { priorityRank } from "../browser-bootstrap.js";\nimport {\n  readBrowserBootstrap,`,
  );
  // Add refreshAllRef to return if missing
  if (!hook.includes("refreshAllRef,")) {
    hook = hook.replace("    invokeRef,", "    invokeRef,\n    refreshAllRef,");
  }
  if (!hook.includes("setUpdateInstalling,")) {
    hook = hook.replace("    updateInstalling,", "    updateInstalling,\n    setUpdateInstalling,\n    setFeedbackMessage,");
  }
  write("app/hooks/useDashboardHooks.js", hook);
}

// browser-bootstrap: move normalizeCurrencyCode to format.js
{
  let bootstrap = read("app/browser-bootstrap.js");
  bootstrap = bootstrap.replace(/\nfunction normalizeCurrencyCode[\s\S]*?\n\}/, "");
  bootstrap = bootstrap.replace(/\nfunction priorityRank[\s\S]*?\n\}/, "");
  write("app/browser-bootstrap.js", bootstrap);

  let format = read("app/utils/format.js");
  if (!format.includes("normalizeCurrencyCode")) {
    format = `import { CURRENCY_OPTIONS } from "../constants.js";\n${format.replace(/^import[^\n]+\nimport[^\n]+\n\n/, "")}`;
    format = `${format.trim()}\n\nexport function normalizeCurrencyCode(raw) {\n  const candidate = String(raw || "").trim().toUpperCase();\n  return CURRENCY_OPTIONS.includes(candidate) ? candidate : "USD";\n}\n\nexport function priorityRank(priority) {\n  const map = { critical: 4, high: 3, medium: 2, low: 1 };\n  return map[priority] || 0;\n}\n`;
    write("app/utils/format.js", format);
  }

  let hook = read("app/hooks/useDashboardHooks.js");
  hook = hook.replace(
    'import { priorityRank } from "../browser-bootstrap.js";\n',
    "",
  );
  hook = hook.replace(
    'import { normalizeCurrencyCode, formatDaemonEndpoint, getOsReducedMotionPreference } from "../utils/format.js";',
    'import { normalizeCurrencyCode, formatDaemonEndpoint, getOsReducedMotionPreference, priorityRank } from "../utils/format.js";',
  );
  write("app/hooks/useDashboardHooks.js", hook);
}

console.log("Fixed exports, CSS bundle, panel destructuring");

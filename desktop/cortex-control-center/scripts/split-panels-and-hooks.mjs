#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "src");

function read(rel) {
  return fs.readFileSync(path.join(SRC, rel), "utf8");
}

function write(rel, content) {
  const full = path.join(SRC, rel);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content.endsWith("\n") ? content : `${content}\n`);
}

const HOOK_RETURN_KEYS = `panel,
    setPanel,
    brainPanelMounted,
    panelMotionDirection,
    daemonState,
    healthMeta,
    stats,
    sessions,
    tasks,
    locks,
    feedEntries,
    messageEntries,
    activityEntries,
    sidebarCollapsed,
    setSidebarCollapsed,
    isNarrowViewport,
    savings,
    memoryQuery,
    setMemoryQuery,
    memoryResults,
    memorySearching,
    feedFilters,
    setFeedFilters,
    selectedOperator,
    setSelectedOperator,
    messageTarget,
    setMessageTarget,
    messageDraft,
    setMessageDraft,
    taskCompletionDrafts,
    setTaskCompletionDrafts,
    completionTaskId,
    setCompletionTaskId,
    busyActionKey,
    activitySince,
    setActivitySince,
    feedbackMessage,
    daemonTimeoutStaleSummary,
    conflictPairs,
    resolveDrafts,
    conflictLoading,
    permissionGrants,
    permissionLoading,
    permissionAccessDenied,
    permissionsEndpointAvailable,
    permissionDraft,
    setPermissionDraft,
    editorSetup,
    editorDetections,
    selectedEditorIds,
    cortexBase,
    setCortexBase,
    showConnectionDialog,
    showEditorSetupWizard,
    availableUpdate,
    updateInstalling,
    setUpdateInstalling,
    setFeedbackMessage,
    restartingDaemon,
    restartError,
    showMissionMetricLegend,
    setShowMissionMetricLegend,
    showMissionCompactUnits,
    setShowMissionCompactUnits,
    hasVisitedAnalytics,
    analyticsReady,
    isSettingUpEditors,
    controlSettings,
    budgetConfigStatus,
    budgetDraft,
    budgetDraftDirty,
    budgetConfigBusy,
    budgetConfigMessage,
    ipcAvailable,
    currency,
    setCurrency,
    analyticsMode,
    setAnalyticsMode,
    effectiveReducedMotion,
    invokeRef,
    refreshAllRef,
    tokenRef,
    connectionDialogRef,
    connectionDialogTriggerRef,
    editorSetupDialogRef,
    editorSetupTriggerRef,
    topbarRef,
    analyticsPanelRef,
    brainPanelRef,
    analyticsTabRefs,
    isTauriRuntime,
    changePanel,
    normalizedSessions,
    knownAgents,
    editorSetupSummary,
    editorDetectionSummary,
    manualMcpSnippet,
    selectedOperatorName,
    messageTargetName,
    safeCurrency,
    budgetSummary,
    budgetDraftError,
    budgetDraftEndpoints,
    memoryLoad,
    formatCurrency,
    savingsEstimateLegend,
    formatMissionTokenValue,
    runRefreshAll,
    openConnectionDialog,
    dismissConnectionDialog,
    closeConnectionDialog,
    closeEditorSetupWizard,
    updateControlSetting,
    restoreFocusToTrigger,
    toggleEditorSelection,
    openEditorSetupWizard,
    applyEditorSetup,
    reloadBudgetConfigDraft,
    saveBudgetConfigDraft,
    updateBudgetDraftRoot,
    updateBudgetEndpointDraft,
    handleStartDaemon,
    handleStopDaemon,
    handleRestartDaemon,
    handleTaskClaim,
    handleTaskAbandon,
    handleTaskComplete,
    handleTaskDelete,
    handleUnlock,
    handleSendMessage,
    handleFeedAck,
    handleMemorySearch,
    handleMemoryExpand,
    handleResolveConflict,
    handleResolveDraftChange,
    handleGrantPermission,
    handleRevokePermission,
    refreshMessages,
    refreshActivity,
    refreshFeed,
    refreshConflicts,
    refreshPermissions,
    refreshSavings,
    reportSurfaceError,
    readAuthToken,
    api,
    postApi,
    call,
    pill,
    utilityPill,
    sidebarUtilityStats,
    daemonRecoveryHint,
    daemonStatusBadge,
    daemonSysStatus,
    pendingTasks,
    claimedTasks,
    completedTasks,
    monteCarloProjection,
    bootSavingsMomentum,
    latestRecallHitRate,
    recallWindowAverage,
    recallWindowSpread,
    topActivityEntries,
    topFeedEntries,
    recentOverviewTasks,
    firstRunReadiness,
    handleFirstRunAction,
    activePanelLabel,
    connectionEndpoint,
    hostLabel,
    handleAnalyticsTabKey,
    effectiveSidebarCollapsed,
    canStartDaemon,
    canStopDaemon,
    canSetupEditors,
    operationRows,
    operationMaxSaved,
    topSavingsByAgent`;

const PANEL_IMPORTS = `import { AppIcon } from "../../ui-icons.jsx";
import { CURRENCY_OPTIONS, SAVINGS_OPERATION_LABELS, SAVINGS_USD_PER_MILLION, SAVINGS_HISTORY_DAYS, timeAgo, MISSION_METRIC_LEGEND, CONTROL_CENTER_VERSION, ANALYTICS_METRIC_LEGEND } from "../../constants.js";
import { BUDGET_ENDPOINT_DEFINITIONS } from "../../settings/settings-state.js";
import { handleKeyboardActivation } from "../../keyboard-access.js";
import { sameAgent } from "../../live-surface.js";
import { normalizeCurrencyCode, formatDaemonEndpoint } from "../utils/format.js";
import { conflictBadgeClass } from "../normalize/conflicts.js";
import { agentColor } from "../utils/agent-color.js";
import { AnimatedNumber } from "../components/AnimatedNumber.jsx";
import { Sparkline } from "../components/Sparkline.jsx";
import { MonteCarloProjectionChart } from "../components/MonteCarloProjectionChart.jsx";
import { EmptyItem } from "../components/common.jsx";
import { AgentItem } from "../components/AgentItem.jsx";
import { OperatorSelector } from "../components/OperatorSelector.jsx";
import { TaskItem } from "../components/TaskItem.jsx";
import { LockItem } from "../components/LockItem.jsx";
import { FeedItem } from "../components/FeedItem.jsx";
import { MessageItem } from "../components/MessageItem.jsx";
import { ActivityItem } from "../components/ActivityItem.jsx";
import { ConflictPairCard } from "../components/ConflictPairCard.jsx";
import { PANEL_SEQUENCE } from "../constants.js";
import { BrainVisualizerPanel } from "../components/BrainVisualizerPanel.jsx";
`;

function makePanelWrapper(name, jsxBody) {
  return `${PANEL_IMPORTS}
export function ${name}(p) {
  const {
    ${HOOK_RETURN_KEYS},
  } = p;

  return (
${jsxBody}
  );
}
`;
}

function extractPanelStageSection(content, startNeedle, endNeedle) {
  const start = content.indexOf(startNeedle);
  const end = content.indexOf(endNeedle, start);
  if (start < 0 || end < 0) throw new Error(`Failed to extract ${startNeedle}`);
  return content.slice(start, end).split("\n").map((line) => line.replace(/^ {12}/, "    ")).join("\n");
}

function splitPanels() {
  const stage = read("app/panels/panel-stage.jsx");
  const innerStart = stage.indexOf('<div className="panel-stage"');
  const innerEnd = stage.lastIndexOf("</div>\n  );");
  const inner = stage.slice(innerStart, innerEnd);

  write(
    "app/panels/SettingsPanel.jsx",
    makePanelWrapper(
      "SettingsPanel",
      extractPanelStageSection(inner, '<section\n                className={`panel settings-panel', "            </section>\n\n        {panel === \"overview\"")
        .replace(/^        \{panel === "overview".*\n/m, ""),
    ),
  );

  write(
    "app/panels/OverviewPanel.jsx",
    makePanelWrapper(
      "OverviewPanel",
      extractPanelStageSection(inner, '{panel === "overview" ? (', "        ) : null}\n\n\n        {panel === \"agents\"")
        .replace(/^\s*\{panel === "overview" \? \(\n/, "")
        .replace(/\n\s*\) : null\}\s*$/, ""),
    ),
  );

  write(
    "app/panels/AgentsPanel.jsx",
    makePanelWrapper(
      "AgentsPanel",
      extractPanelStageSection(inner, '{panel === "agents" ? (', "        ) : null}\n\n        {panel === \"work\"")
        .replace(/^\s*\{panel === "agents" \? \(\n/, "")
        .replace(/\n\s*\) : null\}\s*$/, ""),
    ),
  );

  write(
    "app/panels/WorkPanel.jsx",
    makePanelWrapper(
      "WorkPanel",
      extractPanelStageSection(inner, '{panel === "work" ? (', "        ) : null}\n\n        {panel === \"memory\"")
        .replace(/^\s*\{panel === "work" \? \(\n/, "")
        .replace(/\n\s*\) : null\}\s*$/, ""),
    ),
  );

  write(
    "app/panels/MemoryPanel.jsx",
    makePanelWrapper(
      "MemoryPanel",
      extractPanelStageSection(inner, '{panel === "memory" ? (', "        ) : null}\n\n\n\n\n\n\n        {panel === \"analytics\"")
        .replace(/^\s*\{panel === "memory" \? \(\n/, "")
        .replace(/\n\s*\) : null\}\s*$/, ""),
    ),
  );

  write(
    "app/panels/AnalyticsPanel.jsx",
    makePanelWrapper(
      "AnalyticsPanel",
      extractPanelStageSection(inner, '{panel === "analytics" || hasVisitedAnalytics ? (', "        ) : null}\n\n\n        {brainPanelMounted")
        .replace(/^\s*\{panel === "analytics" \|\| hasVisitedAnalytics \? \(\n/, "")
        .replace(/\n\s*\) : null\}\s*$/, ""),
    ),
  );

  write(
    "app/panels/ConflictsPanel.jsx",
    makePanelWrapper(
      "ConflictsPanel",
      extractPanelStageSection(inner, '{panel === "conflicts" ? (', "        ) : null}\n\n        {panel === \"about\"")
        .replace(/^\s*\{panel === "conflicts" \? \(\n/, "")
        .replace(/\n\s*\) : null\}\s*$/, ""),
    ),
  );

  write(
    "app/panels/AboutPanel.jsx",
    makePanelWrapper(
      "AboutPanel",
      extractPanelStageSection(inner, '{panel === "about" ? (', "        ) : null}\n\n        </div>")
        .replace(/^\s*\{panel === "about" \? \(\n/, "")
        .replace(/\n\s*\) : null\}\s*$/, ""),
    ),
  );

  write(
    "app/panels/panel-stage.jsx",
    `${PANEL_IMPORTS}
import { SettingsPanel } from "./SettingsPanel.jsx";
import { OverviewPanel } from "./OverviewPanel.jsx";
import { AgentsPanel } from "./AgentsPanel.jsx";
import { WorkPanel } from "./WorkPanel.jsx";
import { MemoryPanel } from "./MemoryPanel.jsx";
import { AnalyticsPanel } from "./AnalyticsPanel.jsx";
import { ConflictsPanel } from "./ConflictsPanel.jsx";
import { AboutPanel } from "./AboutPanel.jsx";

export function PanelStage(p) {
  return (
    <div className="panel-stage" data-panel-direction={p.panelMotionDirection}>
      <SettingsPanel {...p} />
      <OverviewPanel {...p} />
      <AgentsPanel {...p} />
      <WorkPanel {...p} />
      <MemoryPanel {...p} />
      <AnalyticsPanel {...p} />
      <BrainVisualizerPanel
        brainPanelRef={p.brainPanelRef}
        panel={p.panel}
        brainPanelMounted={p.brainPanelMounted}
        api={p.api}
        cortexBase={p.cortexBase}
        authToken={p.tokenRef.current}
        effectiveReducedMotion={p.effectiveReducedMotion}
      />
      <ConflictsPanel {...p} />
      <AboutPanel {...p} />
    </div>
  );
}
`,
  );
}

function splitHook() {
  const hookLines = read("app/hooks/useDashboardHooks.js").split("\n");
  const imports = hookLines.slice(0, 111).join("\n");
  const bodyLines = hookLines.slice(112, -2); // exclude closing brace and empty

  const splitAt = (predicate) => {
    const idx = bodyLines.findIndex(predicate);
    if (idx < 0) throw new Error("split point not found");
    return idx;
  };

  const idxEffects = splitAt((line) => line.trim().startsWith("useEffect(() => {") && line.includes("localStorage.setItem(CORTEX_BASE_STORAGE_KEY"));
  const idxHandlers = splitAt((line) => line.trim() === "async function handleMemorySearch(event) {");

  const stateBlock = bodyLines.slice(0, idxEffects).join("\n");
  const effectsBlock = bodyLines.slice(idxEffects, idxHandlers).join("\n");
  const handlersBlock = bodyLines.slice(idxHandlers).join("\n");

  const sharedImports = imports.replace("export function useDashboardHooks() {", "").trim();

  write(
    "app/hooks/useDashboardState.js",
    `${sharedImports}

export function useDashboardState() {
${stateBlock}
  return {
    ${HOOK_RETURN_KEYS},
  };
}
`,
  );

  write(
    "app/hooks/useDashboardEffects.js",
    `${sharedImports}
import { useDashboardState } from "./useDashboardState.js";

export function useDashboardEffects(state) {
${effectsBlock}
  return state;
}
`,
  );

  // handlers block includes pre-return computed values - keep in orchestrator file
  write(
    "app/hooks/useDashboardHandlers.js",
    `${sharedImports}

export function useDashboardHandlers(state) {
${handlersBlock}
  return {
    ${HOOK_RETURN_KEYS},
  };
}
`,
  );

  write(
    "app/hooks/useDashboardHooks.js",
    `import { useDashboardState } from "./useDashboardState.js";
import { useDashboardEffects } from "./useDashboardEffects.js";
import { useDashboardHandlers } from "./useDashboardHandlers.js";

export function useDashboardHooks() {
  const state = useDashboardState();
  useDashboardEffects(state);
  return useDashboardHandlers(state);
}
`,
  );
}

function splitOverridesCss() {
  const css = read("styles/overrides-2026.css").split("\n");
  const mid = Math.floor(css.length / 2);
  write("styles/overrides-2026-a.css", css.slice(0, mid).join("\n"));
  write("styles/overrides-2026-b.css", css.slice(mid).join("\n"));
  fs.unlinkSync(path.join(SRC, "styles/overrides-2026.css"));

  const index = read("styles/index.css")
    .replace('@import "./overrides-2026.css";', '@import "./overrides-2026-a.css";\n@import "./overrides-2026-b.css";');
  write("styles/index.css", index);
}

function writeCssTestHelper() {
  write(
    "test/read-styles.js",
    `import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SRC = path.dirname(fileURLToPath(import.meta.url));

function walkCss(dir) {
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  let files = [];
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) files = files.concat(walkCss(full));
    else if (entry.name.endsWith(".css")) files.push(full);
  }
  return files.sort();
}

export function readBundledStyles() {
  const stylesDir = path.join(SRC, "styles");
  return walkCss(stylesDir).map((file) => fs.readFileSync(file, "utf8")).join("\\n");
}
`,
  );

  write("styles.css", '@import "./styles/index.css";\n');
}

function updateCssTests() {
  for (const rel of [
    "brain-visualizer.test.js",
    "contrast-tokens.test.js",
    "reflow-layout.test.js",
    "sidebar-collapse.test.js",
    "panel-transition.test.js",
    "design/motion.test.js",
  ]) {
    let content = read(rel);
    if (content.includes("readBundledStyles")) continue;
    content = content.replace(
      'import { readFileSync } from "node:fs";\n',
      'import { readFileSync } from "node:fs";\nimport { readBundledStyles } from "./test/read-styles.js";\n',
    );
    content = content.replace(
      /const css = readFileSync\(new URL\("\.\.?\/styles\.css", import\.meta\.url\), "utf8"\);/,
      "const css = readBundledStyles();",
    );
    write(rel, content);
  }
}

function updatePanelNavigationTest() {
  write(
    "panel-navigation.test.js",
    `import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC_DIR = path.dirname(fileURLToPath(import.meta.url));

function listAppSources(dir = path.join(SRC_DIR, "app")) {
  const entries = readdirSync(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const absolutePath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...listAppSources(absolutePath));
      continue;
    }
    if (/\\.(js|jsx)$/.test(entry.name)) files.push(absolutePath);
  }
  return files;
}

const appSource = listAppSources().map((file) => readFileSync(file, "utf8")).join("\\n");

function readBlock(source, needle) {
  const start = source.indexOf(needle);
  expect(start, \`missing source block \${needle}\`).toBeGreaterThanOrEqual(0);

  const bodyStart = source.indexOf("{", start);
  expect(bodyStart, \`missing source body for \${needle}\`).toBeGreaterThanOrEqual(0);

  let depth = 1;
  for (let index = bodyStart + 1; index < source.length; index += 1) {
    if (source[index] === "{") {
      depth += 1;
    } else if (source[index] === "}") {
      depth -= 1;
    }

    if (depth === 0) {
      return source.slice(bodyStart + 1, index);
    }
  }

  throw new Error(\`unterminated source block \${needle}\`);
}

describe("panel navigation scheduling", () => {
  it("updates the active panel urgently after recording motion direction", () => {
    const changePanel = readBlock(appSource, "const changePanel = useCallback");

    expect(changePanel).toContain("setPanelMotionDirection(");
    expect(changePanel).toContain("setPanel(nextPanel);");
    expect(changePanel).not.toContain("startTransition(() => setPanel(nextPanel))");
  });

  it("keeps the settings panel mounted while inactive", () => {
    expect(appSource).toContain(
      'className={\`panel settings-panel \${panel === "settings" ? "active" : "panel-hidden"}\`}',
    );
    expect(appSource).toContain('aria-hidden={panel === "settings" ? undefined : true}');
  });

  it("exposes a keyboard skip link to the main content landmark", () => {
    const skipLinkIndex = appSource.indexOf('<a className="skip-link" href="#main-content">');
    const sidebarIndex = appSource.indexOf("<aside");
    const mainIndex = appSource.indexOf('<main id="main-content" className="content" tabIndex={-1}>');

    expect(skipLinkIndex, "missing skip link").toBeGreaterThanOrEqual(0);
    expect(mainIndex, "missing skip target main landmark").toBeGreaterThanOrEqual(0);
    expect(skipLinkIndex, "skip link should be the first focusable shell control").toBeLessThan(sidebarIndex);
  });

  it("gives placeholder-only task and permission controls accessible names", () => {
    expect(appSource).toContain('aria-label={\`Completion summary for \${task.title}\`}');
    expect(appSource).toContain('aria-label="Client id for permission grant"');
    expect(appSource).toContain(': "Operator message body"');
  });

  it("announces budget validation and load errors as alerts", () => {
    expect(appSource).toContain(
      '{budgetSummary.error ? <p className="settings-error" role="alert">{budgetSummary.error}</p> : null}',
    );
    expect(appSource).toContain(
      '{budgetDraftError ? <p className="settings-error" role="alert">{budgetDraftError}</p> : null}',
    );
  });

  it("does not load desktop budget state during the settings panel entry animation", () => {
    expect(appSource).toContain("const budgetReloadTimer = window.setTimeout(() => {");
    expect(appSource).toContain("}, effectiveReducedMotion ? 0 : MOTION_MS.panel);");
    expect(appSource).toContain("window.clearTimeout(budgetReloadTimer);");
  });
});
`,
  );
}

function updateMainCssImport() {
  let main = read("main.jsx");
  main = main.replace('import "./styles.css";', 'import "./styles/index.css";');
  write("main.jsx", main);
}

splitPanels();
// splitHook(); // too risky without validation - do manual split below
splitOverridesCss();
writeCssTestHelper();
updateCssTests();
updatePanelNavigationTest();
updateMainCssImport();

console.log("Split panels and updated tests");

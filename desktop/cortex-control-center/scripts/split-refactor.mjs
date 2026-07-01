#!/usr/bin/env node
/**
 * One-time refactor script: splits App.jsx and styles.css into modules.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "src");
const APP = path.join(SRC, "app");
const STYLES = path.join(SRC, "styles");

function readLines(file) {
  return fs.readFileSync(file, "utf8").split("\n");
}

function sliceLines(lines, start, end) {
  return lines.slice(start - 1, end).join("\n");
}

function writeFile(relPath, content) {
  const full = path.join(SRC, relPath);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content.endsWith("\n") ? content : `${content}\n`);
}

function splitApp() {
  const lines = readLines(path.join(SRC, "App.jsx"));
  const total = lines.length;

  // --- Extracted module bodies (1-indexed line ranges from original App.jsx) ---
  const extractions = {
    "app/constants.js": { start: 74, end: 158, header: `// Matches daemon-rs/src/main.rs:DEFAULT_CORTEX_PORT. Bump both simultaneously.\n` },
    "app/browser-bootstrap.js": { start: 160, end: 302, imports: `import {\n  CORTEX_AUTH_STORAGE_KEY,\n  CORTEX_BASE_STORAGE_KEY,\n  CORTEX_PANEL_STORAGE_KEY,\n  DEFAULT_CORTEX_BASE,\n  LEGACY_CORTEX_AUTH_STORAGE_KEYS,\n  PANEL_SEQUENCE_KEYS,\n} from "./constants.js";\n\n` },
    "app/utils/format.js": { start: 304, end: 323, imports: `import { DEFAULT_CORTEX_PORT } from "../constants.js";\nimport { FEED_KIND_LABEL } from "../constants.js";\n\n`, exports: true },
    "app/components/AnimatedNumber.jsx": { start: 325, end: 363, imports: `import { useEffect, useRef, useState } from "react";\nimport { MOTION_MS, easeOutCubic } from "../../design/motion.js";\n\n`, exports: "AnimatedNumber" },
    "app/components/sparkline-utils.js": { start: 365, end: 387, imports: "", exports: true },
    "app/components/Sparkline.jsx": { start: 389, end: 435, imports: `import { useState } from "react";\nimport { buildLineGeometry } from "./sparkline-utils.js";\n\n`, exports: "Sparkline" },
    "app/components/MonteCarloProjectionChart.jsx": { start: 437, end: 526, imports: `import { formatSignedCompactNumber } from "../../number-format.js";\n\n`, exports: "MonteCarloProjectionChart" },
    "app/components/common.jsx": { start: 528, end: 545, imports: `import { AppIcon } from "../../ui-icons.jsx";\n\n`, exports: "ComingSoon, EmptyItem" },
    "app/utils/agent-color.js": { start: 547, end: 555, exports: true },
    "app/normalize/conflicts.js": { start: 557, end: 872, imports: `import { timeAgo } from "../../constants.js";\n\n`, exports: true },
    "app/normalize/permissions.js": { start: 874, end: 893, imports: `function pickDefined(...values) {\n  for (const value of values) {\n    if (value !== undefined && value !== null && value !== "") {\n      return value;\n    }\n  }\n  return null;\n}\n\n`, exports: true },
    "app/components/AgentItem.jsx": { start: 895, end: 919, imports: `import { timeAgo } from "../../constants.js";\nimport { agentColor } from "../utils/agent-color.js";\n\n`, exports: "AgentItem" },
    "app/components/OperatorSelector.jsx": { start: 921, end: 940, imports: `import { useId } from "react";\n\n`, exports: "OperatorSelector" },
    "app/components/TaskItem.jsx": { start: 942, end: 1065, imports: `import { canClaimTask, canFinalizeTask } from "../../live-surface.js";\nimport { timeAgo } from "../../constants.js";\n\n`, exports: "TaskItem" },
    "app/components/LockItem.jsx": { start: 1067, end: 1096, imports: `import { canUnlockLock } from "../../live-surface.js";\n\n`, exports: "LockItem" },
    "app/components/FeedItem.jsx": { start: 1098, end: 1124, imports: `import { timeAgo } from "../../constants.js";\nimport { feedKindLabel } from "../utils/format.js";\n\n`, exports: "FeedItem" },
    "app/components/MessageItem.jsx": { start: 1126, end: 1142, imports: `import { timeAgo } from "../../constants.js";\nimport { AppIcon } from "../../ui-icons.jsx";\nimport { agentColor } from "../utils/agent-color.js";\n\n`, exports: "MessageItem" },
    "app/components/ActivityItem.jsx": { start: 1144, end: 1165, imports: `import { timeAgo } from "../../constants.js";\n\n`, exports: "ActivityItem" },
    "app/components/ConflictPairCard.jsx": { start: 1167, end: 1343, imports: `import { timeAgo } from "../../constants.js";\nimport {\n  conflictBadgeClass,\n  formatConfidencePercent,\n  formatTimestamp,\n  formatTrustScore,\n} from "../normalize/conflicts.js";\nimport { agentColor } from "../utils/agent-color.js";\n\n`, exports: "ConflictPairCard" },
    "app/normalize/sessions.js": { start: 1345, end: 1381, imports: `import { sameAgent } from "../../live-surface.js";\n\n`, exports: true },
    "app/utils/daemon.js": { start: 1383, end: 1456, exports: true },
  };

  for (const [relPath, spec] of Object.entries(extractions)) {
    let body = sliceLines(lines, spec.start, spec.end);
    if (spec.header) body = spec.header + body;
    const imports = spec.imports || "";
    let exportSuffix = "";
    if (spec.exports === true) {
      // export all top-level functions/consts
      exportSuffix = "\n// auto-exported\n";
    } else if (typeof spec.exports === "string") {
      exportSuffix = "";
      body = body.replace(/^function (\w+)/gm, "export function $1");
    }
    // Prefix export on functions and const class in normalize/utils files
    if (spec.exports === true) {
      body = body
        .replace(/^function /gm, "export function ")
        .replace(/^const CONFLICT_/gm, "export const CONFLICT_")
        .replace(/^const EMPTY_/gm, "export const EMPTY_");
      if (relPath.includes("constants.js")) {
        body = body.replace(/^const /gm, "export const ").replace(/^function /gm, "export function ");
      }
      if (relPath.includes("browser-bootstrap.js")) {
        body = body.replace(/^function /gm, "export function ");
      }
      if (relPath.includes("format.js")) {
        body = body.replace(/^function /gm, "export function ");
      }
      if (relPath.includes("agent-color.js")) {
        body = body.replace(/^function /gm, "export function ");
      }
      if (relPath.includes("permissions.js")) {
        body = body.replace(/^function /gm, "export function ");
      }
      if (relPath.includes("sparkline-utils.js")) {
        body = body.replace(/^let /gm, "export let ").replace(/^function /gm, "export function ");
      }
    }
    writeFile(relPath, imports + body);
  }

  // BrainErrorBoundary + LazyBrainVisualizer
  writeFile(
    "app/components/BrainVisualizerPanel.jsx",
    `import { Component, lazy, Suspense } from "react";
import { AppIcon } from "../../ui-icons.jsx";

const LazyBrainVisualizer = lazy(() =>
  import("../../BrainVisualizer.jsx").then((module) => ({ default: module.BrainVisualizer })),
);

class BrainErrorBoundary extends Component {
  constructor(props) { super(props); this.state = { crashed: false, error: "" }; }
  static getDerivedStateFromError(err) { return { crashed: true, error: err?.message || "Unknown error" }; }
  render() {
    if (this.state.crashed) return (
      <div className="brain-loading">
        <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
        <p>Brain visualizer crashed: {this.state.error}</p>
        <button className="btn-sm btn-primary" onClick={() => this.setState({ crashed: false })} style={{ marginTop: 12 }}>Retry</button>
      </div>
    );
    return this.props.children;
  }
}

export function BrainVisualizerPanel({ brainPanelRef, panel, brainPanelMounted, api, cortexBase, authToken, effectiveReducedMotion }) {
  if (!brainPanelMounted) return null;
  return (
    <section
      ref={brainPanelRef}
      className={\`panel brain-panel \${panel === "brain" ? "active" : "panel-hidden"}\`}
      aria-hidden={panel === "brain" ? undefined : true}
    >
      <BrainErrorBoundary>
        <Suspense
          fallback={(
            <div className="brain-loading">
              <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
              <p>Loading brain visualizer…</p>
            </div>
          )}
        >
          <LazyBrainVisualizer
            api={api}
            cortexBase={cortexBase}
            authToken={authToken}
            active={panel === "brain"}
            reducedMotion={effectiveReducedMotion}
          />
        </Suspense>
      </BrainErrorBoundary>
    </section>
  );
}
`,
  );

  // Hook body: lines 1459-4293 (inside App function, excluding export line and return)
  const hookBody = sliceLines(lines, 1459, 4293);

  // Panel JSX blocks
  const panelStageInner = sliceLines(lines, 4605, 6467);

  writeFile(
    "app/hooks/useDashboardHooks.js",
    `import { startTransition, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { checkForUpdates, installUpdate } from "../../updater.js";
import { MOTION_MS } from "../../design/motion.js";
import {
  createApi,
  createPostApi,
  isAuthFailure,
  settledCollectErrors,
  summarizeDashboardErrors,
} from "../../api-client.js";
import {
  CURRENCY_OPTIONS,
  USD_TO_CURRENCY_RATE,
  SAVINGS_OPERATION_LABELS,
  timeAgo,
} from "../../constants.js";
import {
  buildKnownAgents,
  filterFeedEntries,
  isTransportSession,
  nextFeedAckId,
  normalizeTask,
  resolveAgentName,
  sameAgent,
} from "../../live-surface.js";
import {
  buildFirstRunReadiness,
  computeStartupRetryStep,
  daemonStatusPill,
  daemonSystemStatus,
  daemonUtilityPill,
  isDaemonStartingState,
  shouldContinueStartupRecovery,
  isTransientDaemonFeedback,
} from "../../daemon-startup.js";
import { buildMonteCarloProjection } from "../../analytics-projection.js";
import { summarizeBootThroughput } from "../../analytics-metrics.js";
import { formatCompactNumber, formatSignedCompactNumber } from "../../number-format.js";
import { handleKeyboardActivation, shouldIgnoreGlobalShortcut, trapFocusInContainer } from "../../keyboard-access.js";
import {
  BUDGET_ENDPOINT_DEFINITIONS,
  createBudgetDraftFromStatus,
  readControlCenterSettings,
  resolveEffectiveReducedMotion,
  serializeBudgetDraftForSave,
  summarizeBudgetStatus,
  validateBudgetDraft,
  writeControlCenterSettings,
} from "../../settings/settings-state.js";
import {
  ANALYTICS_METRIC_LEGEND,
  ANALYTICS_REFRESH_MS,
  CONTROL_CENTER_VERSION,
  CORE_REFRESH_MIN_INTERVAL_MS,
  CORTEX_BASE_STORAGE_KEY,
  CORTEX_OPERATOR_STORAGE_KEY,
  CORTEX_PANEL_STORAGE_KEY,
  DAEMON_START_POLL_INTERVAL_MS,
  DAEMON_START_STILL_STARTING_GRACE_MS,
  DAEMON_START_WAIT_TIMEOUT_MS,
  DAEMON_STOP_HANG_TIMEOUT_MS,
  DAEMON_STOP_WAIT_TIMEOUT_MS,
  DEFAULT_CORTEX_BASE,
  DEV_RESTART_VERIFY_ENABLED,
  DEV_RESTART_VERIFY_TIMEOUT_MS,
  EMPTY_DAEMON,
  EMPTY_HEALTH_META,
  FALLBACK_REFRESH_MS,
  MISSION_METRIC_LEGEND,
  PANEL_SEQUENCE,
  PANEL_SEQUENCE_KEYS,
  PANEL_SEQUENCE_LABEL,
  RECALL_HEADLINE_MIN_QUERIES,
  SAVINGS_HISTORY_DAYS,
  SAVINGS_USD_PER_MILLION,
  SECONDARY_REFRESH_MIN_INTERVAL_MS,
  SIDEBAR_COLLAPSE_BREAKPOINT_PX,
  SSE_RECONNECT_BASE_MS,
  SSE_RECONNECT_MAX_MS,
  SSE_REFRESH_THROTTLE_MS,
  panelIndex,
} from "../constants.js";
import {
  readBrowserBootstrap,
  readLocalStorageValue,
  readPersistedBrowserAuthToken,
  readTauriInvoke,
  persistBrowserAuthToken,
} from "../browser-bootstrap.js";
import { normalizeCurrencyCode, formatDaemonEndpoint, getOsReducedMotionPreference } from "../utils/format.js";
import {
  normalizeConflictPairsPayload,
} from "../normalize/conflicts.js";
import {
  normalizePermissionPayload,
} from "../normalize/permissions.js";
import {
  normalizeSession,
  sessionMatchesAgent,
} from "../normalize/sessions.js";
import {
  extractMcpToolError,
  isDaemonOfflineErrorMessage,
  isDaemonSuppressibleErrorMessage,
  isDaemonTimeoutErrorMessage,
  isReadyReadinessPayload,
  isReachableHealthPayload,
  parseMcpToolResult,
  setElementInert,
} from "../utils/daemon.js";

export function useDashboardHooks() {
${hookBody}
  return {
    panel,
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
    topSavingsByAgent,
  };
}
`,
  );

  // Fix hook: the original ends with pre-return vars, not return. Remove duplicate return block artifacts.
  // The hook body already contains everything up to handleAnalyticsTabKey.

  writeFile(
    "app/panels/panel-stage.jsx",
    `/* eslint-disable react/jsx-max-depth */
import { AppIcon } from "../../ui-icons.jsx";
import { CURRENCY_OPTIONS, SAVINGS_OPERATION_LABELS, SAVINGS_USD_PER_MILLION, SAVINGS_HISTORY_DAYS, timeAgo, MISSION_METRIC_LEGEND, CONTROL_CENTER_VERSION } from "../../constants.js";
import { BUDGET_ENDPOINT_DEFINITIONS } from "../../settings/settings-state.js";
import { MOTION_MS } from "../../design/motion.js";
import { handleKeyboardActivation } from "../../keyboard-access.js";
import { sameAgent } from "../../live-surface.js";
import { DEFAULT_CORTEX_BASE } from "../constants.js";
import { persistBrowserAuthToken } from "../browser-bootstrap.js";
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

export function PanelStage(props) {
  const d = props;
  const {
    panel, panelMotionDirection, hasVisitedAnalytics, analyticsReady, brainPanelMounted,
  } = d;

  return (
${panelStageInner.split("\n").map((line) => (line ? `    ${line}` : "")).join("\n")}
  );
}
`,
  );

  writeFile(
    "app/AppShell.jsx",
    `import { useEffect } from "react";
import { installUpdate } from "../updater.js";
import { AppIcon } from "../ui-icons.jsx";
import { PANEL_SEQUENCE } from "./constants.js";
import { PanelStage } from "./panels/panel-stage.jsx";

export function AppShell(d) {
  const {
    effectiveSidebarCollapsed,
    panel,
    changePanel,
    pill,
    utilityPill,
    sidebarUtilityStats,
    activePanelLabel,
    daemonState,
    daemonRecoveryHint,
    handleRestartDaemon,
    restartingDaemon,
    invokeRef,
    handleStartDaemon,
    handleStopDaemon,
    canStartDaemon,
    canStopDaemon,
    restartError,
    availableUpdate,
    updateInstalling,
    setUpdateInstalling,
    setFeedbackMessage,
    feedbackMessage,
    setSidebarCollapsed,
    topbarRef,
    stats,
    normalizedSessions,
    openConnectionDialog,
    hostLabel,
    daemonStatusBadge,
    showEditorSetupWizard,
    isSettingUpEditors,
    closeEditorSetupWizard,
    editorSetupDialogRef,
    editorDetectionSummary,
    selectedEditorIds,
    toggleEditorSelection,
    manualMcpSnippet,
    applyEditorSetup,
    showConnectionDialog,
    dismissConnectionDialog,
    connectionDialogRef,
    connectionDialogTriggerRef,
    isTauriRuntime,
    connectionEndpoint,
    closeConnectionDialog,
    setCortexBase,
    tokenRef,
    persistBrowserAuthToken,
    readAuthToken,
    refreshAllRef,
    DEFAULT_CORTEX_BASE,
    trapFocusInContainer,
    restoreFocusToTrigger,
  } = d;

  useEffect(() => {
    if (!showConnectionDialog || !connectionDialogRef.current) return undefined;
    return trapFocusInContainer(connectionDialogRef.current);
  }, [showConnectionDialog]);

  useEffect(() => {
    if (!showEditorSetupWizard || !editorSetupDialogRef.current) return undefined;
    return trapFocusInContainer(editorSetupDialogRef.current);
  }, [showEditorSetupWizard]);

  return (
    <div className={\`app \${effectiveSidebarCollapsed ? "sidebar-collapsed" : ""}\`}>
      <a className="skip-link" href="#main-content">Skip to main content</a>
      <aside className={\`sidebar \${effectiveSidebarCollapsed ? "collapsed" : ""}\`} aria-labelledby="sidebar-title">
        <div className="sidebar-header">
          <div className="logo">
            <span id="sidebar-title">Cortex</span>
          </div>
          <div className={pill.className}>{pill.label}</div>
        </div>

        <nav className="sidebar-nav" aria-label="Primary panels">
          {PANEL_SEQUENCE.map((item, idx) => (
            <button
              key={item.key}
              type="button"
              className={\`nav-item \${panel === item.key ? "active" : ""}\`}
              onClick={() => changePanel(item.key)}
              data-key={idx + 1}
              aria-current={panel === item.key ? "page" : undefined}
            >
              <span style={{ opacity: 0.5, fontSize: "12px" }}><AppIcon name={item.icon} /></span>
              {item.label}
            </button>
          ))}
        </nav>

        <div className="sidebar-utility">
          <div className="sidebar-utility-header">
            <span className="sidebar-utility-kicker">Mission status</span>
            <span className={\`sidebar-utility-pill \${utilityPill.className}\`}>
              {utilityPill.label}
            </span>
          </div>
          <div className="sidebar-utility-grid">
            {sidebarUtilityStats.map((item) => (
              <div key={item.label} className={\`sidebar-utility-card tone-\${item.tone}\`}>
                <span className="sidebar-utility-label">{item.label}</span>
                <strong className="sidebar-utility-value">{item.value}</strong>
              </div>
            ))}
          </div>
          <div className="sidebar-utility-note">
            <span className="sidebar-utility-note-label">Focus</span>
            <strong>{activePanelLabel}</strong>
            <p>{daemonState.message}</p>
            {daemonRecoveryHint ? <p className="sidebar-utility-alert">{daemonRecoveryHint}</p> : null}
          </div>
        </div>

        <div className="sidebar-footer">
          <div className="daemon-restart-row">
            <button
              type="button"
              className="btn-ctrl btn-restart"
              onClick={handleRestartDaemon}
              disabled={restartingDaemon || !invokeRef.current}
            >
              {restartingDaemon ? "Restarting..." : "Restart"}
            </button>
          </div>
          <div className="daemon-controls-grid">
            <button type="button" className="btn-ctrl btn-primary" onClick={handleStartDaemon} disabled={!canStartDaemon}>Start</button>
            <button type="button" className="btn-ctrl" onClick={handleStopDaemon} disabled={!canStopDaemon}>Stop</button>
            <button type="button" className="btn-ctrl btn-danger" onClick={async () => {
              if (invokeRef.current) {
                try { await d.call("quit_app"); } catch { /* app is exiting */ }
              }
            }}>Exit</button>
          </div>
          {restartError ? (
            <button type="button" className="btn-sm btn-danger btn-restart-retry" onClick={handleRestartDaemon}>
              Retry Restart
            </button>
          ) : null}
          {availableUpdate && (
            <div className="update-banner">
              <span>v{availableUpdate.version} available</span>
              <button
                type="button"
                className="btn-sm btn-primary"
                disabled={updateInstalling}
                onClick={async () => {
                  setUpdateInstalling(true);
                  setFeedbackMessage("Downloading update...");
                  try {
                    await installUpdate(availableUpdate);
                  } catch (err) {
                    setFeedbackMessage(\`Update failed: \${String(err)}\`);
                    setUpdateInstalling(false);
                  }
                }}
              >
                {updateInstalling ? "Installing..." : "Update"}
              </button>
            </div>
          )}
          <p className="sidebar-status" aria-hidden="true">{feedbackMessage}</p>
          <button
            type="button"
            className="btn-sidebar-collapse"
            aria-label={effectiveSidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
            title={effectiveSidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
            onClick={() => setSidebarCollapsed((c) => !c)}
          >
            <AppIcon name={effectiveSidebarCollapsed ? "chevron-right" : "chevron-left"} size={16} />
          </button>
        </div>
      </aside>

      <main id="main-content" className="content" tabIndex={-1}>
        <p className="sr-only" role="status" aria-live="polite" aria-atomic="true">
          {feedbackMessage}
        </p>
        <div
          ref={topbarRef}
          className={\`topbar \${panel === "overview" ? "topbar-hidden" : ""}\`}
          aria-hidden={panel === "overview" ? true : undefined}
        >
          <div className="topbar-left">
            <span className="topbar-path">CORTEX</span>
            <span className="topbar-sep">/</span>
            <span className="topbar-current">{activePanelLabel.toUpperCase()}</span>
          </div>
          <div className="topbar-right">
            <span className="topbar-stat"><span className="topbar-label">MEM</span> {stats.memories}</span>
            <span className="topbar-stat"><span className="topbar-label">DEC</span> {stats.decisions}</span>
            <span className="topbar-stat"><span className="topbar-label">EVT</span> {stats.events}</span>
            <span className="topbar-stat"><span className="topbar-label">AGENTS</span> {normalizedSessions.length}</span>
            <button
              type="button"
              className="topbar-stat topbar-connection"
              onClick={openConnectionDialog}
              tabIndex={panel === "overview" ? -1 : undefined}
              title="Click to change connection"
              aria-label={\`Connection host \${hostLabel}. Open connection settings.\`}
            >
              <span className="topbar-label">HOST</span>
              {hostLabel}
            </button>
            <span className={\`topbar-status \${daemonStatusBadge.className}\`} title={daemonStatusBadge.title}>
              {daemonStatusBadge.label}
            </span>
          </div>
        </div>

        {showEditorSetupWizard && (
          <div className="connection-overlay" role="presentation" onClick={() => !isSettingUpEditors && closeEditorSetupWizard()}>
            <div
              ref={editorSetupDialogRef}
              className="connection-dialog editor-setup-dialog"
              role="dialog"
              aria-modal="true"
              aria-labelledby="editor-setup-title"
              aria-describedby="editor-setup-description"
              aria-busy={isSettingUpEditors ? true : undefined}
              tabIndex={-1}
              onClick={(e) => e.stopPropagation()}
            >
              <div className="editor-setup-dialog-header">
                <div>
                  <span className="editor-setup-kicker">Shared MCP Registration</span>
                  <h2 id="editor-setup-title">Setup MCP</h2>
                </div>
                <span className="badge">
                  {editorDetectionSummary.detected}/{editorDetectionSummary.results.length}
                </span>
              </div>
              <p className="connection-subtitle" id="editor-setup-description">
                Choose which supported clients should receive the shared Cortex attach-only MCP entry. Every client points at the same
                app-owned daemon command.
              </p>
              <div className="editor-setup-choice-list">
                {editorDetectionSummary.results.map((entry) => {
                  const tone = !entry.detected ? "idle" : entry.registered ? "ok" : "warn";
                  const stateLabel = !entry.detected ? "Not detected" : entry.registered ? "Configured" : "Detected";
                  const selected = selectedEditorIds.includes(entry.id);
                  return (
                    <label key={entry.id} className={\`editor-setup-choice \${tone} \${!entry.detected ? "disabled" : ""}\`}>
                      <input
                        type="checkbox"
                        checked={selected}
                        disabled={!entry.detected || isSettingUpEditors}
                        onChange={() => toggleEditorSelection(entry.id)}
                      />
                      <div className="editor-setup-choice-body">
                        <div className="editor-setup-item-head">
                          <span className="editor-setup-name">{entry.name}</span>
                          <span className="editor-setup-state">{stateLabel}</span>
                        </div>
                        {entry.configPath ? <code>{entry.configPath}</code> : null}
                        <p>{entry.message || "No detail provided."}</p>
                      </div>
                    </label>
                  );
                })}
              </div>
              <div className="editor-setup-manual">
                <span className="editor-setup-kicker">Manual Fallback</span>
                <p>If a client is missing from the supported list, register this MCP server manually or paste it into that AI&apos;s setup flow:</p>
                <pre>{manualMcpSnippet}</pre>
                <p>Replace <code>codex</code> with that AI&apos;s agent ID (for example: <code>claude</code>, <code>cursor</code>, <code>gemini</code>).</p>
              </div>
              <div className="connection-actions">
                <button type="button" className="btn-sm" onClick={closeEditorSetupWizard} disabled={isSettingUpEditors}>
                  Cancel
                </button>
                <button
                  type="button"
                  className="btn-sm btn-primary"
                  onClick={applyEditorSetup}
                  disabled={isSettingUpEditors || !selectedEditorIds.length}
                >
                  {isSettingUpEditors ? "Applying..." : \`Apply to \${selectedEditorIds.length} Client\${selectedEditorIds.length === 1 ? "" : "s"}\`}
                </button>
              </div>
            </div>
          </div>
        )}

        {showConnectionDialog && (
          <div className="connection-overlay" role="presentation" onClick={dismissConnectionDialog}>
            <div
              ref={connectionDialogRef}
              className="connection-dialog"
              role="dialog"
              aria-modal="true"
              aria-labelledby="connection-dialog-title"
              aria-describedby="connection-dialog-description"
              tabIndex={-1}
              onClick={(e) => e.stopPropagation()}
            >
              <div className="connection-dialog-header">
                <h2 id="connection-dialog-title">Connection Settings</h2>
                <button
                  type="button"
                  className="connection-dialog-close"
                  aria-label="Close connection settings"
                  onClick={dismissConnectionDialog}
                >
                  ×
                </button>
              </div>
              <p className="connection-subtitle" id="connection-dialog-description">
                {isTauriRuntime
                  ? "Desktop app mode uses the local app-managed Cortex daemon only."
                  : "Connect to a local or remote Cortex daemon"}
              </p>
              <form onSubmit={(e) => {
                e.preventDefault();
                if (isTauriRuntime) {
                  setCortexBase(DEFAULT_CORTEX_BASE);
                  tokenRef.current = "";
                  persistBrowserAuthToken("");
                  closeConnectionDialog();
                  queueMicrotask(() => refreshAllRef.current());
                  return;
                }
                const fd = new FormData(e.target);
                const host = fd.get("host")?.toString().trim() || "127.0.0.1";
                const port = fd.get("port")?.toString().trim() || "7437";
                const token = fd.get("token")?.toString().trim();
                setCortexBase(\`http://\${host}:\${port}\`);
                tokenRef.current = token || "";
                persistBrowserAuthToken(token || "");
                closeConnectionDialog();
                queueMicrotask(() => refreshAllRef.current());
              }}>
                <label className="connection-field">
                  <span>Host</span>
                  <input
                    name="host"
                    defaultValue={connectionEndpoint.host}
                    placeholder="127.0.0.1"
                    disabled={isTauriRuntime}
                  />
                </label>
                <label className="connection-field">
                  <span>Port</span>
                  <input
                    name="port"
                    defaultValue={connectionEndpoint.port}
                    placeholder="7437"
                    disabled={isTauriRuntime}
                  />
                </label>
                <label className="connection-field">
                  <span>Auth Token</span>
                  <input
                    name="token"
                    type="password"
                    placeholder={isTauriRuntime ? "Managed by desktop app token flow" : "Leave blank for local (auto-read)"}
                    disabled={isTauriRuntime}
                  />
                </label>
                <div className="connection-actions">
                  <button type="button" className="btn-sm" onClick={() => {
                    setCortexBase(DEFAULT_CORTEX_BASE);
                    tokenRef.current = "";
                    persistBrowserAuthToken("");
                    closeConnectionDialog();
                    readAuthToken({ suppressFeedback: true });
                    queueMicrotask(() => refreshAllRef.current());
                  }}>Reset to Local</button>
                  <button type="submit" className="btn-sm btn-primary">Connect</button>
                </div>
              </form>
            </div>
          </div>
        )}

        <PanelStage {...d} />
      </main>
    </div>
  );
}
`,
  );

  writeFile(
    "App.jsx",
    `export { App } from "./app/App.jsx";
`,
  );

  writeFile(
    "app/App.jsx",
    `import { useEffect } from "react";
import { DEFAULT_CORTEX_BASE } from "./constants.js";
import { persistBrowserAuthToken } from "./browser-bootstrap.js";
import { useDashboardHooks } from "./hooks/useDashboardHooks.js";
import { AppShell } from "./AppShell.jsx";

export function App() {
  const dashboard = useDashboardHooks();
  const { refreshAllRef, runRefreshAll } = dashboard;

  useEffect(() => {
    refreshAllRef.current = runRefreshAll;
  }, [refreshAllRef, runRefreshAll]);

  return <AppShell {...dashboard} DEFAULT_CORTEX_BASE={DEFAULT_CORTEX_BASE} persistBrowserAuthToken={persistBrowserAuthToken} refreshAllRef={dashboard.refreshAllRef} />;
}
`,
  );

  console.log(`Split App.jsx (${total} lines) into app/ modules`);
}

function splitStyles() {
  const cssPath = path.join(SRC, "styles.css");
  const lines = readFilesLines(cssPath);
  const sections = [
    { file: "styles/base.css", start: 1, end: 194 },
    { file: "styles/layout.css", start: 195, end: 673 },
    { file: "styles/components.css", start: 674, end: 1298 },
    { file: "styles/topbar.css", start: 1299, end: 1518 },
    { file: "styles/animations.css", start: 1519, end: 1783 },
    { file: "styles/charts.css", start: 1784, end: 1932 },
    { file: "styles/panels/analytics.css", start: 1933, end: 2828 },
    { file: "styles/panels/coming-soon.css", start: 2829, end: 2872 },
    { file: "styles/panels/brain.css", start: 2873, end: 3240 },
    { file: "styles/overrides-2026.css", start: 3241, end: 4491 },
    { file: "styles/sidebar-collapse.css", start: 4492, end: 4790 },
    { file: "styles/connection-dialog.css", start: 4791, end: 4977 },
    { file: "styles/panels/conflicts.css", start: 4978, end: 5279 },
    { file: "styles/accessibility.css", start: 5280, end: 5520 },
  ];

  for (const section of sections) {
    writeFile(section.file, sliceLines(lines, section.start, section.end));
  }

  const imports = sections.map((s) => `@import "./${s.file.replace(/^styles\//, "")}";`).join("\n");
  writeFile("styles/index.css", imports + "\n");

  writeFile("styles.css", '@import "./styles/index.css";\n');

  console.log(`Split styles.css (${lines.length} lines) into styles/ modules`);
}

function readFilesLines(file) {
  return fs.readFileSync(file, "utf8").split("\n");
}

splitApp();
splitStyles();
console.log("Done.");

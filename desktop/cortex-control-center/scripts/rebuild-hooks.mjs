#!/usr/bin/env node
import { execSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "src");
const bakLines = execSync("git show HEAD:desktop/cortex-control-center/src/App.jsx", { cwd: path.join(ROOT, "..") })
  .toString()
  .split("\n");

const HOOK_HEADER = fs.readFileSync(path.join(SRC, "app/hooks/useDashboardState.js"), "utf8").split("\n").slice(0, 111).join("\n");

const FULL_RETURN = fs.readFileSync(path.join(ROOT, "scripts/split-panels-and-hooks.mjs"), "utf8")
  .match(/const HOOK_RETURN_KEYS = `([\s\S]*?)`;/)[1];

const STATE_RETURN = `browserBootstrap,
    isTauriRuntime,
    panel,
    setPanel,
    brainPanelMounted,
    setBrainPanelMounted,
    panelMotionDirection,
    setPanelMotionDirection,
    daemonState,
    setDaemonState,
    healthMeta,
    setHealthMeta,
    stats,
    setStats,
    sessions,
    setSessions,
    tasks,
    setTasks,
    locks,
    setLocks,
    feedEntries,
    setFeedEntries,
    messageEntries,
    setMessageEntries,
    activityEntries,
    setActivityEntries,
    sidebarCollapsed,
    setSidebarCollapsed,
    isNarrowViewport,
    setIsNarrowViewport,
    savings,
    setSavings,
    memoryQuery,
    setMemoryQuery,
    memoryResults,
    setMemoryResults,
    memorySearching,
    setMemorySearching,
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
    setBusyActionKey,
    activitySince,
    setActivitySince,
    feedbackMessage,
    setFeedbackMessage,
    daemonTimeoutStaleSummary,
    setDaemonTimeoutStaleSummary,
    conflictPairs,
    setConflictPairs,
    resolveDrafts,
    setResolveDrafts,
    conflictLoading,
    setConflictLoading,
    permissionGrants,
    setPermissionGrants,
    permissionLoading,
    setPermissionLoading,
    permissionAccessDenied,
    setPermissionAccessDenied,
    permissionsEndpointAvailable,
    setPermissionsEndpointAvailable,
    permissionDraft,
    setPermissionDraft,
    editorSetup,
    setEditorSetup,
    editorDetections,
    setEditorDetections,
    selectedEditorIds,
    setSelectedEditorIds,
    cortexBase,
    setCortexBase,
    showConnectionDialog,
    setShowConnectionDialog,
    showEditorSetupWizard,
    setShowEditorSetupWizard,
    availableUpdate,
    setAvailableUpdate,
    updateInstalling,
    setUpdateInstalling,
    restartingDaemon,
    setRestartingDaemon,
    restartError,
    setRestartError,
    showMissionMetricLegend,
    setShowMissionMetricLegend,
    showMissionCompactUnits,
    setShowMissionCompactUnits,
    hasVisitedAnalytics,
    setHasVisitedAnalytics,
    analyticsReady,
    setAnalyticsReady,
    startupCoreReadyState,
    setStartupCoreReadyState,
    isSettingUpEditors,
    setIsSettingUpEditors,
    controlSettings,
    setControlSettings,
    budgetConfigStatus,
    setBudgetConfigStatus,
    budgetDraft,
    setBudgetDraft,
    budgetDraftDirty,
    setBudgetDraftDirty,
    budgetConfigBusy,
    setBudgetConfigBusy,
    budgetConfigMessage,
    setBudgetConfigMessage,
    ipcAvailable,
    setIpcAvailable,
    osReducedMotion,
    setOsReducedMotion,
    currency,
    setCurrency,
    analyticsMode,
    setAnalyticsMode,
    effectiveReducedMotion,
    invokeRef,
    tokenRef,
    refreshAllRef,
    refreshAllInFlightRef,
    refreshAllQueuedRef,
    daemonTransitionRef,
    recoveryRetryTimerRef,
    startupRetryStateRef,
    startupCoreReadyRef,
    lastCoreRefreshAtRef,
    lastSecondaryRefreshAtRef,
    startupSecondaryRefreshInFlightRef,
    skipInitialFeedRefreshRef,
    skipInitialMessagesRefreshRef,
    skipInitialActivityRefreshRef,
    connectionDialogRef,
    connectionDialogTriggerRef,
    editorSetupDialogRef,
    editorSetupTriggerRef,
    topbarRef,
    analyticsPanelRef,
    brainPanelRef,
    analyticsTabRefs,
    sessionsRef,
    daemonStateRef,
    streamConnectedAtRef,
    streamDisconnectedAtRef,
    streamSessionEventCountRef,
    devVerificationStartedRef,
    permissionsEndpointAvailableRef,
    browserHealthProbeRef,
    connectionDialogAutoPromptSuppressedRef,
    budgetConfigLoadAttemptedRef,
    restoreFocusToTrigger,
    openConnectionDialog,
    dismissConnectionDialog,
    closeConnectionDialog,
    closeEditorSetupWizard,
    updateControlSetting,
    changePanel,
    normalizedSessions,
    knownAgents,
    editorSetupSummary,
    editorDetectionSummary,
    setupCommandPath,
    manualMcpSnippet,
    selectedOperatorName,
    messageTargetName,
    safeCurrency,
    currencyRate,
    activeBudgetStatus,
    budgetSummary,
    budgetDraftError,
    budgetDraftEndpoints,
    memoryLoad,
    currencyFormatter,
    formatCurrency,
    savingsEstimateLegend,
    formatMissionTokenValue,
    clearTransientFeedback,
    setSecondaryAvailabilityFeedback,
    clearRecoveryRetry,
    scheduleRecoveryRetry,
    resetStartupRetryState,
    scheduleStartupRecoveryRetry,
    clearDisconnectedData`;

function sliceByLine(startLine, endLine) {
  return bakLines.slice(startLine - 1, endLine).join("\n");
}

function definedSymbols(block) {
  const names = new Set();
  for (const match of block.matchAll(/^  (?:async function|function|const) ([A-Za-z_$][\w$]*)/gm)) {
    names.add(match[1]);
  }
  return names;
}

function destructureList(exclude = new Set()) {
  return FULL_RETURN.split(",\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .filter((name) => !exclude.has(name))
    .join(",\n    ");
}

function writeHook(file, fn, param, block, ret, { skipDestructure = false } = {}) {
  const exclude = definedSymbols(block);
  const destructure = skipDestructure || !param
    ? ""
    : `  const {
    ${destructureList(exclude)},
  } = ${param};

`;
  fs.writeFileSync(
    path.join(SRC, `app/hooks/${file}`),
    `${HOOK_HEADER}

export function ${fn}(${param}) {
${destructure}${block}
${ret}
}
`,
  );
}

writeHook("useDashboardState.js", "useDashboardState", "", sliceByLine(1459, 1906), `  return {
    ${STATE_RETURN},
  };`, { skipDestructure: true });

writeHook("useRefreshOrchestration.js", "useRefreshOrchestration", "ctx", sliceByLine(1907, 2589), "  return { ...ctx };");
writeHook("useRefreshAll.js", "useRefreshAll", "ctx", sliceByLine(2590, 2852), "  return { ...ctx };");
writeHook("useDashboardEffects.js", "useDashboardEffects", "ctx", `${sliceByLine(2853, 3099)}\n${sliceByLine(3231, 3639)}`, "  return ctx;");
writeHook("useSseStream.js", "useSseStream", "ctx", sliceByLine(3100, 3229), "  return ctx;");
writeHook("useDaemonConnection.js", "useDaemonConnection", "ctx", sliceByLine(3640, 3773), "  return ctx;");
writeHook("useDashboardHandlers.js", "useDashboardHandlers", "ctx", sliceByLine(3775, 4292), `  return {
    ...ctx,
    ${FULL_RETURN},
  };`);


// useDashboardState written above with skipDestructure
fs.writeFileSync(
  path.join(SRC, "app/hooks/useDashboardHooks.js"),
  `import { useDashboardState } from "./useDashboardState.js";
import { useRefreshOrchestration } from "./useRefreshOrchestration.js";
import { useRefreshAll } from "./useRefreshAll.js";
import { useDashboardEffects } from "./useDashboardEffects.js";
import { useSseStream } from "./useSseStream.js";
import { useDaemonConnection } from "./useDaemonConnection.js";
import { useDashboardHandlers } from "./useDashboardHandlers.js";

export function useDashboardHooks() {
  let ctx = useDashboardState();
  ctx = useRefreshOrchestration(ctx);
  ctx = useRefreshAll(ctx);
  ctx = useDashboardEffects(ctx);
  ctx = useSseStream(ctx);
  ctx = useDaemonConnection(ctx);
  return useDashboardHandlers(ctx);
}
`,
);

console.log("Rebuilt hooks from git source");

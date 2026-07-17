import { useCallback } from "react";
import { isAuthFailure, summarizeDashboardErrors } from "../../api-client.js";
import { shouldContinueStartupRecovery } from "../../daemon-startup.js";
import { readTauriInvoke, persistBrowserAuthToken } from "../browser-bootstrap.js";
import { formatDaemonEndpoint } from "../utils/format.js";
import { isDaemonOfflineErrorMessage, isDaemonTimeoutErrorMessage } from "../utils/daemon.js";

export function useRefreshAll(ctx) {
  const {
    cortexBase,
    setFeedbackMessage,
    invokeRef,
    tokenRef,
    readAuthToken,
    call,
    setIpcAvailable,
    refreshDaemonState,
    refreshHealth,
    probeReadiness,
    daemonTransitionRef,
    setDaemonState,
    daemonStateRef,
    setDaemonTimeoutStaleSummary,
    clearStartupCoreReady,
    scheduleStartupRecoveryRetry,
    clearDisconnectedData,
    resetStartupRetryState,
    clearRecoveryRetry,
    clearTransientFeedback,
    refreshProtectedDataForStartup,
    startupCoreReadyRef,
    setStartupCoreReadyState,
    refreshSecondaryDataInBackground,
    connectionDialogAutoPromptSuppressedRef,
    setShowConnectionDialog,
    setSecondaryAvailabilityFeedback,
    refreshAllInFlightRef,
    refreshAllQueuedRef,
  } = ctx;

  const refreshAll = useCallback(async () => {
    try {
      invokeRef.current = await readTauriInvoke();
    } catch {
      invokeRef.current = null;
    }
    setIpcAvailable(Boolean(invokeRef.current));

    const nextDaemonState = await refreshDaemonState();
    let healthReady = await refreshHealth();
    let readinessReady = false;
    if (
      invokeRef.current
      && nextDaemonState?.managed
      && !nextDaemonState?.reachable
      && !healthReady
    ) {
      readinessReady = await probeReadiness();
      if (readinessReady) {
        healthReady = true;
      }
    }
    const reachableViaHealthFallback =
      Boolean(invokeRef.current)
      && Boolean(healthReady)
      && !Boolean(nextDaemonState?.reachable);
    const reachableViaReadinessFallback =
      Boolean(invokeRef.current)
      && Boolean(readinessReady)
      && !Boolean(nextDaemonState?.reachable);
    const daemonReachable =
      Boolean(nextDaemonState?.reachable) || reachableViaHealthFallback || reachableViaReadinessFallback;

    if (daemonTransitionRef.current) {
      return;
    }

    if (reachableViaHealthFallback || reachableViaReadinessFallback) {
      setDaemonState((current) => ({
        ...current,
        running: true,
        reachable: true,
        managed: Boolean(nextDaemonState?.managed),
        authTokenReady: Boolean(tokenRef.current),
        message: `Connected to daemon on ${formatDaemonEndpoint(cortexBase)} (${reachableViaReadinessFallback ? "readiness fallback active" : "IPC fallback active"}).`,
      }));
    }

    const keepStartupRecovery =
      shouldContinueStartupRecovery({
        invokeAvailable: Boolean(invokeRef.current),
        daemonReachable,
        currentDaemonState: nextDaemonState,
        previousDaemonState: daemonStateRef.current,
      });

    if (keepStartupRecovery) {
      setDaemonTimeoutStaleSummary("");
      clearStartupCoreReady();
      if (!scheduleStartupRecoveryRetry("Daemon is still starting. Reconnect will continue automatically.")) {
        let timeoutMessage = `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`;
        try {
          const stopResult = await call("stop_daemon");
          if (stopResult?.message) {
            timeoutMessage = `${timeoutMessage}. ${stopResult.message}`;
          }
        } catch (error) {
          const detail = error?.message || String(error || "");
          if (detail) {
            timeoutMessage = `${timeoutMessage}. Startup recovery cleanup failed: ${detail}`;
          }
        }
        tokenRef.current = "";
        persistBrowserAuthToken("");
        clearDisconnectedData();
        setDaemonState({
          running: false,
          reachable: false,
          managed: false,
          authTokenReady: false,
          pid: null,
          message: timeoutMessage,
        });
      }
      return;
    }

    if (!daemonReachable) {
      resetStartupRetryState();
      setDaemonTimeoutStaleSummary("");
      clearStartupCoreReady();
      clearRecoveryRetry();
      if (invokeRef.current) {
        tokenRef.current = "";
        persistBrowserAuthToken("");
      }
      clearDisconnectedData();
      clearTransientFeedback(nextDaemonState?.message || `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`);
      return;
    }

    if (invokeRef.current && !healthReady) {
      setDaemonTimeoutStaleSummary("");
      clearStartupCoreReady();
      scheduleStartupRecoveryRetry("Daemon is reachable but still warming up. Retrying shortly...");
      return;
    }

    const authToken = await readAuthToken({ suppressFeedback: true });
    if (invokeRef.current && !authToken) {
      setDaemonTimeoutStaleSummary("");
      clearStartupCoreReady();
      scheduleStartupRecoveryRetry("Waiting for daemon auth token to finish rotating...");
      return;
    }

    resetStartupRetryState();
    let {
      coreErrors,
      secondaryErrors,
      coreSuccessCount,
      coreTotalCount,
    } = await refreshProtectedDataForStartup();
    if (invokeRef.current && coreErrors.length && coreErrors.every((error) => isAuthFailure(error))) {
      const refreshedToken = await readAuthToken({ suppressFeedback: true });
      if (refreshedToken) {
        ({
          coreErrors,
          secondaryErrors,
          coreSuccessCount,
          coreTotalCount,
        } = await refreshProtectedDataForStartup());
      }
    }
    const browserCoreAuthFailuresOnly =
      !invokeRef.current
      && coreErrors.length > 0
      && coreErrors.every((error) => isAuthFailure(error));
    if (browserCoreAuthFailuresOnly) {
      tokenRef.current = "";
      persistBrowserAuthToken("");
    }

    if (coreErrors.length) {
      const unique = [...new Set(coreErrors)];
      const timeoutErrors = unique.filter((error) => isDaemonTimeoutErrorMessage(error));
      const warmupErrorsOnly = unique.every(
        (error) => isDaemonTimeoutErrorMessage(error) || isAuthFailure(error)
      );
      const partialCoreReady =
        daemonReachable
        && coreSuccessCount > 0
        && warmupErrorsOnly;
      if (partialCoreReady) {
        startupCoreReadyRef.current = true;
        setStartupCoreReadyState(true);
        refreshSecondaryDataInBackground();
        const timeoutSummary = timeoutErrors.length
          ? summarizeDashboardErrors(timeoutErrors) || "IPC request timeouts detected."
          : "";
        if (timeoutSummary) {
          setDaemonTimeoutStaleSummary(timeoutSummary);
        } else {
          setDaemonTimeoutStaleSummary("");
        }
        const partialSummary = summarizeDashboardErrors(unique) || "Protected endpoints are still warming up.";
        setFeedbackMessage(
          `Connected (core ${coreSuccessCount}/${coreTotalCount || 3} ready). ${partialSummary}`
        );
        scheduleRecoveryRetry(1000);
      } else if (daemonReachable && unique.every((error) => isDaemonTimeoutErrorMessage(error))) {
        clearStartupCoreReady();
        clearRecoveryRetry();
        const summary = summarizeDashboardErrors(unique) || "IPC request timeouts detected.";
        setDaemonTimeoutStaleSummary(summary);
        setFeedbackMessage(
          summary
            ? `Connected (core stale). IPC requests timed out: ${summary}`
            : "Connected (core stale). IPC requests timed out; retrying."
        );
        scheduleRecoveryRetry(1000);
      } else if (unique.every((error) => isDaemonOfflineErrorMessage(error))) {
        clearStartupCoreReady();
        setDaemonTimeoutStaleSummary("");
        clearDisconnectedData();
        clearTransientFeedback(nextDaemonState?.message || `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`);
        scheduleRecoveryRetry(1000);
      } else if (invokeRef.current && unique.every((error) => isAuthFailure(error))) {
        clearStartupCoreReady();
        setDaemonTimeoutStaleSummary("");
        setFeedbackMessage("Waiting for daemon auth token to finish rotating...");
        scheduleRecoveryRetry(1000);
      } else {
        clearStartupCoreReady();
        setDaemonTimeoutStaleSummary("");
        clearRecoveryRetry();
        setFeedbackMessage(summarizeDashboardErrors(unique));
        if (
          !invokeRef.current
          && unique.every((error) => isAuthFailure(error))
          && !connectionDialogAutoPromptSuppressedRef.current
        ) {
          setShowConnectionDialog(true);
        }
      }
    } else {
      connectionDialogAutoPromptSuppressedRef.current = false;
      clearRecoveryRetry();
      const uniqueSecondary = [...new Set(secondaryErrors)];
      if (uniqueSecondary.length) {
        const timeoutErrors = uniqueSecondary.filter((error) => isDaemonTimeoutErrorMessage(error));
        if (timeoutErrors.length) {
          setDaemonTimeoutStaleSummary(summarizeDashboardErrors(timeoutErrors) || "IPC request timeouts detected.");
        } else {
          setDaemonTimeoutStaleSummary("");
        }
        setSecondaryAvailabilityFeedback(uniqueSecondary);
      } else {
        setDaemonTimeoutStaleSummary("");
        clearTransientFeedback();
      }
    }
  }, [
    call,
    clearStartupCoreReady,
    clearRecoveryRetry,
    clearTransientFeedback,
    readAuthToken,
    refreshDaemonState,
    refreshHealth,
    probeReadiness,
    refreshProtectedDataForStartup,
    clearDisconnectedData,
    cortexBase,
    resetStartupRetryState,
    scheduleRecoveryRetry,
    scheduleStartupRecoveryRetry,
    refreshSecondaryDataInBackground,
    setSecondaryAvailabilityFeedback,
  ]);

  const runRefreshAll = useCallback(() => {
    if (refreshAllInFlightRef.current) {
      refreshAllQueuedRef.current = true;
      return refreshAllInFlightRef.current;
    }

    let pendingRefresh = null;
    pendingRefresh = (async () => {
      do {
        refreshAllQueuedRef.current = false;
        await refreshAll();
      } while (refreshAllQueuedRef.current);
    })().finally(() => {
      if (refreshAllInFlightRef.current === pendingRefresh) {
        refreshAllInFlightRef.current = null;
      }
    });

    refreshAllInFlightRef.current = pendingRefresh;
    return pendingRefresh;
  }, [refreshAll]);

  return {
    ...ctx,
    runRefreshAll,
  };
}

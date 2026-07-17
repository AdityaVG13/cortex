import { useCallback } from "react";
import { isAuthFailure, summarizeDashboardErrors } from "../../api-client.js";
import { shouldContinueStartupRecovery } from "../../daemon-startup.js";
import {
  readTauriInvoke,
  persistBrowserAuthToken,
} from "../browser-bootstrap.js";
import { formatDaemonEndpoint } from "../utils/format.js";
import {
  isDaemonOfflineErrorMessage,
  isDaemonTimeoutErrorMessage,
} from "../utils/daemon.js";
function useRefreshAll(ctx) {
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
    } = ctx,
    refreshAll = useCallback(async () => {
      try {
        invokeRef.current = await readTauriInvoke();
      } catch {
        invokeRef.current = null;
      }
      setIpcAvailable(!!invokeRef.current);
      const nextDaemonState = await refreshDaemonState();
      let healthReady = await refreshHealth(),
        readinessReady = !1;
      invokeRef.current &&
        nextDaemonState?.managed &&
        !nextDaemonState?.reachable &&
        !healthReady &&
        ((readinessReady = await probeReadiness()),
        readinessReady && (healthReady = !0));
      const reachableViaHealthFallback =
          !!invokeRef.current && !!healthReady && !nextDaemonState?.reachable,
        reachableViaReadinessFallback =
          !!invokeRef.current &&
          !!readinessReady &&
          !nextDaemonState?.reachable,
        daemonReachable =
          !!nextDaemonState?.reachable ||
          reachableViaHealthFallback ||
          reachableViaReadinessFallback;
      if (daemonTransitionRef.current) return;
      if (
        ((reachableViaHealthFallback || reachableViaReadinessFallback) &&
          setDaemonState((current) => ({
            ...current,
            running: !0,
            reachable: !0,
            managed: !!nextDaemonState?.managed,
            authTokenReady: !!tokenRef.current,
            message: `Connected to daemon on ${formatDaemonEndpoint(cortexBase)} (${reachableViaReadinessFallback ? "readiness fallback active" : "IPC fallback active"}).`,
          })),
        shouldContinueStartupRecovery({
          invokeAvailable: !!invokeRef.current,
          daemonReachable,
          currentDaemonState: nextDaemonState,
          previousDaemonState: daemonStateRef.current,
        }))
      ) {
        if (
          (setDaemonTimeoutStaleSummary(""),
          clearStartupCoreReady(),
          !scheduleStartupRecoveryRetry(
            "Daemon is still starting. Reconnect will continue automatically.",
          ))
        ) {
          let timeoutMessage = `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`;
          try {
            const stopResult = await call("stop_daemon");
            stopResult?.message &&
              (timeoutMessage = `${timeoutMessage}. ${stopResult.message}`);
          } catch (error) {
            const detail = error?.message || String(error || "");
            detail &&
              (timeoutMessage = `${timeoutMessage}. Startup recovery cleanup failed: ${detail}`);
          }
          ((tokenRef.current = ""),
            persistBrowserAuthToken(""),
            clearDisconnectedData(),
            setDaemonState({
              running: !1,
              reachable: !1,
              managed: !1,
              authTokenReady: !1,
              pid: null,
              message: timeoutMessage,
            }));
        }
        return;
      }
      if (!daemonReachable) {
        (resetStartupRetryState(),
          setDaemonTimeoutStaleSummary(""),
          clearStartupCoreReady(),
          clearRecoveryRetry(),
          invokeRef.current &&
            ((tokenRef.current = ""), persistBrowserAuthToken("")),
          clearDisconnectedData(),
          clearTransientFeedback(
            nextDaemonState?.message ||
              `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`,
          ));
        return;
      }
      if (invokeRef.current && !healthReady) {
        (setDaemonTimeoutStaleSummary(""),
          clearStartupCoreReady(),
          scheduleStartupRecoveryRetry(
            "Daemon is reachable but still warming up. Retrying shortly...",
          ));
        return;
      }
      const authToken = await readAuthToken({ suppressFeedback: !0 });
      if (invokeRef.current && !authToken) {
        (setDaemonTimeoutStaleSummary(""),
          clearStartupCoreReady(),
          scheduleStartupRecoveryRetry(
            "Waiting for daemon auth token to finish rotating...",
          ));
        return;
      }
      resetStartupRetryState();
      let { coreErrors, secondaryErrors, coreSuccessCount, coreTotalCount } =
        await refreshProtectedDataForStartup();
      if (
        (invokeRef.current &&
          coreErrors.length &&
          coreErrors.every((error) => isAuthFailure(error)) &&
          (await readAuthToken({ suppressFeedback: !0 })) &&
          ({ coreErrors, secondaryErrors, coreSuccessCount, coreTotalCount } =
            await refreshProtectedDataForStartup()),
        !invokeRef.current &&
          coreErrors.length > 0 &&
          coreErrors.every((error) => isAuthFailure(error)) &&
          ((tokenRef.current = ""), persistBrowserAuthToken("")),
        coreErrors.length)
      ) {
        const unique = [...new Set(coreErrors)],
          timeoutErrors = unique.filter((error) =>
            isDaemonTimeoutErrorMessage(error),
          ),
          warmupErrorsOnly = unique.every(
            (error) =>
              isDaemonTimeoutErrorMessage(error) || isAuthFailure(error),
          );
        if (daemonReachable && coreSuccessCount > 0 && warmupErrorsOnly) {
          ((startupCoreReadyRef.current = !0),
            setStartupCoreReadyState(!0),
            refreshSecondaryDataInBackground());
          const timeoutSummary = timeoutErrors.length
            ? summarizeDashboardErrors(timeoutErrors) ||
              "IPC request timeouts detected."
            : "";
          setDaemonTimeoutStaleSummary(timeoutSummary || "");
          const partialSummary =
            summarizeDashboardErrors(unique) ||
            "Protected endpoints are still warming up.";
          (setFeedbackMessage(
            `Connected (core ${coreSuccessCount}/${coreTotalCount || 3} ready). ${partialSummary}`,
          ),
            scheduleRecoveryRetry(1e3));
        } else if (
          daemonReachable &&
          unique.every((error) => isDaemonTimeoutErrorMessage(error))
        ) {
          (clearStartupCoreReady(), clearRecoveryRetry());
          const summary =
            summarizeDashboardErrors(unique) ||
            "IPC request timeouts detected.";
          (setDaemonTimeoutStaleSummary(summary),
            setFeedbackMessage(
              summary
                ? `Connected (core stale). IPC requests timed out: ${summary}`
                : "Connected (core stale). IPC requests timed out; retrying.",
            ),
            scheduleRecoveryRetry(1e3));
        } else
          unique.every((error) => isDaemonOfflineErrorMessage(error))
            ? (clearStartupCoreReady(),
              setDaemonTimeoutStaleSummary(""),
              clearDisconnectedData(),
              clearTransientFeedback(
                nextDaemonState?.message ||
                  `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`,
              ),
              scheduleRecoveryRetry(1e3))
            : invokeRef.current && unique.every((error) => isAuthFailure(error))
              ? (clearStartupCoreReady(),
                setDaemonTimeoutStaleSummary(""),
                setFeedbackMessage(
                  "Waiting for daemon auth token to finish rotating...",
                ),
                scheduleRecoveryRetry(1e3))
              : (clearStartupCoreReady(),
                setDaemonTimeoutStaleSummary(""),
                clearRecoveryRetry(),
                setFeedbackMessage(summarizeDashboardErrors(unique)),
                !invokeRef.current &&
                  unique.every((error) => isAuthFailure(error)) &&
                  !connectionDialogAutoPromptSuppressedRef.current &&
                  setShowConnectionDialog(!0));
      } else {
        ((connectionDialogAutoPromptSuppressedRef.current = !1),
          clearRecoveryRetry());
        const uniqueSecondary = [...new Set(secondaryErrors)];
        if (uniqueSecondary.length) {
          const timeoutErrors = uniqueSecondary.filter((error) =>
            isDaemonTimeoutErrorMessage(error),
          );
          (timeoutErrors.length
            ? setDaemonTimeoutStaleSummary(
                summarizeDashboardErrors(timeoutErrors) ||
                  "IPC request timeouts detected.",
              )
            : setDaemonTimeoutStaleSummary(""),
            setSecondaryAvailabilityFeedback(uniqueSecondary));
        } else (setDaemonTimeoutStaleSummary(""), clearTransientFeedback());
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
    ]),
    runRefreshAll = useCallback(() => {
      if (refreshAllInFlightRef.current)
        return (
          (refreshAllQueuedRef.current = !0),
          refreshAllInFlightRef.current
        );
      let pendingRefresh = null;
      return (
        (pendingRefresh = (async () => {
          do ((refreshAllQueuedRef.current = !1), await refreshAll());
          while (refreshAllQueuedRef.current);
        })().finally(() => {
          refreshAllInFlightRef.current === pendingRefresh &&
            (refreshAllInFlightRef.current = null);
        })),
        (refreshAllInFlightRef.current = pendingRefresh),
        pendingRefresh
      );
    }, [refreshAll]);
  return { ...ctx, runRefreshAll };
}
export { useRefreshAll };

import { useCallback } from "react";
import { DAEMON_START_POLL_INTERVAL_MS, DAEMON_START_STILL_STARTING_GRACE_MS, DAEMON_START_WAIT_TIMEOUT_MS, DAEMON_STOP_HANG_TIMEOUT_MS, DAEMON_STOP_WAIT_TIMEOUT_MS, EMPTY_DAEMON } from "../constants.js";
import { persistBrowserAuthToken } from "../browser-bootstrap.js";
import { formatDaemonEndpoint } from "../utils/format.js";
import { isDaemonOfflineErrorMessage, isReachableHealthPayload } from "../utils/daemon.js";

export function useDaemonConnection(ctx) {
  const {
    cortexBase,
    setFeedbackMessage,
    invokeRef,
    tokenRef,
    runRefreshAll,
    readAuthToken,
    api,
    call,
    setDaemonState,
    daemonTransitionRef,
    resetStartupRetryState,
    clearDisconnectedData,
    scheduleStartupRecoveryRetry,
  } = ctx;

  const waitForDaemonReachable = useCallback(async (options = {}) => {
    const shortCircuitIfStarting = options?.shortCircuitIfStarting === true;
    const started = Date.now();
    while (Date.now() - started < DAEMON_START_WAIT_TIMEOUT_MS) {
      try {
        if (invokeRef.current) {
          const state = { ...EMPTY_DAEMON, ...(await call("daemon_status")) };
          setDaemonState(state);
          if (state?.reachable) return true;
          if (
            shortCircuitIfStarting
            && state?.running
            && !state?.reachable
            && Date.now() - started >= DAEMON_START_STILL_STARTING_GRACE_MS
          ) {
            return false;
          }
        } else {
          const health = await api("/health");
          if (isReachableHealthPayload(health)) return true;
        }
      } catch {
        // continue polling until timeout
      }
      await new Promise((resolve) => setTimeout(resolve, DAEMON_START_POLL_INTERVAL_MS));
    }
    return false;
  }, [api, call]);

  const waitForDaemonOffline = useCallback(async () => {
    const started = Date.now();
    while (Date.now() - started < DAEMON_STOP_WAIT_TIMEOUT_MS) {
      try {
        if (invokeRef.current) {
          const state = await call("daemon_status");
          setDaemonState(state);
          if (!state?.reachable) return true;
        } else {
          await api("/health");
        }
      } catch (error) {
        if (isDaemonOfflineErrorMessage(error?.message || error)) {
          return true;
        }
      }
      await new Promise((resolve) => setTimeout(resolve, DAEMON_START_POLL_INTERVAL_MS));
    }
    return false;
  }, [api, call]);

  const runRestartDaemonSequence = useCallback(async () => {
    daemonTransitionRef.current = true;
    resetStartupRetryState();

    const statusBefore = await call("daemon_status").catch(() => null);
    const shouldStop = Boolean(statusBefore?.running || statusBefore?.reachable);
    const managedBefore = Boolean(statusBefore?.managed);
    let restartSkippedExternal = false;
    let startResult = null;

    if (shouldStop) {
      setFeedbackMessage("Restarting daemon: stopping...");
      const stopPromise = call("stop_daemon")
        .then((result) => ({ ok: true, result }))
        .catch((error) => ({ ok: false, error: error?.message || String(error) }));
      const stopResult = await Promise.race([
        stopPromise,
        new Promise((resolve) => setTimeout(() => resolve({ timedOut: true }), DAEMON_STOP_HANG_TIMEOUT_MS)),
      ]);
      let stopFailure = "";
      if (stopResult?.timedOut) {
        setFeedbackMessage("Shutdown is taking longer than expected. Waiting for daemon to go offline...");
      } else if (!stopResult?.ok) {
        stopFailure = stopResult?.error || "Existing daemon rejected shutdown.";
      }
      const stopState = stopResult?.ok ? stopResult.result : null;
      const unmanagedStillReachable = Boolean(stopState?.reachable && !stopState?.managed);
      const stopped = unmanagedStillReachable ? false : await waitForDaemonOffline();
      if (!stopped) {
        if (unmanagedStillReachable && !managedBefore) {
          restartSkippedExternal = true;
          setFeedbackMessage("Daemon is externally managed and remained online. Continuing without forced shutdown.");
        } else {
          throw new Error(stopFailure || "Existing daemon did not stop cleanly.");
        }
      }
      if (!restartSkippedExternal) {
        tokenRef.current = "";
        persistBrowserAuthToken("");
        clearDisconnectedData();
        setDaemonState({
          running: false,
          reachable: false,
          managed: false,
          authTokenReady: false,
          pid: null,
          message: `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`,
        });
      }
    } else {
      setFeedbackMessage("Daemon already stopped. Starting...");
    }

    if (!restartSkippedExternal) {
      setFeedbackMessage("Restarting daemon: starting...");
      startResult = await call("start_daemon");
      if (startResult?.message) {
        setFeedbackMessage(startResult.message);
      }

      const reachable = await waitForDaemonReachable({ shortCircuitIfStarting: true });
      if (!reachable) {
        if (startResult?.running && !startResult?.reachable) {
          scheduleStartupRecoveryRetry("Daemon is still starting. Reconnect will continue automatically.");
        } else {
          throw new Error("Daemon did not become reachable after restart.");
        }
      }
    } else {
      startResult = await call("daemon_status").catch(() => ({
        running: true,
        reachable: true,
        managed: false,
        authTokenReady: Boolean(tokenRef.current),
        pid: null,
        message: "Daemon remained online (externally managed).",
      }));
    }

    daemonTransitionRef.current = false;
    await readAuthToken({ suppressFeedback: true });
    await runRefreshAll();
    return { ...startResult, restartSkippedExternal };
  }, [call, clearDisconnectedData, cortexBase, readAuthToken, resetStartupRetryState, runRefreshAll, scheduleStartupRecoveryRetry, waitForDaemonOffline, waitForDaemonReachable]);
  return {
    ...ctx,
    waitForDaemonReachable,
    waitForDaemonOffline,
    runRestartDaemonSequence,
  };
}

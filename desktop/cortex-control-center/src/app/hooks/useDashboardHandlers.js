import { useCallback, useEffect, useMemo } from "react";
import { buildFirstRunReadiness } from "../../daemon-startup.js";
import { shouldIgnoreGlobalShortcut, trapFocusInContainer } from "../../keyboard-access.js";
import {
  DEFAULT_CORTEX_BASE, PANEL_SEQUENCE, PANEL_SEQUENCE_LABEL, panelIndex, } from "../constants.js";
import { persistBrowserAuthToken } from "../browser-bootstrap.js";
import { formatDaemonEndpoint } from "../utils/format.js";
import { setElementInert } from "../utils/daemon.js";
function useDashboardHandlers(ctx) { const { panel, sidebarCollapsed, isNarrowViewport, daemonState, healthMeta, stats,
    normalizedSessions, editorSetupSummary, isSettingUpEditors, memoryQuery, cortexBase, setFeedbackMessage, restartingDaemon, setRestartingDaemon,
    setRestartError, analyticsMode, setAnalyticsMode, analyticsTabRefs, invokeRef, tokenRef, connectionDialogRef, editorSetupDialogRef,
    topbarRef, analyticsPanelRef, brainPanelRef, showConnectionDialog, showEditorSetupWizard, changePanel, runRefreshAll, readAuthToken,
    api, call, resetStartupRetryState, daemonTransitionRef, waitForDaemonReachable, waitForDaemonOffline, clearDisconnectedData, setDaemonState,
    scheduleStartupRecoveryRetry, runRestartDaemonSequence, openEditorSetupWizard, closeEditorSetupWizard,
    dismissConnectionDialog, setMemorySearching, setMemoryResults, } = ctx;
  async function handleMemorySearch(e) { if ((e?.preventDefault(), !!memoryQuery.trim())) { setMemorySearching(!0);
      try { const peekResult = await api(`/peek?q=${encodeURIComponent(memoryQuery.trim())}&k=15`, !0);
        setMemoryResults(peekResult?.matches || []);
      } catch { setMemoryResults([]);
      }
      setMemorySearching(!1);
    }
  }
  async function handleMemoryExpand(source) { try {
      const match = (await api(`/recall?q=${encodeURIComponent(source)}&k=3`, !0))?.results?.find( (r) => r.source === source, );
      match && setMemoryResults((prev) => prev.map((m) => (m.source === source ? { ...m, excerpt: match.excerpt, expanded: !0 } : m)), );
    } catch (err) { setFeedbackMessage(`Memory expand failed: ${err.message || err}`);
    }
  }
  async function handleStartDaemon() { if (invokeRef.current) { (resetStartupRetryState(), (daemonTransitionRef.current = !0));
      try { const result = await call("start_daemon");
        (setFeedbackMessage(result.message || "Daemon start requested."), (await waitForDaemonReachable({ shortCircuitIfStarting: !0 })) ||
            scheduleStartupRecoveryRetry("Daemon is still starting. Reconnect will continue automatically."), (daemonTransitionRef.current = !1),
          await readAuthToken({ suppressFeedback: !0 }), await runRefreshAll());
      } catch (error) { setFeedbackMessage(`Start failed: ${error.message || error}`);
      } finally { daemonTransitionRef.current = !1;
      }
    }
  }
  async function handleStopDaemon() { if (invokeRef.current) { (resetStartupRetryState(), (daemonTransitionRef.current = !0));
      try { const result = await call("stop_daemon");
        setFeedbackMessage(result.message || "Daemon stop requested.");
        const offline = await waitForDaemonOffline();
        ((tokenRef.current = ""), persistBrowserAuthToken(""), offline
            ? (clearDisconnectedData(), setDaemonState({ running: !1, reachable: !1,
                managed: !1, authTokenReady: !1, pid: null, message: `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`,
              }), setFeedbackMessage(result.message || "Stopped Cortex daemon."))
            : (setFeedbackMessage("Shutdown is taking longer than expected. Waiting for daemon to go offline..."), await runRefreshAll()));
      } catch (error) { setFeedbackMessage(`Stop failed: ${error.message || error}`);
      } finally { daemonTransitionRef.current = !1;
      }
    }
  }
  async function handleRestartDaemon() { if (!(!invokeRef.current || restartingDaemon)) { (setRestartingDaemon(!0), setRestartError(""));
      try { (await runRestartDaemonSequence(), setFeedbackMessage("Daemon restarted successfully."));
      } catch (error) { const message = error?.message || String(error);
        (setRestartError(message), setFeedbackMessage(`Restart failed: ${message}`));
      } finally { ((daemonTransitionRef.current = !1), setRestartingDaemon(!1));
      }
    }
  }
  (useEffect(() => { if (!showConnectionDialog && !showEditorSetupWizard) return;
      function handleDialogKey(event) { if (event.key === "Tab") {
          const dialog = showEditorSetupWizard ? editorSetupDialogRef.current : connectionDialogRef.current;
          trapFocusInContainer(event, dialog);
          return;
        }
        if (event.key === "Escape") { if (showEditorSetupWizard && !isSettingUpEditors) { (event.preventDefault(), closeEditorSetupWizard());
            return;
          }
          showConnectionDialog && (event.preventDefault(), dismissConnectionDialog());
        }
      }
      return ( window.addEventListener("keydown", handleDialogKey), () => window.removeEventListener("keydown", handleDialogKey) );
    }, [ closeEditorSetupWizard, dismissConnectionDialog, isSettingUpEditors, showConnectionDialog, showEditorSetupWizard, ]), useEffect(() => {
      if (!showConnectionDialog) return;
      const frame = window.requestAnimationFrame(() => { connectionDialogRef.current?.focus();
      });
      return () => window.cancelAnimationFrame(frame);
    }, [showConnectionDialog]), useEffect(() => { if (!showEditorSetupWizard) return;
      const frame = window.requestAnimationFrame(() => { editorSetupDialogRef.current?.focus();
      });
      return () => window.cancelAnimationFrame(frame);
    }, [showEditorSetupWizard]), useEffect(() => {
      (setElementInert(topbarRef.current, panel === "overview"), setElementInert(analyticsPanelRef.current, panel !== "analytics"),
        setElementInert(brainPanelRef.current, panel !== "brain"));
    }, [panel]), useEffect(() => { function handleKey(e) { if (shouldIgnoreGlobalShortcut(e, showConnectionDialog || showEditorSetupWizard)) return;
        const idx = panelIndex(panel);
        if (e.key === "ArrowDown" || e.key === "j")
          (e.preventDefault(), changePanel(PANEL_SEQUENCE[(idx + 1) % PANEL_SEQUENCE.length].key));
        else if (e.key === "ArrowUp" || e.key === "k")
          (e.preventDefault(), changePanel(PANEL_SEQUENCE[(idx - 1 + PANEL_SEQUENCE.length) % PANEL_SEQUENCE.length].key));
        else { const num = parseInt(e.key);
          num >= 1 && num <= PANEL_SEQUENCE.length && (e.preventDefault(), changePanel(PANEL_SEQUENCE[num - 1].key));
        }
      }
      return (window.addEventListener("keydown", handleKey), () => window.removeEventListener("keydown", handleKey));
    }, [changePanel, panel, showConnectionDialog, showEditorSetupWizard]));
  const effectiveSidebarCollapsed = sidebarCollapsed || isNarrowViewport, canStartDaemon = !!(invokeRef.current && !restartingDaemon && !daemonState.running),
    canStopDaemon = !!(invokeRef.current && !restartingDaemon && (daemonState.reachable || daemonState.running)),
    canSetupEditors = !!(invokeRef.current && !isSettingUpEditors), firstRunReadiness = useMemo( () => buildFirstRunReadiness({
          daemonState, stats, sessions: normalizedSessions, editorSetupSummary, healthMeta, canStartDaemon, canSetupEditors, isSettingUpEditors,
        }), [ canSetupEditors, canStartDaemon, daemonState.reachable, daemonState.running, editorSetupSummary.registered, healthMeta.dbCorrupted,
        healthMeta.degraded, isSettingUpEditors, normalizedSessions.length, stats.decisions, stats.memories, ], );
  function handleFirstRunAction() { if (!firstRunReadiness.action.disabled)
      switch (firstRunReadiness.action.kind) { case "start_daemon": handleStartDaemon();
          break;
        case "restart_daemon": handleRestartDaemon();
          break;
        case "setup_mcp": openEditorSetupWizard();
          break;
        case "open_memory": changePanel("memory");
          break;
        default: runRefreshAll();
          break;
      }
  }
  const activePanelLabel = PANEL_SEQUENCE_LABEL.get(panel) || "Overview", connectionEndpoint = useMemo(() => { const fallback = { host: "127.0.0.1",
        port: "7437", hostLabel: cortexBase === DEFAULT_CORTEX_BASE ? "LOCAL" : "?", };
      try { const url = new URL(cortexBase);
        return { host: url.hostname || fallback.host,
          port: url.port || fallback.port, hostLabel: cortexBase === DEFAULT_CORTEX_BASE ? "LOCAL" : url.hostname || "?", };
      } catch { return fallback;
      }
    }, [cortexBase]), hostLabel = connectionEndpoint.hostLabel, handleAnalyticsTabKey = useCallback( (event) => {
        const order = ["aggregate", "operations"], currentIndex = Math.max(0, order.indexOf(analyticsMode));
        let nextIndex = null;
        if ( (event.key === "ArrowRight" || event.key === "ArrowDown"
            ? (nextIndex = (currentIndex + 1) % order.length)
            : event.key === "ArrowLeft" || event.key === "ArrowUp"
              ? (nextIndex = (currentIndex - 1 + order.length) % order.length)
              : event.key === "Home"
                ? (nextIndex = 0)
                : event.key === "End" && (nextIndex = order.length - 1), nextIndex === null) )
          return;
        event.preventDefault();
        const nextMode = order[nextIndex];
        (setAnalyticsMode(nextMode), window.requestAnimationFrame(() => { analyticsTabRefs.current[nextMode]?.focus();
          })); }, [analyticsMode], );
  return { ...ctx, effectiveSidebarCollapsed, canStartDaemon, canStopDaemon, canSetupEditors, firstRunReadiness, handleFirstRunAction,
    activePanelLabel, connectionEndpoint, hostLabel, handleAnalyticsTabKey, handleMemorySearch, handleMemoryExpand, handleStartDaemon, handleStopDaemon,
    handleRestartDaemon, };
}
export { useDashboardHandlers };

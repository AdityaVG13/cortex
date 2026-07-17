import { useCallback } from "react";
import { createApi, createPostApi, settledCollectErrors, summarizeDashboardErrors } from "../../api-client.js";
import { filterFeedEntries, normalizeTask } from "../../live-surface.js";
import {
  createBudgetDraftFromStatus, serializeBudgetDraftForSave, validateBudgetDraft, } from "../../settings/settings-state.js";
import {
  CORE_REFRESH_MIN_INTERVAL_MS, EMPTY_DAEMON, EMPTY_HEALTH_META, SECONDARY_REFRESH_MIN_INTERVAL_MS, } from "../constants.js";
import { readPersistedBrowserAuthToken, persistBrowserAuthToken } from "../browser-bootstrap.js";
import { formatDaemonEndpoint } from "../utils/format.js";
import { isRouteMissingError, normalizeConflictPairsPayload } from "../normalize/conflicts.js";
import { normalizePermissionPayload } from "../normalize/permissions.js";
import {
  isDaemonSuppressibleErrorMessage, isDaemonTimeoutErrorMessage, isReadyReadinessPayload, isReachableHealthPayload, } from "../utils/daemon.js";
function useRefreshOrchestration(ctx) { const { panel, stats, sessions, tasks, locks, savings,
      feedFilters, activitySince, permissionsEndpointAvailable, permissionDraft, setPermissionDraft, selectedEditorIds, cortexBase, setFeedbackMessage,
      budgetDraft, invokeRef, tokenRef, editorSetupTriggerRef, selectedOperatorName, closeEditorSetupWizard, daemonTransitionRef, browserHealthProbeRef,
      setDaemonState, clearTransientFeedback, permissionsEndpointAvailableRef, lastCoreRefreshAtRef,
      lastSecondaryRefreshAtRef, startupSecondaryRefreshInFlightRef, daemonStateRef, setDaemonTimeoutStaleSummary,
      setSecondaryAvailabilityFeedback, startupCoreReadyRef, setStartupCoreReadyState, budgetConfigLoadAttemptedRef,
    } = ctx, refreshTokenForApi = useCallback(async () => {
      if (!invokeRef.current) return ((tokenRef.current = readPersistedBrowserAuthToken()), tokenRef.current);
      try { const token = await invokeRef.current("read_auth_token");
        ((tokenRef.current = token || ""), persistBrowserAuthToken(tokenRef.current));
      } catch {}
      return tokenRef.current;
    }, []), api = useCallback( createApi({ getInvoke: () => invokeRef.current,
        getToken: () => tokenRef.current, cortexBase, onTokenRefresh: refreshTokenForApi, }),
      [cortexBase, refreshTokenForApi], ), postApi = useCallback( createPostApi({
        getInvoke: () => invokeRef.current, getToken: () => tokenRef.current, cortexBase, onTokenRefresh: refreshTokenForApi,
      }), [cortexBase, refreshTokenForApi], ), call = useCallback(async (command, args = {}) => {
      if (!invokeRef.current) throw new Error("No Tauri IPC available");
      return invokeRef.current(command, args);
    }, []), readAuthToken = useCallback(
      async ({ suppressFeedback = !1 } = {}) => { if (!invokeRef.current) return ((tokenRef.current = readPersistedBrowserAuthToken()), tokenRef.current);
        if (invokeRef.current)
          try { const token = await call("read_auth_token");
            return ((tokenRef.current = token || ""), persistBrowserAuthToken(tokenRef.current), tokenRef.current);
          } catch (err) { ((tokenRef.current = ""), persistBrowserAuthToken(""));
            const message = err?.message || String(err);
            !suppressFeedback && (!daemonTransitionRef.current || !isDaemonSuppressibleErrorMessage(message)) &&
              setFeedbackMessage(`Auth token read failed: ${message}`);
          }
        return tokenRef.current; }, [call], ), refreshDaemonState = useCallback(async () => { if (invokeRef.current)
        try { const state = { ...EMPTY_DAEMON, ...(await call("daemon_status")) };
          return ((browserHealthProbeRef.current = null), setDaemonState(state), state);
        } catch {}
      let health;
      try { ((health = await api("/health")), (browserHealthProbeRef.current = health || null));
      } catch { browserHealthProbeRef.current = null;
      }
      if (isReachableHealthPayload(health)) { const nextState = { running: !0, reachable: !0,
          managed: !1, authTokenReady: !!tokenRef.current, pid: null, message: `Connected -- ${health.stats?.memories ?? 0} memories`, };
        return (setDaemonState(nextState), nextState);
      } else { const nextState = { running: !1, reachable: !1,
          managed: !1, authTokenReady: !1, pid: null, message: `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`, };
        return (setDaemonState(nextState), nextState);
      }
    }, [api, call]), probeReadiness = useCallback(async () => { try { const readiness = await api("/readiness");
        return isReadyReadinessPayload(readiness);
      } catch { return !1;
      }
    }, [api]), refreshHealth = useCallback(async () => { let health = browserHealthProbeRef.current;
      if (((browserHealthProbeRef.current = null), !health))
        try { health = await api("/health");
        } catch {}
      if (!health) { const readinessReady = await probeReadiness();
        return ( setHealthMeta(EMPTY_HEALTH_META), setStats({ memories: "--", decisions: "--", events: "--" }), readinessReady );
      }
      const status = String(health?.status || "unknown").toLowerCase(), runtimeVersion = String(health?.runtime?.version || "");
      if ( (setHealthMeta({ status, degraded: !!health?.degraded, dbCorrupted: !!health?.db_corrupted, runtimeVersion, budgets: health?.budgets || null, }),
        !health?.stats) )
        return (setStats({ memories: "--", decisions: "--", events: "--" }), isReachableHealthPayload(health));
      const next = health.stats;
      return ( setStats({ memories: next.memories ?? 0, decisions: next.decisions ?? 0, events: next.events ?? 0, }), isReachableHealthPayload(health) );
    }, [api, probeReadiness]), refreshCoreData = useCallback( async (options = {}) => { const throwOnError = options?.throwOnError !== !1,
          jobs = [ { fn: () => api("/sessions", !0), apply: (v) => setSessions(Array.isArray(v?.sessions) ? v.sessions : []),
            }, { fn: () => api("/locks", !0), apply: (v) => setLocks(Array.isArray(v?.locks) ? v.locks : []),
            }, { fn: () => api("/tasks?status=all", !0), apply: (v) => setTasks(Array.isArray(v?.tasks) ? v.tasks.map(normalizeTask) : []),
            }, ], results = await Promise.allSettled(jobs.map((job) => job.fn())), errors = [];
        let successCount = 0;
        (results.forEach((result, index) => { if (result.status === "fulfilled") { (jobs[index].apply(result.value), (successCount += 1));
            return;
          }
          errors.push(result.reason?.message || String(result.reason));
        }), successCount > 0 && clearTransientFeedback());
        const summary = { errors: [...new Set(errors)], successCount, totalCount: jobs.length, };
        if (throwOnError && summary.errors.length) throw new Error(summary.errors.join("; "));
        return summary; }, [api, clearTransientFeedback], ), refreshFeed = useCallback(async () => { const query = new URLSearchParams();
      (query.set("since", feedFilters.since), feedFilters.kind !== "all" && query.set("kind", feedFilters.kind), feedFilters.unread && selectedOperatorName &&
          (query.set("agent", selectedOperatorName), query.set("unread", "true")));
      const feedResult = await api(`/feed?${query.toString()}`, !0), entries = Array.isArray(feedResult?.entries) ? [...feedResult.entries].reverse() : [];
      (setFeedEntries(filterFeedEntries(entries, feedFilters.agent)), clearTransientFeedback());
    }, [api, clearTransientFeedback, feedFilters, selectedOperatorName]), refreshMessages = useCallback(async () => { const operator = selectedOperatorName;
      if (!operator) { setMessageEntries([]);
        return;
      }
      const query = new URLSearchParams();
      query.set("agent", operator);
      const result = await api(`/messages?${query.toString()}`, !0), entries = Array.isArray(result?.messages) ? [...result.messages].reverse() : [];
      (setMessageEntries(entries), clearTransientFeedback());
    }, [api, clearTransientFeedback, selectedOperatorName]), refreshActivity = useCallback(async () => { const query = new URLSearchParams();
      query.set("since", activitySince);
      const result = await api(`/activity?${query.toString()}`, !0), entries = Array.isArray(result?.activities) ? [...result.activities].reverse() : [];
      (setActivityEntries(entries), clearTransientFeedback());
    }, [activitySince, api, clearTransientFeedback]), refreshSavings = useCallback(async () => { const result = await api("/savings", !0);
      (result && setSavings(result), clearTransientFeedback());
    }, [api, clearTransientFeedback]), refreshConflicts = useCallback(async () => {
      const result = await api("/conflicts", !0), normalizedPairs = normalizeConflictPairsPayload(result);
      (setConflictPairs(normalizedPairs), setResolveDrafts((current) => { if (!current || typeof current != "object") return {};
          const next = {}, validKeys = new Set(normalizedPairs.map((pair) => pair.key));
          for (const [key, value] of Object.entries(current)) validKeys.has(key) && (next[key] = value);
          return next;
        }), clearTransientFeedback());
    }, [api, clearTransientFeedback]), refreshPermissions = useCallback(
      async (options = {}) => { if (!(!(options?.force === !0) && !permissionsEndpointAvailableRef.current))
          try { const result = await api("/permissions", !0);
            ((permissionsEndpointAvailableRef.current = !0), setPermissionsEndpointAvailable(!0),
              setPermissionGrants(normalizePermissionPayload(result)), setPermissionAccessDenied(!1), clearTransientFeedback());
          } catch (error) { if (String(error?.message || error || "").includes("HTTP 403")) {
              ((permissionsEndpointAvailableRef.current = !0), setPermissionsEndpointAvailable(!0), setPermissionAccessDenied(!0), setPermissionGrants([]));
              return;
            }
            if (isRouteMissingError(error)) { ((permissionsEndpointAvailableRef.current = !1),
                setPermissionsEndpointAvailable(!1), setPermissionAccessDenied(!1), setPermissionGrants([]), clearTransientFeedback());
              return;
            }
            throw error;
          } }, [api, clearTransientFeedback], ), refreshSecondaryData = useCallback( async (options = {}) => {
        const force = options?.force === !0, wantsWorkStreams = panel === "work" || panel === "overview", wantsMemoryAdmin = panel === "memory", jobs = [];
        return ( wantsWorkStreams && jobs.push(refreshFeed, refreshMessages, refreshActivity),
          (wantsWorkStreams || wantsMemoryAdmin) && jobs.push(refreshConflicts), wantsMemoryAdmin && jobs.push(() => refreshPermissions({ force })),
          jobs.length ? settledCollectErrors(jobs) : [] );
      }, [panel, refreshFeed, refreshMessages, refreshActivity, refreshConflicts, refreshPermissions], ), refreshProtectedData = useCallback(
      async (options = {}) => { const includeSecondary = options?.includeSecondary !== !1,
          forceCore = options?.forceCore === !0, forceSecondary = options?.forceSecondary === !0,
          now = Date.now(), shouldRefreshCore = forceCore || now - lastCoreRefreshAtRef.current >= CORE_REFRESH_MIN_INTERVAL_MS;
        let coreErrors = [], coreSuccessCount = 0, coreTotalCount = 0;
        if (shouldRefreshCore) { const coreRefresh = await refreshCoreData({ throwOnError: !1 });
          ((coreErrors = coreRefresh.errors), (coreSuccessCount = coreRefresh.successCount),
            (coreTotalCount = coreRefresh.totalCount), coreErrors.length || (lastCoreRefreshAtRef.current = Date.now()));
        }
        if (coreErrors.length || !includeSecondary)
          return { coreErrors, secondaryErrors: [], coreSuccessCount, coreTotalCount, };
        if (!(forceSecondary || now - lastSecondaryRefreshAtRef.current >= SECONDARY_REFRESH_MIN_INTERVAL_MS))
          return { coreErrors: [], secondaryErrors: [], coreSuccessCount, coreTotalCount, };
        const secondaryErrors = await refreshSecondaryData({ force: forceSecondary, });
        return ( secondaryErrors.length || (lastSecondaryRefreshAtRef.current = Date.now()),
          { coreErrors: [], secondaryErrors, coreSuccessCount, coreTotalCount } );
      }, [refreshCoreData, refreshSecondaryData], ), refreshSecondaryDataInBackground = useCallback(() => {
      typeof window > "u" || startupSecondaryRefreshInFlightRef.current || ((startupSecondaryRefreshInFlightRef.current = !0), window.setTimeout(() => {
          (async () => { if (!daemonStateRef.current?.reachable) return;
            const secondaryErrors = await refreshSecondaryData({ force: !0 });
            if ( (secondaryErrors.length || (setDaemonTimeoutStaleSummary(""), (lastSecondaryRefreshAtRef.current = Date.now())),
              !secondaryErrors.length || !daemonStateRef.current?.reachable) )
              return;
            const timeoutErrors = secondaryErrors.filter((error) => isDaemonTimeoutErrorMessage(error));
            (timeoutErrors.length
              ? setDaemonTimeoutStaleSummary( summarizeDashboardErrors(timeoutErrors) || "IPC request timeouts detected.", )
              : setDaemonTimeoutStaleSummary(""), setSecondaryAvailabilityFeedback(secondaryErrors));
          })().finally(() => { startupSecondaryRefreshInFlightRef.current = !1;
          });
        }, 0));
    }, [refreshSecondaryData, setSecondaryAvailabilityFeedback]), refreshProtectedDataForStartup = useCallback(async () => {
      const includeSecondary = startupCoreReadyRef.current;
      let result = await refreshProtectedData({ includeSecondary, forceCore: !0, });
      return ( !includeSecondary && !result.coreErrors.length && ((startupCoreReadyRef.current = !0),
          setStartupCoreReadyState(!0), refreshSecondaryDataInBackground(), (result = { ...result, secondaryErrors: [] })), result );
    }, [refreshProtectedData, refreshSecondaryDataInBackground]), clearStartupCoreReady = useCallback(() => {
      ((startupCoreReadyRef.current = !1), setStartupCoreReadyState(!1), (lastCoreRefreshAtRef.current = 0), (lastSecondaryRefreshAtRef.current = 0));
    }, []), handleResolveConflict = useCallback(
      async (keepId, action, supersededId, pair = null) => { const resolver = selectedOperatorName ? `user:${selectedOperatorName}` : "user:control-center",
          resolutionBody = { keepId, action, supersededId, conflictId: pair?.conflictId || null, winnerId: action === "keep" ? keepId : null,
            loserId: action === "keep" ? supersededId : null, resolution: action, resolvedBy: resolver, };
        setConflictLoading(!0);
        try { try { await postApi("/conflicts/resolve", resolutionBody);
          } catch (primaryError) { if (!isRouteMissingError(primaryError)) throw primaryError;
            await postApi("/resolve", resolutionBody);
          }
          await refreshConflicts();
        } catch (err) { setFeedbackMessage(`Resolve failed: ${err.message || err}`);
        } finally { setConflictLoading(!1);
        } }, [postApi, refreshConflicts, selectedOperatorName], ),
    handleResolveDraftChange = useCallback((pairKey, updates) => { setResolveDrafts((current) => {
        const draft = current[pairKey] || { action: "keep", winner: "left" };
        return { ...current, [pairKey]: { ...draft, ...updates } };
      });
    }, []), handleGrantPermission = useCallback(async () => {
      if (!permissionsEndpointAvailable) { setFeedbackMessage("Permission endpoint unavailable on this daemon build.");
        return;
      }
      const client = String(permissionDraft.client || "").trim();
      if (!client) { setFeedbackMessage("Permission grant failed: client is required.");
        return;
      }
      setPermissionLoading(!0);
      try { (await postApi("/permissions/grant", { client, permission: permissionDraft.permission || "read",
          scope: String(permissionDraft.scope || "*").trim() || "*", grantedBy: selectedOperatorName ? `user:${selectedOperatorName}` : "user:control-center",
        }), setPermissionDraft((current) => ({ ...current, client: "" })), await refreshPermissions({ force: !0 }));
      } catch (err) { setFeedbackMessage(`Permission grant failed: ${err.message || err}`);
      } finally { setPermissionLoading(!1);
      }
    }, [permissionDraft, permissionsEndpointAvailable, postApi, refreshPermissions, selectedOperatorName]), handleRevokePermission = useCallback(
      async (grant) => { if (!permissionsEndpointAvailable) { setFeedbackMessage("Permission endpoint unavailable on this daemon build.");
          return;
        }
        if (!(!grant?.client || !grant?.permission)) { setPermissionLoading(!0);
          try { (await postApi("/permissions/revoke", { client: grant.client, permission: grant.permission,
              scope: grant.scope || "*", }), await refreshPermissions({ force: !0 }));
          } catch (err) { setFeedbackMessage(`Permission revoke failed: ${err.message || err}`);
          } finally { setPermissionLoading(!1);
          }
        } }, [permissionsEndpointAvailable, postApi, refreshPermissions], ), openEditorSetupWizard = useCallback( async (event) => {
        ((editorSetupTriggerRef.current = event?.currentTarget || document.activeElement), setIsSettingUpEditors(!0));
        try { const result = await call("detect_editors");
          (setEditorDetections(result), setSelectedEditorIds(result.filter((entry) => entry.detected).map((entry) => entry.id)), setShowEditorSetupWizard(!0));
          const detected = result.filter((entry) => entry.detected).length;
          setFeedbackMessage( detected
              ? `Setup MCP found ${detected} supported client(s). Review and apply the selections.`
              : "Setup MCP found no supported clients. Use the manual snippet for other MCP-capable tools.", );
        } catch (err) { setFeedbackMessage(`MCP setup scan: ${String(err)}`);
        } finally { setIsSettingUpEditors(!1);
        } }, [call], ), toggleEditorSelection = useCallback((editorId) => { setSelectedEditorIds((current) =>
        current.includes(editorId) ? current.filter((id) => id !== editorId) : [...current, editorId], );
    }, []), applyEditorSetup = useCallback(async () => {
      if (!selectedEditorIds.length) { setFeedbackMessage("Select at least one detected client before applying MCP setup.");
        return;
      }
      setIsSettingUpEditors(!0);
      try { const result = await call("setup_editors", { editorIds: selectedEditorIds, });
        (setEditorSetup(result), closeEditorSetupWizard());
        const detected = result.filter((entry) => entry.detected).length, registered = result.filter((entry) => entry.registered).length,
          failed = result.filter((entry) => entry.detected && !entry.registered).length;
        setFeedbackMessage( detected
            ? failed
              ? `Setup MCP finished with ${failed} issue(s). Review client details in Overview.`
              : `Setup MCP configured ${registered} client(s).`
            : "Setup MCP found no supported clients on this machine.", );
      } catch (err) { setFeedbackMessage(`Editor setup: ${String(err)}`);
      } finally { setIsSettingUpEditors(!1);
      }
    }, [call, closeEditorSetupWizard, selectedEditorIds]), updateBudgetDraftRoot = useCallback((patch) => {
      (setBudgetDraftDirty(!0), setBudgetConfigMessage(""),
        setBudgetDraft((current) => ({ ...(current?.endpoints ? current : createBudgetDraftFromStatus(null)), ...patch, })));
    }, []), updateBudgetEndpointDraft = useCallback((endpoint, patch) => { (setBudgetDraftDirty(!0), setBudgetConfigMessage(""),
        setBudgetDraft((current) => { const base = current?.endpoints ? current : createBudgetDraftFromStatus(null);
          return { ...base, endpoints: { ...base.endpoints, [endpoint]: { ...base.endpoints[endpoint], ...patch }, }, };
        }));
    }, []), reloadBudgetConfigDraft = useCallback( async ({ silent = !1 } = {}) => { if (!invokeRef.current) {
          silent || setBudgetConfigMessage("Budget editing requires the desktop app.");
          return;
        }
        ((budgetConfigLoadAttemptedRef.current = !0), setBudgetConfigBusy(!0));
        try { const status = await call("read_budget_config");
          (setBudgetConfigStatus(status), setHealthMeta((current) => ({ ...current, budgets: status })),
            setBudgetDraft(createBudgetDraftFromStatus(status)), setBudgetDraftDirty(!1),
            silent || setBudgetConfigMessage(status?.source ? `Loaded ${status.source}` : "Loaded budget config."));
        } catch (err) { setBudgetConfigMessage(`Budget load failed: ${err?.message || String(err)}`);
        } finally { setBudgetConfigBusy(!1);
        } }, [call], ), saveBudgetConfigDraft = useCallback( async (event) => {
        if ((event.preventDefault(), !invokeRef.current)) { setBudgetConfigMessage("Budget editing requires the desktop app.");
          return;
        }
        const validationError = validateBudgetDraft(budgetDraft);
        if (validationError) { setBudgetConfigMessage(validationError);
          return;
        }
        setBudgetConfigBusy(!0);
        try { const status = await call("save_budget_config", { draft: serializeBudgetDraftForSave(budgetDraft), });
          (setBudgetConfigStatus(status), setHealthMeta((current) => ({ ...current, budgets: status })),
            setBudgetDraft(createBudgetDraftFromStatus(status)), setBudgetDraftDirty(!1),
            setBudgetConfigMessage("Saved budgets.toml. Restart daemon to apply enforcement."), setFeedbackMessage("Budget config saved."));
        } catch (err) { setBudgetConfigMessage(`Budget save failed: ${err?.message || String(err)}`);
        } finally { setBudgetConfigBusy(!1);
        } }, [budgetDraft, call], );
  return { ...ctx, api, postApi, call, readAuthToken, refreshDaemonState, probeReadiness,
    refreshHealth, refreshCoreData, refreshFeed, refreshMessages, refreshActivity, refreshSavings, refreshConflicts, refreshPermissions,
    refreshSecondaryData, refreshProtectedData, refreshSecondaryDataInBackground, refreshProtectedDataForStartup,
    clearStartupCoreReady, handleResolveConflict, handleResolveDraftChange, handleGrantPermission,
    handleRevokePermission, openEditorSetupWizard, toggleEditorSelection, applyEditorSetup,
    updateBudgetDraftRoot, updateBudgetEndpointDraft, reloadBudgetConfigDraft, saveBudgetConfigDraft, };
}
export { useRefreshOrchestration };

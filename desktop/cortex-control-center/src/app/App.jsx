import { useEffect } from "react";
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

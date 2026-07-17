import React from "react";
import { useEffect } from "react";
import { DEFAULT_CORTEX_BASE } from "./constants.js";
import { persistBrowserAuthToken } from "./browser-bootstrap.js";
import { useDashboardHooks } from "./hooks/useDashboardHooks.js";
import { AppShell } from "./AppShell.jsx";
function App() {
  const dashboard = useDashboardHooks(),
    { refreshAllRef, runRefreshAll } = dashboard;
  return (
    useEffect(() => {
      refreshAllRef.current = runRefreshAll;
    }, [refreshAllRef, runRefreshAll]),
    React.createElement(AppShell, {
      ...dashboard,
      DEFAULT_CORTEX_BASE,
      persistBrowserAuthToken,
      refreshAllRef: dashboard.refreshAllRef,
    })
  );
}
export { App };

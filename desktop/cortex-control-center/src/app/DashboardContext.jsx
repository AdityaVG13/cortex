import { createContext, useContext } from "react";

const DashboardContext = createContext(null);

function DashboardProvider({ value, children }) {
  return <DashboardContext.Provider value={value}>{children}</DashboardContext.Provider>;
}

function useDashboard() {
  const value = useContext(DashboardContext);
  if (!value) throw new Error("useDashboard must be used inside DashboardProvider");
  return value;
}

export { DashboardProvider, useDashboard };

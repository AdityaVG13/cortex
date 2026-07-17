import { BrainVisualizerPanel } from "../components/BrainVisualizerPanel.jsx";
import { SettingsPanel } from "./SettingsPanel.jsx";
import { OverviewPanel } from "./OverviewPanel.jsx";
import { AgentsPanel } from "./AgentsPanel.jsx";
import { WorkPanel } from "./WorkPanel.jsx";
import { MemoryPanel } from "./MemoryPanel.jsx";
import { AnalyticsPanel } from "./AnalyticsPanel.jsx";
import { ConflictsPanel } from "./ConflictsPanel.jsx";
import { AboutPanel } from "./AboutPanel.jsx";
import { useDashboard } from "../DashboardContext.jsx";

function renderActivePanel(p) {
  switch (p.panel) {
    case "settings":
      return <SettingsPanel />;
    case "overview":
      return <OverviewPanel />;
    case "agents":
      return <AgentsPanel />;
    case "work":
      return <WorkPanel />;
    case "memory":
      return <MemoryPanel />;
    case "analytics":
      return <AnalyticsPanel />;
    case "brain":
      return <BrainVisualizerPanel />;
    case "conflicts":
      return <ConflictsPanel />;
    case "about":
      return <AboutPanel />;
    default:
      return null;
  }
}

export function PanelStage() {
  const dashboard = useDashboard();
  return (
    <div className="panel-stage" data-panel-direction={dashboard.panelMotionDirection}>
      {renderActivePanel(dashboard)}
    </div>
  );
}

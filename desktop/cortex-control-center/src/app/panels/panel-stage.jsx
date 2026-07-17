import { BrainVisualizerPanel } from "../components/BrainVisualizerPanel.jsx";
import { SettingsPanel } from "./SettingsPanel.jsx";
import { OverviewPanel } from "./OverviewPanel.jsx";
import { AgentsPanel } from "./AgentsPanel.jsx";
import { WorkPanel } from "./WorkPanel.jsx";
import { MemoryPanel } from "./MemoryPanel.jsx";
import { AnalyticsPanel } from "./AnalyticsPanel.jsx";
import { ConflictsPanel } from "./ConflictsPanel.jsx";
import { AboutPanel } from "./AboutPanel.jsx";

function renderActivePanel(p) {
  switch (p.panel) {
    case "settings":
      return <SettingsPanel {...p} />;
    case "overview":
      return <OverviewPanel {...p} />;
    case "agents":
      return <AgentsPanel {...p} />;
    case "work":
      return <WorkPanel {...p} />;
    case "memory":
      return <MemoryPanel {...p} />;
    case "analytics":
      return <AnalyticsPanel {...p} />;
    case "brain":
      return (
        <BrainVisualizerPanel
          brainPanelRef={p.brainPanelRef}
          panel={p.panel}
          brainPanelMounted={p.brainPanelMounted}
          api={p.api}
          cortexBase={p.cortexBase}
          authToken={p.tokenRef.current}
          effectiveReducedMotion={p.effectiveReducedMotion}
        />
      );
    case "conflicts":
      return <ConflictsPanel {...p} />;
    case "about":
      return <AboutPanel {...p} />;
    default:
      return null;
  }
}

export function PanelStage(p) {
  return (
    <div className="panel-stage" data-panel-direction={p.panelMotionDirection}>
      {renderActivePanel(p)}
    </div>
  );
}

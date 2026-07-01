import { AppIcon } from "../../ui-icons.jsx";
import { CURRENCY_OPTIONS, SAVINGS_OPERATION_LABELS, timeAgo } from "../../constants.js";
import { SAVINGS_USD_PER_MILLION, SAVINGS_HISTORY_DAYS, MISSION_METRIC_LEGEND, CONTROL_CENTER_VERSION, ANALYTICS_METRIC_LEGEND } from "../constants.js";
import { BUDGET_ENDPOINT_DEFINITIONS } from "../../settings/settings-state.js";
import { handleKeyboardActivation } from "../../keyboard-access.js";
import { sameAgent } from "../../live-surface.js";
import { normalizeCurrencyCode, formatDaemonEndpoint } from "../utils/format.js";
import { conflictBadgeClass } from "../normalize/conflicts.js";
import { agentColor } from "../utils/agent-color.js";
import { AnimatedNumber } from "../components/AnimatedNumber.jsx";
import { Sparkline } from "../components/Sparkline.jsx";
import { MonteCarloProjectionChart } from "../components/MonteCarloProjectionChart.jsx";
import { EmptyItem } from "../components/common.jsx";
import { AgentItem } from "../components/AgentItem.jsx";
import { OperatorSelector } from "../components/OperatorSelector.jsx";
import { TaskItem } from "../components/TaskItem.jsx";
import { LockItem } from "../components/LockItem.jsx";
import { FeedItem } from "../components/FeedItem.jsx";
import { MessageItem } from "../components/MessageItem.jsx";
import { ActivityItem } from "../components/ActivityItem.jsx";
import { ConflictPairCard } from "../components/ConflictPairCard.jsx";
import { PANEL_SEQUENCE } from "../constants.js";
import { BrainVisualizerPanel } from "../components/BrainVisualizerPanel.jsx";
import { SettingsPanel } from "./SettingsPanel.jsx";
import { OverviewPanel } from "./OverviewPanel.jsx";
import { AgentsPanel } from "./AgentsPanel.jsx";
import { WorkPanel } from "./WorkPanel.jsx";
import { MemoryPanel } from "./MemoryPanel.jsx";
import { AnalyticsPanel } from "./AnalyticsPanel.jsx";
import { ConflictsPanel } from "./ConflictsPanel.jsx";
import { AboutPanel } from "./AboutPanel.jsx";

export function PanelStage(p) {
  return (
    <div className="panel-stage" data-panel-direction={p.panelMotionDirection}>
      <SettingsPanel {...p} />
      <OverviewPanel {...p} />
      <AgentsPanel {...p} />
      <WorkPanel {...p} />
      <MemoryPanel {...p} />
      <AnalyticsPanel {...p} />
      <BrainVisualizerPanel
        brainPanelRef={p.brainPanelRef}
        panel={p.panel}
        brainPanelMounted={p.brainPanelMounted}
        api={p.api}
        cortexBase={p.cortexBase}
        authToken={p.tokenRef.current}
        effectiveReducedMotion={p.effectiveReducedMotion}
      />
      <ConflictsPanel {...p} />
      <AboutPanel {...p} />
    </div>
  );
}

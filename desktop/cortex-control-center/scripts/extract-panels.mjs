#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "src");
const bak = fs.readFileSync(path.join(SRC, "App.jsx.bak"), "utf8").split("\n");

function slice(start, end) {
  return bak.slice(start - 1, end).join("\n");
}

function write(rel, content) {
  fs.mkdirSync(path.dirname(path.join(SRC, rel)), { recursive: true });
  fs.writeFileSync(path.join(SRC, rel), content.endsWith("\n") ? content : `${content}\n`);
}

const HOOK_KEYS = fs.readFileSync(path.join(ROOT, "scripts/split-panels-and-hooks.mjs"), "utf8")
  .match(/const HOOK_RETURN_KEYS = `([\s\S]*?)`;/)[1];

const IMPORTS = `import { AppIcon } from "../../ui-icons.jsx";
import { CURRENCY_OPTIONS, SAVINGS_OPERATION_LABELS, SAVINGS_USD_PER_MILLION, SAVINGS_HISTORY_DAYS, timeAgo, MISSION_METRIC_LEGEND, CONTROL_CENTER_VERSION, ANALYTICS_METRIC_LEGEND } from "../../constants.js";
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
`;

function panel(name, start, end) {
  const body = slice(start, end);
  write(
    `app/panels/${name}.jsx`,
    `${IMPORTS}
export function ${name}(p) {
  const {
    ${HOOK_KEYS},
  } = p;

  return (
    <>
${body}
    </>
  );
}
`,
  );
}

panel("SettingsPanel", 4606, 4869);
panel("OverviewPanel", 4871, 5208);
panel("AgentsPanel", 5211, 5296);
panel("WorkPanel", 5298, 5581);
panel("MemoryPanel", 5583, 5838);
panel("AnalyticsPanel", 5846, 6307);
panel("ConflictsPanel", 6337, 6364);
panel("AboutPanel", 6366, 6465);

write(
  "app/panels/panel-stage.jsx",
  `${IMPORTS}import { BrainVisualizerPanel } from "../components/BrainVisualizerPanel.jsx";
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
`,
);

console.log("Panels extracted from backup");

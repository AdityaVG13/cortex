import React from "react";
import { timeAgo } from "../../constants.js";
import { agentColor } from "../utils/agent-color.js";
function AgentItem({ session }) {
  const color = agentColor(session.agent);
  return React.createElement(
    "li",
    null,
    React.createElement(
      "div",
      { className: "agent-row" },
      React.createElement("span", {
        className: "agent-indicator",
        style: { background: color, boxShadow: `0 0 8px ${color}` },
      }),
      React.createElement("span", { className: "item-name" }, session.agent),
      React.createElement(
        "span",
        { className: "agent-pulse", style: { color } },
        "ACTIVE",
      ),
    ),
    React.createElement(
      "div",
      { className: "item-detail" },
      session.description || "Working",
      " - ",
      session.project || "\u2014",
    ),
    React.createElement(
      "div",
      { className: "item-meta" },
      React.createElement(
        "span",
        { className: "mono-inline" },
        (session.files || [])
          .slice(0, 4)
          .map((file) =>
            React.createElement(
              "span",
              { key: file, className: "lock-path" },
              file,
            ),
          ),
      ),
      React.createElement(
        "span",
        { className: "muted-inline" },
        timeAgo(session.lastHeartbeat),
      ),
    ),
  );
}
export { AgentItem };

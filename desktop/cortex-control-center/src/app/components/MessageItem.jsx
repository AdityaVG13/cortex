import React from "react";
import { timeAgo } from "../../constants.js";
import { AppIcon } from "../../ui-icons.jsx";
import { agentColor } from "../utils/agent-color.js";
function MessageItem({ entry }) {
  const fromColor = agentColor(entry.from);
  return (
    <li className="msg-bubble">
      <div className="msg-header">
        <span className="msg-agent" style={{ color: fromColor }}>
          <span
            className="agent-indicator"
            style={{
              background: fromColor,
              boxShadow: `0 0 6px ${fromColor}`,
              display: "inline-block",
              width: 6,
              height: 6,
              borderRadius: "50%",
              marginRight: 6,
              verticalAlign: "middle",
            }}
          />
          {entry.from || "unknown"}
        </span>
        <span className="msg-arrow">
          <AppIcon name="outbound" />
        </span>
        <span className="msg-to">{entry.to || "unknown"}</span>
        <span className="muted-inline">{timeAgo(entry.timestamp)}</span>
      </div>
      <div className="msg-body">{entry.message || "(empty message)"}</div>
    </li>
  );
}
export { MessageItem };

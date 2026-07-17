import React from "react";
import { timeAgo } from "../../constants.js";
import { agentColor } from "../utils/agent-color.js";
function AgentItem({ session }) {
  const color = agentColor(session.agent);
  return (
    <li>
      <div className="agent-row">
        <span className="agent-indicator" style={{ background: color, boxShadow: `0 0 8px ${color}` }} />
        <span className="item-name">{session.agent}</span>
        <span className="agent-pulse" style={{ color }}>
          ACTIVE
        </span>
      </div>
      <div className="item-detail">
        {session.description || "Working"}
        {" - "}
        {session.project || "\u2014"}
      </div>
      <div className="item-meta">
        <span className="mono-inline">
          {(session.files || []).slice(0, 4).map((file) => (
            <span key={file} className="lock-path">
              {file}
            </span>
          ))}
        </span>
        <span className="muted-inline">{timeAgo(session.lastHeartbeat)}</span>
      </div>
    </li>
  );
}
export { AgentItem };

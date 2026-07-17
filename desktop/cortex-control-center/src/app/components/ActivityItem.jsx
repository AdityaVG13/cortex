import React from "react";
import { timeAgo } from "../../constants.js";
function ActivityItem({ entry }) {
  const files = Array.isArray(entry.files) ? entry.files.slice(0, 6) : [];
  return (
    <li>
      <div className="item-meta">
        <span className="item-name">{entry.agent || "unknown"}</span>
        <span className="muted-inline">{timeAgo(entry.timestamp)}</span>
      </div>
      <div className="feed-summary">{entry.description || "(no activity details)"}</div>
      {files.length ? (
        <div className="feed-files">
          {files.map((file) => (
            <span key={`${entry.id}-${file}`} className="lock-path">
              {file}
            </span>
          ))}
        </div>
      ) : null}
    </li>
  );
}
export { ActivityItem };

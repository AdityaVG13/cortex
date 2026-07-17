import React from "react";
import { timeAgo } from "../../constants.js";
import { feedKindLabel } from "../utils/format.js";
function FeedItem({ entry }) { const files = Array.isArray(entry.files) ? entry.files.slice(0, 6) : [], metaBits = [timeAgo(entry.timestamp)];
  return ( entry.priority && metaBits.push(entry.priority), typeof entry.tokens == "number" && metaBits.push(`${entry.tokens} tok`), ( <li>
        <div className="item-meta">
          <span className="feed-kind">{feedKindLabel(entry.kind)}</span>
          <span className="item-name">{entry.agent || "unknown"}</span>
          <span className="muted-inline">{metaBits.join(" - ")}</span>
        </div>
        <div className="feed-summary">{entry.summary || "(no summary)"}</div>
        {entry.taskId ? ( <div className="item-detail">
            {"task: "}
            {entry.taskId}
          </div>
        ) : null}
        {files.length ? ( <div className="feed-files">
            {files.map((file) => ( <span key={`${entry.id}-${file}`} className="lock-path">
                {file}
              </span>
            ))}
          </div>
        ) : null}
      </li> ) );
}
export { FeedItem };

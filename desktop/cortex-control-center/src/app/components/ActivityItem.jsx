import React from "react";
import { timeAgo } from "../../constants.js";
function ActivityItem({ entry }) {
  const files = Array.isArray(entry.files) ? entry.files.slice(0, 6) : [];
  return React.createElement(
    "li",
    null,
    React.createElement(
      "div",
      { className: "item-meta" },
      React.createElement(
        "span",
        { className: "item-name" },
        entry.agent || "unknown",
      ),
      React.createElement(
        "span",
        { className: "muted-inline" },
        timeAgo(entry.timestamp),
      ),
    ),
    React.createElement(
      "div",
      { className: "feed-summary" },
      entry.description || "(no activity details)",
    ),
    files.length
      ? React.createElement(
          "div",
          { className: "feed-files" },
          files.map((file) =>
            React.createElement(
              "span",
              { key: `${entry.id}-${file}`, className: "lock-path" },
              file,
            ),
          ),
        )
      : null,
  );
}
export { ActivityItem };

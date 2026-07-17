import React from "react";
import { EmptyItem } from "../components/common.jsx";
import { ConflictPairCard } from "../components/ConflictPairCard.jsx";
function ConflictsPanel(p) {
  const {
    panel,
    conflictPairs,
    resolveDrafts,
    conflictLoading,
    handleResolveConflict,
    handleResolveDraftChange,
    refreshConflicts,
    reportSurfaceError,
  } = p;
  return React.createElement(
    React.Fragment,
    null,
    panel === "conflicts"
      ? React.createElement(
          "section",
          { className: "panel active" },
          React.createElement(
            "div",
            { className: "panel-header" },
            React.createElement("h1", null, "Conflict Resolution"),
            React.createElement(
              "div",
              { className: "panel-header-actions" },
              React.createElement(
                "span",
                { className: "badge" },
                conflictPairs.length,
                " dispute",
                conflictPairs.length !== 1 ? "s" : "",
              ),
              React.createElement(
                "button",
                {
                  type: "button",
                  className: "btn-sm",
                  onClick: () => refreshConflicts().catch(reportSurfaceError),
                },
                "Refresh",
              ),
            ),
          ),
          conflictPairs.length === 0
            ? React.createElement(
                "div",
                { className: "card full" },
                React.createElement(
                  "ul",
                  null,
                  React.createElement(EmptyItem, {
                    text: "No active conflicts -- all decisions are in harmony",
                  }),
                ),
              )
            : conflictPairs.map((pair) =>
                React.createElement(ConflictPairCard, {
                  key: pair.key,
                  pair,
                  conflictLoading,
                  onResolveQuick: handleResolveConflict,
                  onResolveDraft: handleResolveConflict,
                  resolveDraft: resolveDrafts[pair.key],
                  onResolveDraftChange: handleResolveDraftChange,
                }),
              ),
        )
      : null,
  );
}
export { ConflictsPanel };

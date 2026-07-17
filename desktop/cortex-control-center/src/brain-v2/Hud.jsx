import React from "react";
function tierLabel(tier) {
  return tier === "decision"
    ? "DECISION"
    : tier === "cluster"
      ? "CLUSTER"
      : tier === "loose"
        ? "MEMORY"
        : "NODE";
}
function Hud({ hover, selected }) {
  return React.createElement(
    React.Fragment,
    null,
    hover && !selected
      ? React.createElement(
          "div",
          { className: "brain-v2-tooltip" },
          React.createElement(
            "div",
            { className: "brain-v2-tooltip-tier" },
            tierLabel(hover.tier),
          ),
          React.createElement(
            "div",
            { className: "brain-v2-tooltip-label" },
            hover.label,
          ),
        )
      : null,
    selected
      ? React.createElement(
          "div",
          {
            className: "brain-v2-detail",
            role: "region",
            "aria-label": "Selected brain node",
          },
          React.createElement(
            "div",
            { className: "brain-v2-detail-head" },
            React.createElement(
              "span",
              { className: "brain-v2-detail-tier" },
              tierLabel(selected.tier),
            ),
            React.createElement(
              "span",
              { className: "brain-v2-detail-id" },
              selected.id,
            ),
          ),
          React.createElement(
            "div",
            { className: "brain-v2-detail-label" },
            selected.label,
          ),
          React.createElement(
            "div",
            { className: "brain-v2-detail-grid" },
            React.createElement(
              "div",
              { className: "brain-v2-detail-row" },
              React.createElement(
                "span",
                { className: "brain-v2-detail-key" },
                "AGENT",
              ),
              React.createElement(
                "span",
                { className: "brain-v2-detail-val" },
                selected.agent || "\u2014",
              ),
            ),
            React.createElement(
              "div",
              { className: "brain-v2-detail-row" },
              React.createElement(
                "span",
                { className: "brain-v2-detail-key" },
                "TYPE",
              ),
              React.createElement(
                "span",
                { className: "brain-v2-detail-val" },
                selected.type || "\u2014",
              ),
            ),
            React.createElement(
              "div",
              { className: "brain-v2-detail-row" },
              React.createElement(
                "span",
                { className: "brain-v2-detail-key" },
                "TIER",
              ),
              React.createElement(
                "span",
                { className: "brain-v2-detail-val" },
                selected.tier,
              ),
            ),
            selected.tier === "cluster"
              ? React.createElement(
                  "div",
                  { className: "brain-v2-detail-row" },
                  React.createElement(
                    "span",
                    { className: "brain-v2-detail-key" },
                    "MEMBERS",
                  ),
                  React.createElement(
                    "span",
                    { className: "brain-v2-detail-val" },
                    selected.memberCount,
                  ),
                )
              : null,
            React.createElement(
              "div",
              { className: "brain-v2-detail-row" },
              React.createElement(
                "span",
                { className: "brain-v2-detail-key" },
                "RADIUS",
              ),
              React.createElement(
                "span",
                { className: "brain-v2-detail-val" },
                Math.round(selected.orbitRadius || 0),
                "u",
              ),
            ),
          ),
          React.createElement(
            "div",
            { className: "brain-v2-detail-footer" },
            "Press Escape or tap empty space to deselect",
          ),
        )
      : null,
  );
}
var Hud_default = Hud;
export { Hud, Hud_default as default };

import React from "react";
import { Component, lazy, Suspense } from "react";
import { AppIcon } from "../../ui-icons.jsx";
const LazyBrainVisualizer = lazy(() =>
  import("../../BrainVisualizer.jsx").then((module) => ({
    default: module.BrainVisualizer,
  })),
);
class BrainErrorBoundary extends Component {
  constructor(props) {
    (super(props), (this.state = { crashed: !1, error: "" }));
  }
  static getDerivedStateFromError(err) {
    return { crashed: !0, error: err?.message || "Unknown error" };
  }
  render() {
    return this.state.crashed
      ? React.createElement(
          "div",
          { className: "brain-loading" },
          React.createElement(
            "div",
            { className: "coming-icon" },
            React.createElement(AppIcon, { name: "brain", size: 48 }),
          ),
          React.createElement("p", null, "Brain visualizer crashed: ", this.state.error),
          React.createElement(
            "button",
            {
              className: "btn-sm btn-primary",
              onClick: () => this.setState({ crashed: !1 }),
              style: { marginTop: 12 },
            },
            "Retry",
          ),
        )
      : this.props.children;
  }
}
function BrainVisualizerPanel({
  brainPanelRef,
  panel,
  brainPanelMounted,
  api,
  cortexBase,
  authToken,
  effectiveReducedMotion,
}) {
  return brainPanelMounted
    ? React.createElement(
        "section",
        {
          ref: brainPanelRef,
          className: `panel brain-panel ${panel === "brain" ? "active" : "panel-hidden"}`,
          "aria-hidden": panel === "brain" ? void 0 : !0,
        },
        React.createElement(
          BrainErrorBoundary,
          null,
          React.createElement(
            Suspense,
            {
              fallback: React.createElement(
                "div",
                { className: "brain-loading" },
                React.createElement(
                  "div",
                  { className: "coming-icon" },
                  React.createElement(AppIcon, { name: "brain", size: 48 }),
                ),
                React.createElement("p", null, "Loading brain visualizer\u2026"),
              ),
            },
            React.createElement(LazyBrainVisualizer, {
              api,
              cortexBase,
              authToken,
              active: panel === "brain",
              reducedMotion: effectiveReducedMotion,
            }),
          ),
        ),
      )
    : null;
}
export { BrainVisualizerPanel };

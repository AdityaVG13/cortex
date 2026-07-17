import React from "react";
import { Component, memo, useState } from "react";
import { AppIcon } from "./ui-icons.jsx";
import { BrainV2 } from "./brain-v2/index.jsx";
class GraphErrorBoundary extends Component {
  constructor(props) {
    (super(props), (this.state = { hasError: !1, error: null }));
  }
  static getDerivedStateFromError(error) {
    return { hasError: !0, error: error.message };
  }
  render() {
    return this.state.hasError
      ? this.props.fallback ||
          React.createElement(
            "div",
            { className: "brain-loading" },
            React.createElement(
              "div",
              { className: "coming-icon" },
              React.createElement(AppIcon, { name: "brain", size: 48 }),
            ),
            React.createElement(
              "p",
              null,
              "3D renderer crashed: ",
              this.state.error,
            ),
            React.createElement(
              "p",
              { className: "brain-fallback-reason" },
              "Showing 2D fallback instead.",
            ),
          )
      : this.props.children;
  }
}
function hasWebGLSupport() {
  if (typeof document > "u") return !1;
  try {
    const canvas = document.createElement("canvas");
    return !!(
      canvas.getContext("webgl2") ||
      canvas.getContext("webgl") ||
      canvas.getContext("experimental-webgl")
    );
  } catch {
    return !1;
  }
}
function BrainVisualizerComponent({
  api = null,
  cortexBase = "http://127.0.0.1:7437",
  authToken = "",
  active = !0,
  reducedMotion = !1,
}) {
  const [webglAvailable] = useState(() => hasWebGLSupport());
  return webglAvailable
    ? React.createElement(
        "div",
        { className: "brain-container" },
        React.createElement(
          "div",
          { className: "brain-hud brain-hud-primary" },
          React.createElement(
            "div",
            { className: "brain-hud-copy" },
            React.createElement(
              "span",
              { className: "brain-mode" },
              "Neural topology",
            ),
            React.createElement(
              "strong",
              { className: "brain-title" },
              "Cortex Brain Map",
            ),
            React.createElement(
              "p",
              null,
              "Living constellation. Select satellites to inspect.",
            ),
          ),
        ),
        React.createElement(
          GraphErrorBoundary,
          null,
          React.createElement(BrainV2, {
            api,
            cortexBase,
            authToken,
            active,
            reducedMotion,
          }),
        ),
      )
    : React.createElement(
        "div",
        { className: "brain-container brain-fallback-container" },
        React.createElement(
          "div",
          { className: "brain-hud brain-hud-fallback" },
          React.createElement(
            "span",
            { className: "brain-fallback-reason" },
            "2D fallback: WebGL unavailable",
          ),
        ),
        React.createElement(
          "div",
          { className: "brain-loading" },
          React.createElement(
            "div",
            { className: "coming-icon" },
            React.createElement(AppIcon, { name: "brain", size: 48 }),
          ),
          React.createElement(
            "p",
            null,
            "WebGL is required for the Brain map.",
          ),
        ),
      );
}
BrainVisualizerComponent.displayName = "BrainVisualizer";
const BrainVisualizer = memo(BrainVisualizerComponent);
var BrainVisualizer_default = BrainVisualizer;
export { BrainVisualizer, BrainVisualizer_default as default };

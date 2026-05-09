import { Component, memo, useState } from "react";
import { AppIcon } from "./ui-icons.jsx";
import { BrainV2 } from "./brain-v2/index.jsx";

class GraphErrorBoundary extends Component {
  constructor(props) {
    super(props);
    this.state = { hasError: false, error: null };
  }
  static getDerivedStateFromError(error) {
    return { hasError: true, error: error.message };
  }
  render() {
    if (this.state.hasError) {
      return this.props.fallback || (
        <div className="brain-loading">
          <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
          <p>3D renderer crashed: {this.state.error}</p>
          <p className="brain-fallback-reason">Showing 2D fallback instead.</p>
        </div>
      );
    }
    return this.props.children;
  }
}

function hasWebGLSupport() {
  if (typeof document === "undefined") return false;
  try {
    const canvas = document.createElement("canvas");
    return Boolean(
      canvas.getContext("webgl2")
        || canvas.getContext("webgl")
        || canvas.getContext("experimental-webgl")
    );
  } catch {
    return false;
  }
}

function BrainVisualizerComponent({ api = null, cortexBase = "http://127.0.0.1:7437", authToken = "", active = true }) {
  const [webglAvailable] = useState(() => hasWebGLSupport());

  if (!webglAvailable) {
    return (
      <div className="brain-container brain-fallback-container">
        <div className="brain-hud brain-hud-fallback">
          <span className="brain-fallback-reason">2D fallback: WebGL unavailable</span>
        </div>
        <div className="brain-loading">
          <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
          <p>WebGL is required for the Brain map.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="brain-container">
      <div className="brain-hud brain-hud-primary">
        <div className="brain-hud-copy">
          <span className="brain-mode">Neural topology</span>
          <strong className="brain-title">Cortex Brain Map</strong>
          <p>Living constellation. Click satellites to inspect.</p>
        </div>
      </div>
      <GraphErrorBoundary>
        <BrainV2 api={api} cortexBase={cortexBase} authToken={authToken} active={active} />
      </GraphErrorBoundary>
    </div>
  );
}

BrainVisualizerComponent.displayName = "BrainVisualizer";
export const BrainVisualizer = memo(BrainVisualizerComponent);
export default BrainVisualizer;

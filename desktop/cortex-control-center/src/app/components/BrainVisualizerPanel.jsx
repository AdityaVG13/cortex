import { Component, lazy, Suspense } from "react";
import { AppIcon } from "../../ui-icons.jsx";

const LazyBrainVisualizer = lazy(() =>
  import("../../BrainVisualizer.jsx").then((module) => ({ default: module.BrainVisualizer })),
);

class BrainErrorBoundary extends Component {
  constructor(props) { super(props); this.state = { crashed: false, error: "" }; }
  static getDerivedStateFromError(err) { return { crashed: true, error: err?.message || "Unknown error" }; }
  render() {
    if (this.state.crashed) return (
      <div className="brain-loading">
        <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
        <p>Brain visualizer crashed: {this.state.error}</p>
        <button className="btn-sm btn-primary" onClick={() => this.setState({ crashed: false })} style={{ marginTop: 12 }}>Retry</button>
      </div>
    );
    return this.props.children;
  }
}

export function BrainVisualizerPanel({ brainPanelRef, panel, brainPanelMounted, api, cortexBase, authToken, effectiveReducedMotion }) {
  if (!brainPanelMounted) return null;
  return (
    <section
      ref={brainPanelRef}
      className={`panel brain-panel ${panel === "brain" ? "active" : "panel-hidden"}`}
      aria-hidden={panel === "brain" ? undefined : true}
    >
      <BrainErrorBoundary>
        <Suspense
          fallback={(
            <div className="brain-loading">
              <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
              <p>Loading brain visualizer…</p>
            </div>
          )}
        >
          <LazyBrainVisualizer
            api={api}
            cortexBase={cortexBase}
            authToken={authToken}
            active={panel === "brain"}
            reducedMotion={effectiveReducedMotion}
          />
        </Suspense>
      </BrainErrorBoundary>
    </section>
  );
}

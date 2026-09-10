import React from "react";
import { brainHealthRows, nextCaptureAction } from "../brain-health.js";

function BrainHealthStrip({ brainHealth, onCapturePolicy, onReflexRebuild }) {
  const rows = brainHealthRows(brainHealth);
  const action = nextCaptureAction(brainHealth?.captureScope?.state);
  return (
    <div className="brain-health-strip" data-testid="brain-health-strip">
      {rows.map(([label, value, tone]) => (
        <div key={label} className={`sys-item brain-health-item tone-${tone}`} title={`${label}: ${value}`}>
          <span className="sys-label">{label}</span>
          <span className={`sys-value ${tone === "ok" ? "sys-ok" : ""}`}>{value}</span>
        </div>
      ))}
      {onCapturePolicy ? (
        <button type="button" className="sys-item sys-item-action" onClick={() => onCapturePolicy(action.state)} title="Capture scope control: pause keeps deliveries, stop halts automatic delivery too">
          <span className="sys-label">CAPTURE</span>
          <span className="sys-value">{action.label.toUpperCase()}</span>
        </button>
      ) : null}
      {onReflexRebuild ? (
        <button type="button" className="sys-item sys-item-action" onClick={onReflexRebuild} title="Rebuild the warm Reflex snapshot (derived, discardable)">
          <span className="sys-label">REFLEX</span>
          <span className="sys-value">REBUILD</span>
        </button>
      ) : null}
    </div>
  );
}

export { BrainHealthStrip };

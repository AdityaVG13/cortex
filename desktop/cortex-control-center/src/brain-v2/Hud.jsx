import React from "react";
function tierLabel(tier) {
  return tier === "decision" ? "DECISION" : tier === "cluster" ? "CLUSTER" : tier === "loose" ? "MEMORY" : "NODE";
}
function Hud({ hover, selected }) {
  return (
    <React.Fragment>
      {hover && !selected ? (
        <div className="brain-v2-tooltip">
          <div className="brain-v2-tooltip-tier">{tierLabel(hover.tier)}</div>
          <div className="brain-v2-tooltip-label">{hover.label}</div>
        </div>
      ) : null}
      {selected ? (
        <div className="brain-v2-detail" role="region" aria-label="Selected brain node">
          <div className="brain-v2-detail-head">
            <span className="brain-v2-detail-tier">{tierLabel(selected.tier)}</span>
            <span className="brain-v2-detail-id">{selected.id}</span>
          </div>
          <div className="brain-v2-detail-label">{selected.label}</div>
          <div className="brain-v2-detail-grid">
            <div className="brain-v2-detail-row">
              <span className="brain-v2-detail-key">AGENT</span>
              <span className="brain-v2-detail-val">{selected.agent || "\u2014"}</span>
            </div>
            <div className="brain-v2-detail-row">
              <span className="brain-v2-detail-key">TYPE</span>
              <span className="brain-v2-detail-val">{selected.type || "\u2014"}</span>
            </div>
            <div className="brain-v2-detail-row">
              <span className="brain-v2-detail-key">TIER</span>
              <span className="brain-v2-detail-val">{selected.tier}</span>
            </div>
            {selected.tier === "cluster" ? (
              <div className="brain-v2-detail-row">
                <span className="brain-v2-detail-key">MEMBERS</span>
                <span className="brain-v2-detail-val">{selected.memberCount}</span>
              </div>
            ) : null}
            <div className="brain-v2-detail-row">
              <span className="brain-v2-detail-key">RADIUS</span>
              <span className="brain-v2-detail-val">{Math.round(selected.orbitRadius || 0)}u</span>
            </div>
          </div>
          <div className="brain-v2-detail-footer">Press Escape or tap empty space to deselect</div>
        </div>
      ) : null}
    </React.Fragment>
  );
}
var Hud_default = Hud;
export { Hud, Hud_default as default };

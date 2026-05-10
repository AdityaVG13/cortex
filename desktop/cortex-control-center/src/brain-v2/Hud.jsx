function tierLabel(tier) {
  if (tier === "decision") return "DECISION";
  if (tier === "cluster") return "CLUSTER";
  if (tier === "loose") return "MEMORY";
  return "NODE";
}

export function Hud({ hover, selected }) {
  return (
    <>
      {hover && !selected ? (
        <div className="brain-v2-tooltip">
          <div className="brain-v2-tooltip-tier">{tierLabel(hover.tier)}</div>
          <div className="brain-v2-tooltip-label">{hover.label}</div>
        </div>
      ) : null}
      {selected ? (
        <div className="brain-v2-detail" role="dialog" aria-label="Selected node">
          <div className="brain-v2-detail-head">
            <span className="brain-v2-detail-tier">{tierLabel(selected.tier)}</span>
            <span className="brain-v2-detail-id">{selected.id}</span>
          </div>
          <div className="brain-v2-detail-label">{selected.label}</div>
          <div className="brain-v2-detail-grid">
            <div className="brain-v2-detail-row">
              <span className="brain-v2-detail-key">AGENT</span>
              <span className="brain-v2-detail-val">{selected.agent || "—"}</span>
            </div>
            <div className="brain-v2-detail-row">
              <span className="brain-v2-detail-key">TYPE</span>
              <span className="brain-v2-detail-val">{selected.type || "—"}</span>
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
          <div className="brain-v2-detail-footer">right-click to deselect</div>
        </div>
      ) : null}
    </>
  );
}

export default Hud;

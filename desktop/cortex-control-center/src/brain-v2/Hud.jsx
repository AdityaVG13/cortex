import { useEffect, useImperativeHandle, useRef, useState, forwardRef } from "react";

const TICKER_MAX = 5;
const TICKER_TTL_MS = 6_000;

export const Hud = forwardRef(function Hud({ stats }, ref) {
  const [entries, setEntries] = useState([]);
  const [hover, setHover] = useState(null);
  const [selected, setSelected] = useState(null);
  const queueRef = useRef([]);
  const rafRef = useRef(null);

  useImperativeHandle(ref, () => ({
    pushFiringEntry: (label) => {
      queueRef.current.push({ id: `${performance.now()}-${Math.random()}`, label, ts: performance.now() });
      if (rafRef.current != null) return;
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        const next = queueRef.current.splice(0, queueRef.current.length);
        if (!next.length) return;
        setEntries(prev => [...next, ...prev].slice(0, TICKER_MAX));
      });
    },
    setHover: (slot) => setHover(slot),
    setSelected: (slot) => setSelected(slot),
  }), []);

  useEffect(() => {
    if (!entries.length) return undefined;
    const handle = setInterval(() => {
      const now = performance.now();
      setEntries(prev => prev.filter(e => now - e.ts < TICKER_TTL_MS));
    }, 1_000);
    return () => clearInterval(handle);
  }, [entries.length]);

  return (
    <>
      <div className="brain-v2-hud-strip">
        <span className="brain-v2-hud-stat"><span className="brain-v2-hud-label">NODES</span>{stats?.nodes ?? 0}</span>
        <span className="brain-v2-hud-stat"><span className="brain-v2-hud-label">CLUSTERS</span>{stats?.clusters ?? 0}</span>
        <span className="brain-v2-hud-stat"><span className="brain-v2-hud-label">DECISIONS</span>{stats?.decisions ?? 0}</span>
        <span className="brain-v2-hud-stat"><span className="brain-v2-hud-label">FIRING</span>{stats?.activeBeams ?? 0}</span>
      </div>
      <div className="brain-v2-ticker" aria-hidden="true">
        {entries.map(entry => (
          <div key={entry.id} className="brain-v2-ticker-line">{entry.label}</div>
        ))}
      </div>
      {hover && !selected ? (
        <div className="brain-v2-tooltip">
          <div className="brain-v2-tooltip-tier">{hover.tier}</div>
          <div className="brain-v2-tooltip-label">{hover.label}</div>
          {hover.tier === "cluster" ? (
            <div className="brain-v2-tooltip-meta">{hover.memberCount} members</div>
          ) : null}
        </div>
      ) : null}
      {selected ? (
        <div className="brain-v2-detail">
          <div className="brain-v2-detail-tier">{selected.tier}</div>
          <div className="brain-v2-detail-label">{selected.label}</div>
          <div className="brain-v2-detail-meta">
            <span>agent: {selected.agent}</span>
            {selected.tier === "cluster" ? <span>members: {selected.memberCount}</span> : null}
          </div>
        </div>
      ) : null}
    </>
  );
});

export default Hud;

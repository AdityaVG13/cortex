import { useEffect, useRef, useState } from "react";
import { createScene } from "./Scene.js";
import { createCore, tickCore, disposeCore } from "./Core.js";
import { createSatellites } from "./Satellites.js";
import { createBeams } from "./Beams.js";
import { buildTiers } from "./Tiers.js";

export function BrainV2({ api = null, active = true }) {
  const containerRef = useRef(null);
  const sceneRef = useRef(null);
  const coreRef = useRef(null);
  const satellitesRef = useRef(null);
  const beamsRef = useRef(null);
  const [dimensions, setDimensions] = useState({
    width: Math.max(window.innerWidth - 260, 400),
    height: Math.max(window.innerHeight - 20, 300),
  });
  const [tiers, setTiers] = useState({ decisions: [], clusters: [], looseMemories: [], coldStart: false });
  const [error, setError] = useState(null);

  useEffect(() => {
    if (!active) return undefined;
    function onResize() {
      setDimensions({
        width: Math.max(window.innerWidth - 260, 400),
        height: Math.max(window.innerHeight - 20, 300),
      });
    }
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [active]);

  useEffect(() => {
    if (!active || !containerRef.current) return undefined;
    const sceneHandle = createScene({
      container: containerRef.current,
      width: dimensions.width,
      height: dimensions.height,
    });
    sceneRef.current = sceneHandle;
    sceneHandle.scene._camera = sceneHandle.camera;

    const core = createCore();
    coreRef.current = core;
    sceneHandle.scene.add(core);

    const satellites = createSatellites({ scene: sceneHandle.scene });
    satellitesRef.current = satellites;

    const beams = createBeams({ scene: sceneHandle.scene });
    beamsRef.current = beams;

    if (typeof window !== "undefined") {
      window.__brainFire = (fromId, toId, color) => {
        const sats = satellitesRef.current;
        if (!sats) return;
        const a = sats.getSlotById(fromId);
        const b = sats.getSlotById(toId) || { x: 0, y: 0, z: 0 };
        if (!a) return;
        beamsRef.current?.fire({ from: a, to: b, color: color || "#22d3ee" });
        sats.pulseSlot(fromId);
      };
    }

    const unregister = sceneHandle.registerTick((t, now) => {
      tickCore(core, t, now);
      satellites.tick(t, now);
      beams.tick(now);
    });

    return () => {
      unregister();
      if (typeof window !== "undefined" && window.__brainFire) {
        delete window.__brainFire;
      }
      if (beamsRef.current) {
        beamsRef.current.dispose();
        beamsRef.current = null;
      }
      if (satellitesRef.current) {
        satellitesRef.current.dispose();
        satellitesRef.current = null;
      }
      if (coreRef.current) {
        sceneHandle.scene.remove(coreRef.current);
        disposeCore(coreRef.current);
        coreRef.current = null;
      }
      sceneHandle.dispose();
      sceneRef.current = null;
    };
  }, [active]);

  useEffect(() => {
    if (!sceneRef.current) return;
    sceneRef.current.resize(dimensions.width, dimensions.height);
  }, [dimensions.width, dimensions.height]);

  useEffect(() => {
    if (!active) return undefined;
    let cancelled = false;
    async function load() {
      if (typeof api !== "function") return;
      try {
        const dump = await api("/dump", true);
        if (cancelled || !dump) return;
        const next = buildTiers(dump);
        setTiers(next);
      } catch (err) {
        if (!cancelled) setError(err?.message || String(err));
      }
    }
    load();
    return () => { cancelled = true; };
  }, [active, api]);

  useEffect(() => {
    if (!satellitesRef.current) return;
    satellitesRef.current.setData(tiers);
  }, [tiers]);

  return (
    <div
      ref={containerRef}
      className="brain-v2-container"
      style={{
        position: "relative",
        width: dimensions.width,
        height: dimensions.height,
        background: "#040812",
        overflow: "hidden",
      }}
    >
      {error ? (
        <div className="brain-v2-error" style={{ position: "absolute", top: 12, right: 12, color: "#ff8a8a" }}>
          {error}
        </div>
      ) : null}
    </div>
  );
}

export default BrainV2;

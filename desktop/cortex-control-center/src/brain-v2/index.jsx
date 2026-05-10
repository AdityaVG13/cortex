import { useEffect, useRef, useState } from "react";
import { createScene } from "./Scene.js";
import { createCore, tickCore, disposeCore, pulseCoreHalo } from "./Core.js";
import { createSatellites } from "./Satellites.js";
import { createBeams } from "./Beams.js";
import { buildTiers } from "./Tiers.js";
import { createFiringClient } from "./FiringClient.js";
import { createIdleSimulator } from "./IdleSimulator.js";
import { createEventDispatcher } from "./EventDispatcher.js";
import { createHover } from "./Hover.js";
import { createCamera } from "./Camera.js";
import { Hud } from "./Hud.jsx";

export function BrainV2({ api = null, cortexBase = "http://127.0.0.1:7437", authToken = "", active = true }) {
  const containerRef = useRef(null);
  const sceneRef = useRef(null);
  const coreRef = useRef(null);
  const satellitesRef = useRef(null);
  const beamsRef = useRef(null);
  const firingClientRef = useRef(null);
  const idleSimRef = useRef(null);
  const dispatcherRef = useRef(null);
  const hoverRef = useRef(null);
  const cameraHandleRef = useRef(null);
  const slotsAccessor = useRef([]);
  const hudRef = useRef(null);
  const hoveredSlotRef = useRef(null);
  const selectedSlotRef = useRef(null);
  const [dimensions, setDimensions] = useState({
    width: Math.max(window.innerWidth - 260, 400),
    height: Math.max(window.innerHeight - 20, 300),
  });
  const [tiers, setTiers] = useState({ decisions: [], clusters: [], looseMemories: [], coldStart: false });
  const [error, setError] = useState(null);
  const [stats, setStats] = useState({ nodes: 0, clusters: 0, decisions: 0, activeBeams: 0 });

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

    const cameraHandle = createCamera({
      camera: sceneHandle.camera,
      controls: sceneHandle.controls,
    });
    cameraHandleRef.current = cameraHandle;

    sceneHandle.controls.addEventListener("start", cameraHandle.pauseAutoRotate);

    const hover = createHover({
      camera: sceneHandle.camera,
      instancedMesh: satellites.bodies,
      slotsRef: slotsAccessor,
      onHoverChange: (slot) => {
        hoveredSlotRef.current = slot;
        hudRef.current?.setHover?.(slot);
      },
    });
    hoverRef.current = hover;

    const dispatcher = createEventDispatcher({
      satellites,
      beams,
      core,
      pulseCoreHalo: () => pulseCoreHalo(core),
      onTickerEntry: (label) => hudRef.current?.pushFiringEntry?.(label),
      onSpotlight: (slot) => slot && cameraHandle.spotlight(slot),
    });
    dispatcherRef.current = dispatcher;

    const idleSim = createIdleSimulator({
      onFake: (slotId) => dispatcher.dispatchFake(slotId),
      getNodeIds: () => satellitesRef.current?.getAllIds() || [],
    });
    idleSimRef.current = idleSim;

    if (authToken) {
      firingClientRef.current = createFiringClient({
        baseUrl: cortexBase,
        token: authToken,
        onEvent: (event) => {
          idleSim.noteRealEvent();
          dispatcher.dispatch(event);
        },
      });
    }

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

    let frame = 0;
    const unregister = sceneHandle.registerTick((t, now) => {
      tickCore(core, t, now);
      satellites.tick(t, now);
      beams.tick(now);
      cameraHandle.tick(now);
      hover.tick();
      frame += 1;
      if ((frame & 31) === 0) {
        const next = {
          nodes: satellites.getAllIds().length,
          clusters: 0,
          decisions: 0,
          activeBeams: beams.activeCount(),
        };
        const slots = slotsAccessor.current || [];
        for (const slot of slots) {
          if (slot.tier === "cluster") next.clusters += 1;
          else if (slot.tier === "decision") next.decisions += 1;
        }
        setStats(next);
      }
    });

    return () => {
      unregister();
      if (typeof window !== "undefined" && window.__brainFire) {
        delete window.__brainFire;
      }
      sceneHandle.controls.removeEventListener("start", cameraHandle.pauseAutoRotate);
      if (firingClientRef.current) {
        firingClientRef.current.disconnect();
        firingClientRef.current = null;
      }
      if (idleSimRef.current) {
        idleSimRef.current.dispose();
        idleSimRef.current = null;
      }
      hoverRef.current = null;
      cameraHandleRef.current = null;
      dispatcherRef.current = null;
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
    const flat = [];
    for (const d of tiers.decisions || []) flat.push(d);
    for (const c of tiers.clusters || []) flat.push(c);
    for (const m of tiers.looseMemories || []) flat.push(m);
    slotsAccessor.current = flat;
  }, [tiers]);

  function handlePointerMove(e) {
    if (!hoverRef.current || !containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    hoverRef.current.setCursor(e.clientX, e.clientY, rect);
  }

  function handlePointerLeave() {
    hoverRef.current?.clearCursor();
  }

  function handleClick(e) {
    if (e.button === 2) return;
    if (!hoverRef.current || !satellitesRef.current || !containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    hoverRef.current.setCursor(e.clientX, e.clientY, rect);
    hoverRef.current.tick();
    const slot = hoveredSlotRef.current;
    if (!slot) {
      satellitesRef.current.setSelected(null);
      hudRef.current?.setSelected?.(null);
      selectedSlotRef.current = null;
      return;
    }
    if (selectedSlotRef.current?.id === slot.id) {
      satellitesRef.current.setSelected(null);
      hudRef.current?.setSelected?.(null);
      selectedSlotRef.current = null;
      return;
    }
    satellitesRef.current.setSelected(slot.id);
    hudRef.current?.setSelected?.(slot);
    selectedSlotRef.current = slot;
    cameraHandleRef.current?.spotlight?.(slot);
  }

  function handleContextMenu(e) {
    e.preventDefault();
    satellitesRef.current?.setSelected(null);
    hudRef.current?.setSelected?.(null);
    selectedSlotRef.current = null;
  }

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
      onPointerMove={handlePointerMove}
      onPointerLeave={handlePointerLeave}
      onClick={handleClick}
      onContextMenu={handleContextMenu}
    >
      {error ? (
        <div className="brain-v2-error">
          {error}
        </div>
      ) : null}
      <Hud ref={hudRef} stats={stats} />
    </div>
  );
}

export default BrainV2;

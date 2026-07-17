import React from "react";
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
import { CAMERA_SPOTLIGHT_DURATION_MS, createCamera } from "./Camera.js";
import { Hud } from "./Hud.jsx";
import {
  brainKeyboardHelpText,
  isBrainNavigationKey,
  nextBrainNodeIndex,
} from "./Keyboard.js";
import { brainBudgetForQuality, detectBrainQualityTier } from "./Quality.js";
const TICKER_MAX = 5;
function BrainV2({
  api = null,
  cortexBase = "http://127.0.0.1:7437",
  authToken = "",
  active = !0,
  reducedMotion = !1,
}) {
  const containerRef = useRef(null),
    sceneRef = useRef(null),
    coreRef = useRef(null),
    satellitesRef = useRef(null),
    beamsRef = useRef(null),
    firingClientRef = useRef(null),
    idleSimRef = useRef(null),
    dispatcherRef = useRef(null),
    hoverRef = useRef(null),
    cameraHandleRef = useRef(null),
    spotlightReturnTimerRef = useRef(null),
    slotsAccessor = useRef([]),
    hoveredSlotRef = useRef(null),
    selectedSlotRef = useRef(null),
    statRefs = useRef({
      nodes: null,
      clusters: null,
      decisions: null,
      beams: null,
    }),
    lastStatsRef = useRef({
      nodes: 0,
      clusters: 0,
      decisions: 0,
      activeBeams: 0,
    }),
    lastStatsAtRef = useRef(0),
    tickerRef = useRef(null),
    tickerEntriesRef = useRef([]),
    [quality] = useState(() => detectBrainQualityTier()),
    [dimensions, setDimensions] = useState({
      width: Math.max(window.innerWidth - 260, 400),
      height: Math.max(window.innerHeight - 20, 300),
    }),
    [tiers, setTiers] = useState({
      decisions: [],
      clusters: [],
      looseMemories: [],
      coldStart: !1,
    }),
    [error, setError] = useState(null),
    [hoverSlot, setHoverSlot] = useState(null),
    [selectedSlot, setSelectedSlot] = useState(null),
    [keyboardSlotIndex, setKeyboardSlotIndex] = useState(-1);
  (useEffect(() => {
    if (!active) return;
    function onResize() {
      setDimensions({
        width: Math.max(window.innerWidth - 260, 400),
        height: Math.max(window.innerHeight - 20, 300),
      });
    }
    return (
      window.addEventListener("resize", onResize),
      () => window.removeEventListener("resize", onResize)
    );
  }, [active]),
    useEffect(() => {
      if (!active || !containerRef.current) return;
      const sceneHandle = createScene({
        container: containerRef.current,
        width: dimensions.width,
        height: dimensions.height,
        animated: !reducedMotion,
        pixelRatio: quality.pixelRatio,
      });
      ((sceneRef.current = sceneHandle),
        (sceneHandle.scene._camera = sceneHandle.camera));
      const core = createCore();
      ((coreRef.current = core), sceneHandle.scene.add(core));
      const satellites = createSatellites({
        scene: sceneHandle.scene,
        slotBudget: quality.nodeBudget.total,
      });
      satellitesRef.current = satellites;
      const beams = createBeams({ scene: sceneHandle.scene });
      beamsRef.current = beams;
      const cameraHandle = createCamera({
        camera: sceneHandle.camera,
        controls: sceneHandle.controls,
        autoRotate: !reducedMotion,
      });
      ((cameraHandleRef.current = cameraHandle),
        sceneHandle.controls.addEventListener(
          "start",
          cameraHandle.pauseAutoRotate,
        ));
      const hover = createHover({
        camera: sceneHandle.camera,
        slotsRef: slotsAccessor,
        hitRadiusScale: quality.hitRadiusScale,
        onHoverChange: (slot) => {
          ((hoveredSlotRef.current = slot), setHoverSlot(slot));
        },
      });
      hoverRef.current = hover;
      function pushTickerEntry(label) {
        const entry = {
          id: `${performance.now()}-${Math.random()}`,
          label,
          ts: performance.now(),
        };
        ((tickerEntriesRef.current = [entry, ...tickerEntriesRef.current].slice(
          0,
          TICKER_MAX,
        )),
          tickerRef.current &&
            renderTicker(tickerRef.current, tickerEntriesRef.current));
      }
      const dispatcher = createEventDispatcher({
        satellites,
        beams,
        core,
        pulseCoreHalo: () => {
          reducedMotion || pulseCoreHalo(core);
        },
        onTickerEntry: pushTickerEntry,
        onSpotlight: (slot) => {
          if (reducedMotion || !slot) return;
          const cameraHandle2 = cameraHandleRef.current;
          cameraHandle2 &&
            (cameraHandle2.spotlight(slot),
            spotlightReturnTimerRef.current &&
              window.clearTimeout(spotlightReturnTimerRef.current),
            (spotlightReturnTimerRef.current = window.setTimeout(() => {
              (cameraHandleRef.current?.returnToOrigin(),
                (spotlightReturnTimerRef.current = null));
            }, CAMERA_SPOTLIGHT_DURATION_MS)));
        },
      });
      dispatcherRef.current = dispatcher;
      const idleSim =
        !reducedMotion && quality.idleFiring
          ? createIdleSimulator({
              onFake: (slotId) => dispatcher.dispatchFake(slotId),
              getNodeIds: () => satellitesRef.current?.getAllIds() || [],
            })
          : null;
      ((idleSimRef.current = idleSim),
        !reducedMotion &&
          authToken &&
          (firingClientRef.current = createFiringClient({
            baseUrl: cortexBase,
            token: authToken,
            onEvent: (event) => {
              (idleSim?.noteRealEvent(), dispatcher.dispatch(event));
            },
          })),
        typeof window < "u" &&
          (window.__brainFire = (fromId, toId, color) => {
            if (reducedMotion) return;
            const sats = satellitesRef.current;
            if (!sats) return;
            const a = sats.getSlotById(fromId),
              b = sats.getSlotById(toId) || { x: 0, y: 0, z: 0 };
            a &&
              (beamsRef.current?.fire({
                from: a,
                to: b,
                color: color || "#22d3ee",
              }),
              sats.pulseSlot(fromId));
          }));
      const unregister = sceneHandle.registerTick((t, now) => {
        if (
          (reducedMotion ||
            (tickCore(core, t, now),
            satellites.tick(t, now),
            beams.tick(now),
            cameraHandle.tick(now)),
          hover.tick(),
          now - lastStatsAtRef.current >= 1e3)
        ) {
          lastStatsAtRef.current = now;
          const next = brainStatsForSlots(
              slotsAccessor.current || [],
              beams.activeCount(),
            ),
            prev = lastStatsRef.current;
          (next.nodes !== prev.nodes ||
            next.clusters !== prev.clusters ||
            next.decisions !== prev.decisions ||
            next.activeBeams !== prev.activeBeams) &&
            ((lastStatsRef.current = next), writeStats(statRefs.current, next));
        }
      });
      return () => {
        (unregister(),
          typeof window < "u" &&
            window.__brainFire &&
            delete window.__brainFire,
          spotlightReturnTimerRef.current &&
            (window.clearTimeout(spotlightReturnTimerRef.current),
            (spotlightReturnTimerRef.current = null)),
          sceneHandle.controls.removeEventListener(
            "start",
            cameraHandle.pauseAutoRotate,
          ),
          firingClientRef.current &&
            (firingClientRef.current.disconnect(),
            (firingClientRef.current = null)),
          idleSimRef.current &&
            (idleSimRef.current.dispose(), (idleSimRef.current = null)),
          (hoverRef.current = null),
          (cameraHandleRef.current = null),
          (dispatcherRef.current = null),
          beamsRef.current &&
            (beamsRef.current.dispose(), (beamsRef.current = null)),
          satellitesRef.current &&
            (satellitesRef.current.dispose(), (satellitesRef.current = null)),
          coreRef.current &&
            (sceneHandle.scene.remove(coreRef.current),
            disposeCore(coreRef.current),
            (coreRef.current = null)),
          sceneHandle.dispose(),
          (sceneRef.current = null));
      };
    }, [active, reducedMotion, quality]),
    useEffect(() => {
      sceneRef.current &&
        sceneRef.current.resize(dimensions.width, dimensions.height);
    }, [dimensions.width, dimensions.height]),
    useEffect(() => {
      if (!active) return;
      let cancelled = !1;
      async function load() {
        if (typeof api == "function")
          try {
            const dump = await api("/dump", !0);
            if (cancelled || !dump) return;
            const next = buildTiers(dump, {
              budget: brainBudgetForQuality(quality.tier),
            });
            setTiers(next);
          } catch (err) {
            cancelled || setError(err?.message || String(err));
          }
      }
      return (
        load(),
        () => {
          cancelled = !0;
        }
      );
    }, [active, api, quality.tier]),
    useEffect(() => {
      if (!satellitesRef.current) return;
      satellitesRef.current.setData(tiers);
      const flat = [];
      for (const d of tiers.decisions || []) flat.push(d);
      for (const c of tiers.clusters || []) flat.push(c);
      for (const m of tiers.looseMemories || []) flat.push(m);
      ((slotsAccessor.current = flat),
        setKeyboardSlotIndex((current) =>
          current >= flat.length
            ? flat.length
              ? flat.length - 1
              : -1
            : current,
        ));
      const nextStats = brainStatsForSlots(
        flat,
        beamsRef.current?.activeCount() || 0,
      );
      ((lastStatsRef.current = nextStats),
        writeStats(statRefs.current, nextStats),
        sceneRef.current?.requestFrame());
    }, [tiers]));
  function selectSlot(slot) {
    (satellitesRef.current?.setSelected(slot?.id || null),
      setSelectedSlot(slot || null),
      (selectedSlotRef.current = slot || null),
      sceneRef.current?.requestFrame());
  }
  function handlePointerMove(e) {
    if (!hoverRef.current || !containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    (hoverRef.current.setCursor(e.clientX, e.clientY, rect),
      reducedMotion && hoverRef.current.tick());
  }
  function handlePointerLeave() {
    hoverRef.current?.clearCursor();
  }
  function handleClick(e) {
    if (
      e.button === 2 ||
      !hoverRef.current ||
      !satellitesRef.current ||
      !containerRef.current
    )
      return;
    const rect = containerRef.current.getBoundingClientRect();
    (hoverRef.current.setCursor(e.clientX, e.clientY, rect),
      hoverRef.current.tick());
    const slot = hoveredSlotRef.current;
    if (!slot) {
      (selectSlot(null), setKeyboardSlotIndex(-1));
      return;
    }
    if (selectedSlotRef.current?.id === slot.id) {
      (selectSlot(null), setKeyboardSlotIndex(-1));
      return;
    }
    const index = (slotsAccessor.current || []).findIndex(
      (candidate) => candidate?.id === slot.id,
    );
    (setKeyboardSlotIndex(index), selectSlot(slot));
  }
  function handleContextMenu(e) {
    (e.preventDefault(), selectSlot(null), setKeyboardSlotIndex(-1));
  }
  function handleKeyDown(e) {
    if (!isBrainNavigationKey(e.key)) return;
    const nodes = slotsAccessor.current || [];
    if (!nodes.length) return;
    if ((e.preventDefault(), e.key === "Escape")) {
      ((hoveredSlotRef.current = null),
        setHoverSlot(null),
        selectSlot(null),
        setKeyboardSlotIndex(-1));
      return;
    }
    const nextIndex = nextBrainNodeIndex({
      key: e.key,
      currentIndex: keyboardSlotIndex,
      selectedId: selectedSlotRef.current?.id,
      nodes,
    });
    if (nextIndex < 0) return;
    const slot = nodes[nextIndex];
    ((hoveredSlotRef.current = slot),
      setHoverSlot(slot),
      setKeyboardSlotIndex(nextIndex),
      selectSlot(slot));
  }
  const brainNodeCount =
      (tiers.decisions?.length || 0) +
      (tiers.clusters?.length || 0) +
      (tiers.looseMemories?.length || 0),
    brainHelpId = "brain-v2-keyboard-help",
    selectedAnnouncement = selectedSlot
      ? `Selected ${selectedSlot.label || selectedSlot.id}.`
      : brainKeyboardHelpText(brainNodeCount);
  return React.createElement(
    "div",
    {
      ref: containerRef,
      className: "brain-v2-container",
      role: "region",
      tabIndex: 0,
      "aria-label": `Cortex Brain Map with ${brainNodeCount} nodes`,
      "aria-describedby": brainHelpId,
      style: {
        position: "relative",
        width: dimensions.width,
        height: dimensions.height,
        background: "#040812",
        overflow: "hidden",
      },
      onPointerMove: handlePointerMove,
      onPointerLeave: handlePointerLeave,
      onClick: handleClick,
      onContextMenu: handleContextMenu,
      onKeyDown: handleKeyDown,
    },
    React.createElement(
      "p",
      { id: brainHelpId, className: "sr-only" },
      brainKeyboardHelpText(brainNodeCount),
    ),
    React.createElement(
      "p",
      { className: "sr-only", "aria-live": "polite" },
      selectedAnnouncement,
    ),
    error
      ? React.createElement("div", { className: "brain-v2-error" }, error)
      : null,
    React.createElement(
      "div",
      { className: "brain-v2-hud-strip" },
      React.createElement(
        "span",
        { className: "brain-v2-hud-stat" },
        React.createElement(
          "span",
          { className: "brain-v2-hud-label" },
          "NODES",
        ),
        React.createElement(
          "span",
          {
            ref: (el) => {
              statRefs.current.nodes = el;
            },
          },
          "0",
        ),
      ),
      React.createElement(
        "span",
        { className: "brain-v2-hud-stat" },
        React.createElement(
          "span",
          { className: "brain-v2-hud-label" },
          "CLUSTERS",
        ),
        React.createElement(
          "span",
          {
            ref: (el) => {
              statRefs.current.clusters = el;
            },
          },
          "0",
        ),
      ),
      React.createElement(
        "span",
        { className: "brain-v2-hud-stat" },
        React.createElement(
          "span",
          { className: "brain-v2-hud-label" },
          "DECISIONS",
        ),
        React.createElement(
          "span",
          {
            ref: (el) => {
              statRefs.current.decisions = el;
            },
          },
          "0",
        ),
      ),
      React.createElement(
        "span",
        { className: "brain-v2-hud-stat" },
        React.createElement(
          "span",
          { className: "brain-v2-hud-label" },
          "FIRING",
        ),
        React.createElement(
          "span",
          {
            ref: (el) => {
              statRefs.current.beams = el;
            },
          },
          "0",
        ),
      ),
    ),
    React.createElement("div", {
      className: "brain-v2-ticker",
      "aria-hidden": "true",
      ref: tickerRef,
    }),
    React.createElement(Hud, {
      stats: null,
      hover: hoverSlot,
      selected: selectedSlot,
      firingEntries: [],
    }),
  );
}
function writeStats(refs, stats) {
  refs &&
    (refs.nodes && (refs.nodes.textContent = String(stats.nodes)),
    refs.clusters && (refs.clusters.textContent = String(stats.clusters)),
    refs.decisions && (refs.decisions.textContent = String(stats.decisions)),
    refs.beams && (refs.beams.textContent = String(stats.activeBeams)));
}
function brainStatsForSlots(slots, activeBeams = 0) {
  let clusters = 0,
    decisions = 0;
  for (const slot of slots)
    slot.tier === "cluster"
      ? (clusters += 1)
      : slot.tier === "decision" && (decisions += 1);
  return { nodes: slots.length, clusters, decisions, activeBeams };
}
function renderTicker(host, entries) {
  if (host) {
    for (; host.firstChild;) host.removeChild(host.firstChild);
    for (const entry of entries) {
      const div = document.createElement("div");
      ((div.className = "brain-v2-ticker-line"),
        (div.textContent = entry.label),
        host.appendChild(div));
    }
  }
}
var index_default = BrainV2;
export { BrainV2, index_default as default };

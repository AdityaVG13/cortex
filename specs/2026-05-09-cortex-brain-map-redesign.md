# Cortex Brain Map — Constellation Lattice Redesign

**Date:** 2026-05-09
**Status:** Draft (awaiting user review)
**Component:** `desktop/cortex-control-center/src/BrainVisualizer.jsx`

## 1. Problem

The current Cortex Brain Map renders memory and decision nodes as a force-directed cloud constrained to two anatomical "hemispheres" (frontal/parietal/temporal/occipital/limbic regions). Visually this reads as a literal brain — two orange-and-cyan blobs sitting in an orbital ring. The user explicitly does not want a brain shape; the goal is a Jarvis-style (Iron Man / Avengers MCU) holographic constellation that still communicates topology and supports a click-to-fire neuron animation revealing a node's connections.

## 2. Goals

1. Replace anatomical hemisphere layout with a non-brain organizing geometry that reads as Jarvis-style holographic projection.
2. On node click, animate signal propagation outward from the clicked node along its edges with a BFS-timed ripple, finishing in ≤1.2 s.
3. Preserve all existing affordances: pan/orbit/zoom, hover tooltip, selection panel, auto-rotate toggle, 2D fallback, WebGL detection, error boundary.
4. Hold 60 fps at 1000 nodes / 300 links with bloom enabled on a midrange GPU; auto-degrade gracefully.
5. No regression in the 2D fallback path.

## 3. Non-goals

- Audio cues (Tone.js sub-pluck) — explicitly out of scope, captured as follow-up.
- Touch/mobile ripple gestures.
- Saving or replaying ripple sequences.
- Changing the data shape returned from `/dump` or downstream Cortex APIs.
- Refactoring unrelated parts of `BrainVisualizer.jsx`.

## 4. Visual direction

**Constellation Lattice** — nested hybrid wireframe shells with orbital rings overlaid.

- **Outer shell:** geodesic icosphere wireframe, R≈140, subdivision 2, 1 px emissive cyan (`#40e0ff`) lines.
- **Inner shell:** smaller icosphere, R≈80, dimmer line opacity, amber (`#ffd166`) tint.
- **Orbital rings:** 3 elliptical rings at orthogonal-ish tilts (reusing the existing `ellipseRing` builder), thin cyan, slow auto-rotation.
- **Reticle:** 1 large flat reticle ring on a screen-aligned plane, segmented arcs + tick marks, slow rotation.
- **Center crosshair:** subtle 4-arm cross at origin (Jarvis "lock" glyph).
- **Backdrop:** keep `#040812`, add a subtle radial vignette.

Key Jarvis vocabulary applied: emissive line-art over volumes, segmented arcs, tick-mark dials, persistent corner gauge clusters, soft multi-tap bloom, near-black backdrop, cyan core + amber accent.

### Node placement

- Memory nodes are projected onto the **outer shell** surface, force-directed within the tangent plane (light spring to surface, free angular movement).
- Decision nodes are projected onto the **inner shell**, same scheme.
- Conflict edges pierce both shells (visual: long cyan→red gradient line).
- The existing hemisphere/region/anatomy split (`BRAIN_REGIONS`, `brainRegionForNode`, hemisphere math in `brainLayoutPoint`) is **removed**. Layout is pure radial-by-type.
- A `useShellSplit` flag will gate the outer/inner-shells split vs single-shell + color-only mode (option C from brainstorming) so we can A/B in implementation cheaply.

### HUD

- Top-left card and top-right stats are kept as today.
- Add four corner glyph clusters with tick dials and small status readouts (memory count, agent count, last-firing timestamp).
- Add a small radar reticle bottom-right showing camera bearing.
- Selection panel keeps the current detail panel and adds a **"RIPPLE PATH"** mini-trace listing the BFS hop tree of the most recent activation.

## 5. Click-to-fire animation

When the user clicks a node, the system runs a depth-capped BFS from that node and animates a propagating activation along edges and into neighbor nodes.

### Timing

- **Depth cap:** 2 hops (source = depth 0, frontier reaches depth 2).
- **Per-hop delay:** 110 ms (`stepMs`).
- **Activation rise:** `easeOutCubic`, 80 ms (`riseMs`).
- **Activation decay:** continuous exponential `exp(-t / tau)` with `tau = 280 ms`. The previously listed `easeInQuad` curve is dropped — exponential decay is the canonical action-potential shape and is what the GPU shader applies. Practical visible tail: ~500 ms (decay reaches 16 % at `t = tau × ln(6) ≈ 500 ms`).
- **Total ripple budget formula:** `depth × stepMs + riseMs + visibleTailMs`.
- **Computed total at depth 2:** `2 × 110 + 80 + 500 = 800 ms`. Acceptance criterion uses 800 ms with a 100 ms slack → **must complete within 900 ms**.
- **Depth attenuation:** activation magnitude scales by `0.55^depth` per hop.

### Per-element behavior

- **Source node:** scale pulse 1.0 → 1.4 → 1.0, emissive flash, 600 ms total.
- **Edges on the BFS frontier:** traveling pulse via `TubeGeometry` + GLSL `uHeadPos` shader.
- **Secondary nodes (depth 1, 2):** pop-in ripple ring (additive sprite, expands and fades, 400 ms) on packet arrival.
- **Non-affected nodes:** fade to 25 % opacity, 200 ms ease. Restore at 400 ms ease-out when ripple ends.
- **Camera:** 15 % ease toward the clicked node's position, no re-center, no orbit. User retains orbit control.

### Activation engine

A new module `RippleEngine.js` owns:
- Adjacency precomputation (built once on `graphData` change, keyed by node id).
- BFS executed at click time, returning `{ nodeId → depth, edgeKey → depth }`.
- Activation values written to a `DataTexture` keyed by edge index, sampled in the pulse shader's vertex stage.
- A single `requestAnimationFrame` loop comparing `now − clickTime` against `depth × stepMs` and writing activation crossings — no per-frame BFS.
- Decoupling: time advances continuously; activation values decay with `exp(-t / tau)` per element, `tau = 280 ms`. Multiple simultaneous ripples are additive — same edge can receive activation from two sources, values sum and clamp at 1.0.

## 6. Render pipeline

- **Layer assignment:**
  - Layer 0 (default, no bloom): wireframe shells, orbital rings, reticle, crosshair, HUD overlays. Material: `LineBasicMaterial` w/ `THREE.AdditiveBlending` on near-black backdrop — already glows mildly, no halo bleed.
  - Layer 1 (selective bloom): node instances, traveling pulse tubes, node halos, secondary ripple sprites, reticle tick highlights.
- **Effect composer:** `@react-three/postprocessing` `<EffectComposer>` with selective `<Bloom>` bound to layer 1, `intensity = 0.85`, `luminanceThreshold = 0.18`, `luminanceSmoothing = 0.4`. Tonemapping: `ACESFilmicToneMapping`, `toneMappingExposure = 1.0`.
- **Geometry & draw calls:**
  - 1× `THREE.InstancedMesh` for nodes (1 draw call).
  - 1× merged `BufferGeometry` for all edges with `aEdgeId` + `aActivation` vertex attributes, driven by a single `ShaderMaterial` (1 draw call).
  - Shells: 2 line-mesh draw calls (outer + inner). Rings: 3 draw calls. HUD overlays: ≤ 6 draw calls.
  - **Total draw call ceiling: 50.** Asserted by perf test.
- **Bloom auto-degrade:** monitor a 1 s rolling window of frame times; disable bloom when **median frame time ≥ 33.3 ms** (≤30 fps p50). Re-enable when median falls below 22 ms (≥45 fps p50) sustained for 3 s. Single source of truth — §9 references this section.
- **WebGL fallback:** existing 2D grid path preserved unchanged. No path keeps `linkDirectionalParticles` — the per-link object model conflicts with merged edge geometry, and a parallel non-merged mode is rejected as scope creep.

## 7. File layout

`BrainVisualizer.jsx` stays as the entry component. New modules:

```
desktop/cortex-control-center/src/brain/
  ShellGeometry.js        # icosphere + ellipse ring builders, replaces createJarvisBrainShell
  ShellLayout.js          # applyShellLayout, replaces applyBrainLayout
  RippleEngine.js         # adjacency, BFS scheduler, activation state, DataTexture writes
  PulseShader.js          # GLSL traveling-pulse ShaderMaterial (vertex + fragment)
  HudOverlay.jsx          # corner glyph clusters + bottom-right radar reticle
```

Removed from `BrainVisualizer.jsx`: `BRAIN_REGIONS`, `brainRegionForNode`, anatomical math in `brainLayoutPoint`, `createJarvisBrainShell`, `cortexPath`, `hemisphereOutline`. Preserved: error boundary, WebGL detection, 2D fallback, `fetchBrainData`, `selectedFlow`, `overviewLinkKeys`, hover/click handlers.

Estimated size: ~1015 LOC current → ~1400 LOC distributed across the entry file plus the five modules above.

## 8. Data flow

```
/dump endpoint
  └─> fetchBrainData (unchanged)
        └─> { nodes, links }
              ├─> applyShellLayout(nodes)        [new]
              ├─> RippleEngine.buildAdjacency(links)  [new]
              └─> ForceGraph3D
                    ├─> shell projection force (replaces brainShape force)
                    ├─> InstancedMesh nodes
                    ├─> merged edge geometry + PulseShader
                    └─> click → RippleEngine.fire(nodeId)
                                   └─> writes activation DataTexture
                                         └─> shader reads on next frame
```

## 9. Performance budget

- **Reference machine (single source of truth for fps acceptance):** Windows 11, Intel Core i7-12700H, integrated Iris Xe + NVIDIA GTX 1650, Chrome stable, 1920×1080 viewport, throttling off, dev tools closed.
- **Target:** 60 fps median at 1000 nodes / 300 links with bloom enabled on the reference machine.
- Activation values live in a GPU-side `DataTexture`; no per-frame uniform churn.
- Bloom auto-degrade thresholds defined in §6 (≥33.3 ms median p50 disables; ≤22 ms median p50 sustained 3 s re-enables).
- BFS cost at click time: O(V + E) bounded by the 2-hop frontier, well under 1 ms for the target graph size.
- Single rAF loop drives all activation timing — no per-edge `setTimeout` storm.
- Total draw call ceiling: 50 (asserted in perf test).

## 10. Tests

### Unit (Vitest)

- `BFS depth cap` — frontier never exceeds depth 2 regardless of graph density.
- `Easing` — `easeOutCubic(0)=0`, `easeOutCubic(1)=1`, monotonic on [0,1]; `exp(-t/tau)` decay produces 0.16 ± 0.01 at `t = tau × ln(6)`.
- `Shell projection deterministic` — `applyShellLayout` produces identical positions for identical node ids:
  - across separate runs (seeded hash);
  - across `graphData` reference changes that don't touch the node id set (regression case for memoization invalidation).
- `Adjacency rebuild` — cache invalidates only when `graphData.links` reference changes; node-only changes leave it intact.
- `Ripple additivity` — two simultaneous ripples that share an edge produce a clamped activation ≤ 1.0 with sum semantics.
- `Camera ease` — `applyCameraEase(camera, target, 0.15)` moves camera by exactly 15 % of the source-to-target vector and never re-centers.
- `Dim & restore` — non-selected nodes drop to 0.25 ± 0.01 opacity during ripple and return to 1.0 within 400 ms of completion.

### Perf harness

- `Draw call ceiling` — render a synthetic 1000-node / 300-link scene, assert `renderer.info.render.calls < 50`.
- `Bloom auto-degrade trigger` — inject a 50 ms artificial frame stall for 1.5 s, assert bloom disables; remove stall, assert re-enable within 4 s.

### Visual regression

- Puppeteer snapshot of a static frame with seeded data. **Bloom is disabled in the snapshot harness** (driver/GPU variance routinely exceeds tight thresholds with bloom on). Compared via `pixelmatch` with a 0.1 threshold against a checked-in baseline.

### Manual smoke (recorded in PR)

- 1000-node load < 2 s on the reference machine.
- Click-to-fire latency < 100 ms (click → first frame of activation).
- Total ripple complete within 900 ms (matches §5 budget).
- 60 fps median during a sustained ripple sequence on the reference machine.
- "RIPPLE PATH" HUD trace populated correctly for a clicked node with ≥ 4 neighbors.

## 11. Migration & rollout

- Feature is a visual replacement, not a data change — no migration required.
- **Build-time flag `CORTEX_BRAIN_LATTICE`** (Vite env var, default `1` once shipped, settable to `0` to keep the old anatomy renderer for a short overlap window during internal QA only). Removed entirely after one release cycle.
- **Runtime flag `useShellSplit`** (React state, default `true`):
  - `true` → memory on outer shell, decisions on inner shell (option (a) — outer/inner split).
  - `false` → both groups share the outer shell, distinguished by color + glyph only (option (c) — color-only).
  - Toggle exposed in the HUD's MANUAL/AUTO control row as a small "SPLIT" pill button so the user can A/B in the live app once they see the prototype. No build step required to switch.
- Old `createJarvisBrainShell` and anatomical region code are deleted in the same PR — no compatibility shim, per the user's "no backwards-compatibility hacks" rule.

## 12. Open questions

- **Decision/memory split** — user picked outer/inner shells (option a) tentatively, with an explicit "needs to see it" caveat. The runtime `useShellSplit` toggle (defined in §11) lets the user A/B in the live app once the prototype is visible.
- **Audio cues** — flagged as follow-up; not in this spec.
- **Mobile/touch behavior** — out of scope.

## 13. Risks

- **Dense clusters hidden on far hemisphere of the outer shell.** Mitigation: camera-aligned culling for non-selected nodes during selection, and the 15 % camera ease pulls the user closer to the active region.
- **Bloom on low-end hardware** — covered by auto-degrade.
- **Shader compile failures on non-WebGL2 drivers** — feature-detect WebGL2 + shader compile success at mount; on failure, fall back to the existing **2D grid** path (already used for missing WebGL). No `linkDirectionalParticles` parallel mode is retained — the per-link object model conflicts with merged edge geometry, and adding a second renderer is rejected as scope creep.
- **Layout instability when nodes change** — `applyShellLayout` is deterministic on node id, so re-renders place the same node at the same position; force-relaxation only operates within the tangent plane.

## 14. Acceptance criteria

- No anatomical brain shape visible at any zoom level.
- Outer + inner geodesic shells, 3 orbital rings, reticle, and crosshair all render.
- Click on any node triggers a 2-hop BFS ripple completing within **900 ms** (matches §5 timing math).
- Source node pulses; intermediate edges show a traveling glow; depth-1 and depth-2 neighbors pop in with ripple rings.
- Non-selected nodes fade to 25 % opacity during ripple; restore within 400 ms of completion.
- Camera eases 15 % toward clicked node; user retains orbit control throughout.
- 60 fps median at 1000 nodes / 300 links with bloom enabled on the **reference machine defined in §9**.
- Total draw call count < 50 (asserted by perf harness).
- Bloom auto-degrade triggers at the §6 thresholds and re-enables on recovery.
- "RIPPLE PATH" HUD trace correctly lists BFS hops for a clicked node.
- `useShellSplit` toggle visibly switches between outer/inner-shell and single-shell-color-only modes.
- 2D fallback path unchanged and still renders correctly when WebGL is unavailable.
- All existing tests still pass; new unit, perf, and snapshot tests added.

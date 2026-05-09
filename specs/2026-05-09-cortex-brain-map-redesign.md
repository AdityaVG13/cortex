# Cortex Brain Map — Constellation Lattice Redesign

**Date:** 2026-05-09
**Status:** Draft (awaiting user review)
**Component:** `desktop/cortex-control-center/src/BrainVisualizer.jsx`
**Author:** Aditya + Claude

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

- Per-hop delay: 110 ms.
- Activation rise: `easeOutCubic`, 80 ms.
- Activation decay: `easeInQuad`, 500 ms.
- Total ripple budget: 1.2 s (3 hops × 110 ms + 80 ms rise + 500 ms decay tail).
- Depth cap: 2 hops. Activation magnitude decays by `0.55^depth` per hop.

### Per-element behavior

- **Source node:** scale pulse 1.0 → 1.4 → 1.0, emissive flash, 600 ms total.
- **Edges on the BFS frontier:** traveling pulse via `TubeGeometry` + GLSL `uHeadPos` shader. (Fallback: gate the existing `linkDirectionalParticles` API by activation state — same behavior, lower implementation cost.)
- **Secondary nodes (depth 1, 2):** pop-in ripple ring (additive sprite, expands and fades, 400 ms) on packet arrival.
- **Non-affected nodes:** fade to 25 % opacity, 200 ms ease. Restore at 400 ms ease-out when ripple ends.
- **Camera:** 15 % ease toward the clicked node's position, no re-center, no orbit. User retains orbit control.

### Activation engine

A new module `RippleEngine.js` owns:
- Adjacency precomputation (built once on `graphData` change, keyed by node id).
- BFS executed at click time, returning `{ nodeId → depth, edgeKey → depth }`.
- Activation values written to a `DataTexture` keyed by edge index, sampled in the pulse shader's vertex stage.
- A single `requestAnimationFrame` loop comparing `now − clickTime` against `depth × stepMs` and writing activation crossings — no per-frame BFS.
- Decoupling: time advances continuously; activation values decay with `exp(-t / tau)` per element. Multiple simultaneous ripples are additive.

## 6. Render pipeline

- `@react-three/postprocessing` `<EffectComposer>` with selective `<Bloom>` on layer 1.
- Emissive elements (nodes, pulses, halos, reticle ticks) → layer 1.
- Wireframe shells use `THREE.AdditiveBlending`, no bloom — keeps line-art crisp.
- Single `THREE.InstancedMesh` for nodes (1 draw call).
- Merged `BufferGeometry` for edges with an `aEdgeId` attribute (1 draw call).
- Total draw call target: < 50.
- Bloom auto-disables when measured frame time exceeds 33 ms over a 1 s window (auto-degrade).
- WebGL fallback: existing 2D grid path preserved unchanged.

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

- Target: 60 fps at 1000 nodes / 300 links with bloom enabled on a midrange GPU (e.g. Intel Iris Xe / GTX 1650 class).
- Activation values live in a GPU-side `DataTexture`; no per-frame uniform churn.
- Bloom auto-disabled below 30 fps measured over 1 s.
- BFS cost at click time: O(V + E) bounded by the 2-hop frontier, well under 1 ms for the target graph size.
- Single rAF loop drives all activation timing — no per-edge `setTimeout` storm.

## 10. Tests

- **Unit (Vitest):**
  - `BFS depth cap` — frontier never exceeds depth 2 regardless of graph density.
  - `Easing functions` — `easeOutCubic` and `easeInQuad` boundary values (0, 1) match expected.
  - `Shell projection deterministic` — `applyShellLayout` produces identical positions for identical node ids across runs (seeded hash).
  - `Adjacency rebuild` — invalidates cache only when `graphData.links` reference changes.
- **Visual regression:**
  - Puppeteer snapshot of a static frame with seeded data, compared via `pixelmatch` with a 0.1 threshold.
- **Manual smoke (recorded in PR):**
  - 1000-node load < 2 s.
  - Click-to-fire latency < 100 ms (click → first frame of activation).
  - 60 fps held during a sustained ripple sequence on the developer's reference machine.

## 11. Migration & rollout

- Feature is a visual replacement, not a data change — no migration required.
- Ship behind a single env-gated flag `CORTEX_BRAIN_LATTICE=1` for the first internal build, then default-on once visually approved.
- Old `createJarvisBrainShell` and anatomical region code are deleted in the same PR — no compatibility shim, per the user's "no backwards-compatibility hacks" rule.

## 12. Open questions

- **Decision/memory split** — user picked outer/inner shells (option a) tentatively, with an explicit "needs to see it" caveat. The `useShellSplit` flag preserves the option to switch to color-only (option c) cheaply once the prototype is visible.
- **Audio cues** — flagged as follow-up; not in this spec.
- **Mobile/touch behavior** — out of scope.

## 13. Risks

- **Dense clusters hidden on far hemisphere of the outer shell.** Mitigation: camera-aligned culling for non-selected nodes during selection, and the 15 % camera ease pulls the user closer to the active region.
- **Bloom on low-end hardware** — covered by auto-degrade.
- **Shader compile failures on non-WebGL2 drivers** — fall back to the existing `linkDirectionalParticles` path; the entire shader pipeline is gated behind a feature detection.
- **Layout instability when nodes change** — `applyShellLayout` is deterministic on node id, so re-renders place the same node at the same position; force-relaxation only operates within the tangent plane.

## 14. Acceptance criteria

- No anatomical brain shape visible at any zoom level.
- Outer + inner geodesic shells, 3 orbital rings, reticle, and crosshair all render.
- Click on any node triggers a 2-hop BFS ripple completing within 1.2 s.
- Source node pulses; intermediate edges show a traveling glow; depth-1 and depth-2 neighbors pop in with ripple rings.
- Non-selected nodes fade to 25 % opacity during ripple; restore on completion.
- Camera eases 15 % toward clicked node; user retains orbit control throughout.
- 60 fps held at 1000 nodes / 300 links with bloom on the reference machine.
- 2D fallback path unchanged and still renders correctly when WebGL is unavailable.
- All existing tests still pass; new unit + snapshot tests added.

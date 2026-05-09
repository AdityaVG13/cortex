# Cortex Brain Tab — v2 PRD ("Living Constellation")

**Date:** 2026-05-09
**Status:** Draft (post grill-me, awaiting user sign-off)
**Replaces:** `specs/2026-05-09-cortex-brain-map-redesign.md` and its plan
**Component:** `desktop/cortex-control-center/src/BrainVisualizer.jsx` and a new `desktop/cortex-control-center/src/brain-v2/` tree
**Daemon:** `daemon-rs/src/handlers/events.rs` (new `/brain/firing` SSE), emit calls in `daemon-rs/src/crystallize.rs` and `daemon-rs/src/handlers/recall.rs`
**Author:** Aditya + Claude

---

## 1. Why we're rewriting

V1 (Constellation Lattice) shipped functional but unusable: stutter on 1020 nodes, no firing pulses on click after rebuild, bloom integration repeatedly broke the render, overall did not feel "alive". Root causes:

- `react-force-graph-3d`'s simulation churned O(n²) charge force per tick on pinned nodes
- Per-node `THREE.Mesh` instantiation rather than instancing
- Postprocessing pipeline conflicts with the library's render loop
- No real source of "firing" — the brain looked the same whether the daemon was idle or under load

V2 starts from a different premise: **the Brain is a live window into Cortex's consolidation process**, not a render of every memory in the database. The viz gets its life from the daemon's actual ML pipeline (clustering + recall) via a new SSE endpoint, and the rendering is a bespoke Three.js scene tuned for ~150 satellites + 1 core — sized for what consolidation actually produces, not what `/dump` returns.

## 2. Goals

1. **Showcase aesthetic primary, operational secondary** — the Brain tab is Cortex's visual signature; people see it and know what the product does. It is not a debugger.
2. **Live, but only honest** — every visible firing event maps to a real recall or consolidation event, except when the daemon has been idle ≥12 s, when ambient simulated firing fills the silence.
3. **60 fps median on the reference machine** with no perf-degrade two-mode fallback.
4. **Hand-rolled Three.js** — no `react-force-graph-3d`, no `postprocessing` lib, no R3F.
5. **Three-tier spatial hierarchy** that maps 1:1 to the data model: decisions (inner), clusters (mid), loose memories (outer).
6. **Per-cluster identity** — each cluster gets its own hue so the eye learns "the green satellite is the auth cluster".
7. **Camera that reacts to events** — auto-rotate at rest, briefly spotlights firing satellites.
8. **No bloom postprocessing** — emission via additive blending and pre-blurred sprite halos.
9. **Delete v1 wholesale** in the same PR.

## 3. Non-goals

- Rendering all stored memories (we don't; v1 tried this and stuttered).
- Click-to-trace / manual ripple mode (conflicts with live firing signal).
- Audio cues.
- Mobile/touch support.
- Cinematic intro fly-in on every tab switch.
- Two-mode quality settings or runtime perf-degrade.
- Any change to the public `/events/stream` SSE scrub policy.
- Any new desktop app routes; v2 reuses the Brain tab slot.

## 4. Visual direction — "Living Constellation"

### 4.1 Scene composition

- **Core**: two counter-rotating wireframe icosahedrons at origin (radius ~25), nested. Outer cage rotates +0.18 rad/s on Y. Inner cage rotates -0.32 rad/s on X+Y. A single large additive sprite halo (radius ~80, cyan) sits behind both cages.
- **Inner ring** (R=80, ~20 satellites): **decisions**. Amber hue.
- **Mid ring** (R=140, ~80 satellites): **memory cluster centroids** from `memory_clusters` table. Per-cluster hue (golden-angle distribution around HSL: hue = (cluster_id * 137.508) mod 360°, S=70%, L=58%).
- **Outer cloud** (R=180–220, jittered, ~50 satellites): **most recent loose memories** not yet consolidated. Soft cyan, smaller halos.
- **No shells, no rings, no reticle, no crosshair, no orbital ellipses.** Empty space sells the depth.

### 4.2 Tier sizing

- **Decision satellite radius**: 2.0 units (constant).
- **Cluster centroid satellite radius**: `clamp(log2(member_count + 1) * 1.4, 1.4, 4.0)` units.
- **Loose memory satellite radius**: 1.0 unit (constant).
- Each satellite is one `Sprite` (additive halo) + one tiny `Mesh` (sphere geometry, instanced) for the body. Halo radius = body radius × 3.

### 4.3 Cold-start fallback

If `memory_clusters` is empty (new install), the mid ring renders the top 80 memories by score using cyan hue (no per-cluster identity yet). When the first real cluster materializes, the mid ring transitions over 1.5 s to cluster mode.

### 4.4 Idle behavior

- **Always**: core counter-rotation, halo breathing (1.5 s sin cycle, ±8% intensity), satellites bob ±2% of radius on individual phase offsets.
- **Auto-rotate**: camera orbits Y axis at 0.04 rad/s starting from first paint. User drag pauses; resume after 8 s of no input.
- **Real-event override**: any SSE event resets the idle timer.
- **Simulated firing**: when no real event arrives for 12 s, fire one fake beam every 4–8 s (random satellite → core, decay 600 ms). Real event arrival immediately suppresses any in-flight fakes.

### 4.5 Beam aesthetics

- Beams are **curved arcs** (16 segments, bezier with midpoint lifted radially outward from origin).
- Traveling pulse via merged `BufferGeometry` + GLSL `ShaderMaterial` with `uTime`, `uHeadPos`, per-vertex `aProgress`, `aBeamId`, and per-beam activation read from a `DataTexture` — same pattern that worked in v1 (port ~50 LOC).
- Beam life: 600 ms total. Rise 80 ms (cubic), exponential decay τ=280 ms.
- Beam color: source satellite's hue, fading to white at the head.

### 4.6 Camera spotlight on firing

When any real `/brain/firing` event fires from satellite *S*, camera eases 15% toward *S*'s world position over 800 ms (cubic ease), then resumes auto-rotate. Implementation: lerp `camera.position` and `controls.target` independently. Multiple simultaneous events spotlight only the most recent.

### 4.7 Color palette

- Background: `#040812`
- Decisions: `#ffd166` (amber, unchanged from v1)
- Cluster hue: HSL(`(id * 137.508) mod 360`, 70%, 58%) → multiplied to RGB at sprite/sphere material level
- Loose memories: `#22d3ee` (cyan)
- Core halo: `#40e0ff`
- Beam head: `#f8fbff`
- Selected node: `#ffffff` body + 1.4× halo

## 5. Interaction model

- **Orbit + zoom + pan** via `THREE.OrbitControls` (vanilla three.js examples). `zoomSpeed=0.7`, no damping (simpler), origin-locked target.
- **Hover**: pointer raycast against the satellite InstancedMesh. Highlights nearest satellite, shows DOM tooltip with `label`, `member_count` (clusters), `agent`. No state change.
- **Click**: pins detail panel. Body color → white, halo → 1.4× scale. Other satellites do NOT dim (showcase priority — keep the constellation visible).
- **Right-click**: deselects. Suppresses native context menu.
- **Drag**: pauses auto-rotate. Resumes after 8 s idle.

## 6. Data: `/brain/firing` SSE endpoint

### 6.1 Endpoint

`GET /brain/firing` on the daemon (port 7437). Auth via existing token. Owner-scoped: only emits events for memories/clusters owned by the requesting user.

### 6.2 Event types

```jsonc
// Consolidation pipeline
{ "type": "consolidation_started", "ts": "..." }
{ "type": "member_added",       "ts": "...", "cluster_id": 42, "member_id": "mem-123" }
{ "type": "cluster_finalized",  "ts": "...", "cluster_id": 42, "member_count": 7 }
{ "type": "link_inferred",      "ts": "...", "a": "mem-1", "b": "mem-9", "score": 0.81 }

// Recall — emit on every /recall
{ "type": "recall",             "ts": "...", "node_ids": ["mem-1","mem-3","crystal-7"] }
```

### 6.3 Throttle

SSE coalesces events arriving in the same 50 ms window into one batch. Brain receives at most ~20 events/s. Above that, oldest events drop.

### 6.4 Payload privacy

- No raw memory text on the wire.
- Only: cluster IDs, node IDs, member counts, scores, timestamps.
- IDs map to local data the user already has via `/dump` cache.
- Public `/events/stream` scrub policy unchanged.

### 6.5 Daemon emit work

- `daemon-rs/src/crystallize.rs`: emit `consolidation_started`, `member_added`, `cluster_finalized`, `link_inferred` at the appropriate code points.
- `daemon-rs/src/handlers/recall.rs`: emit `recall` with the `node_ids` returned to the caller.
- `daemon-rs/src/handlers/events.rs`: add `handle_brain_firing_stream` mounted at `/brain/firing` that subscribes to the brain-firing channel (a separate `broadcast::Sender<BrainFiringEvent>` on `RuntimeState`) and emits full payloads (not scrubbed).
- `daemon-rs/src/state.rs`: add `pub brain_firing: broadcast::Sender<BrainFiringEvent>`.

## 7. Render pipeline

- **Single `THREE.Scene`**. No composer, no postprocessing.
- **Cameras**: one `PerspectiveCamera` (fov=55, near=1, far=2000), positioned at `(0, 60, 380)` initially.
- **Renderer**: `THREE.WebGLRenderer` with `antialias=true`, `alpha=false`, `powerPreference="high-performance"`.
- **Tonemapping**: `THREE.LinearToneMapping` (default). Skip ACES — v1 hurt us there.
- **Draw call ceiling**: target ≤ 25 (1× core cages, 1× core halo sprite, 1× satellite InstancedMesh body, 1× satellite halo InstancedMesh sprite, 1× merged beam BufferGeometry, ≤ 5 HUD text quads).
- **Frame loop**: single `requestAnimationFrame` driving:
  1. Update orbit target on selection ease
  2. Tick auto-rotate angle if not paused
  3. Counter-rotate core cages
  4. Update halo intensity (sin breathing)
  5. Update beam shader `uTime` and decay activation `DataTexture`
  6. Cull expired beams
  7. `renderer.render(scene, camera)`

### 7.1 Halo sprite generation

One pre-built halo texture (radial RGBA gradient, 64×64, generated via offscreen canvas at boot). All satellite + core halos use the same texture, tinted via `SpriteMaterial.color`. Avoids texture swapping cost.

### 7.2 Beam geometry

Pool of 64 pre-allocated beam slots in a single merged `BufferGeometry` (16 verts per beam, 1024 total). Each beam slot has `aBeamId`, `aProgress`. When a real event arrives, the engine writes the source/target world positions into the relevant slot's vertices and sets activation to 1.0; engine ticks decay activation per frame.

## 8. File layout

```
desktop/cortex-control-center/src/brain-v2/
  Scene.js              # build/teardown the THREE.Scene + camera + renderer
  Core.js               # counter-rotating icosahedrons + halo
  Satellites.js         # InstancedMesh body + halo sprites; tier-shell layout
  Beams.js              # merged BufferGeometry + traveling-pulse shader
  Halo.js               # pre-built radial gradient texture (cached)
  FiringClient.js       # SSE client for /brain/firing + reconnect
  IdleSimulator.js      # 12s timer + simulated firing scheduler
  Camera.js             # auto-rotate + spotlight ease
  Hover.js              # raycast against satellite InstancedMesh
  Hud.jsx               # slim status strip + bottom-left FIRING ticker (DOM)
  ClusterPalette.js     # cluster_id → HSL hue mapping
  index.js              # mounts everything; exports <BrainV2 />

desktop/cortex-control-center/src/BrainVisualizer.jsx   # becomes a thin wrapper that renders <BrainV2 />, retains the existing 2D fallback for no-WebGL
```

**Deleted in same PR:**
- `desktop/cortex-control-center/src/brain/ShellGeometry.js`
- `desktop/cortex-control-center/src/brain/ShellLayout.js`
- `desktop/cortex-control-center/src/brain/RenderLayers.js`
- `desktop/cortex-control-center/src/brain/PostFx.js`
- `desktop/cortex-control-center/src/brain/PulseShader.js`
- `desktop/cortex-control-center/src/brain/EdgeMesh.js`
- `desktop/cortex-control-center/src/brain/easing.js` (port to brain-v2/util/easing.js)
- `desktop/cortex-control-center/src/brain/RippleEngine.js`
- `desktop/cortex-control-center/src/brain/__tests__/*`

**Dependencies removed from `package.json`:**
- `react-force-graph-3d`
- `postprocessing`

## 9. HUD

- **Top-right slim strip** (single row): NODES (visible satellite count) · CLUSTERS · DECISIONS · MEM · DEC · FPS · auto-rotate state. Tiny monospace.
- **Bottom-left FIRING ticker**: scrolling text feed of last 5 events: `cluster_finalized · 7 members · 42ms ago`. Updates on every SSE event; entries fade after 6 s.
- **Click-pin detail panel** appears in top-left when a satellite is selected: label, type, agent, member_count (clusters), recall count last 24 h, list of linked node IDs (top 5).
- **No legend, no MANUAL/AUTO toggle button** — the auto-rotate state is in the strip and toggles automatically on drag.

## 10. Performance budget

- **Reference machine**: Windows 11, Intel Core i7-12700H, integrated Iris Xe, Chrome stable, 1920×1080 viewport.
- **Target**: 60 fps median sustained, idle and active.
- **Draw calls**: ≤ 25 (asserted in unit test by sampling `renderer.info.render.calls`).
- **Vertex count**: ≤ 5 000 (icosahedrons + InstancedMesh + beams + halos).
- **No two-mode fallback.** If perf can't hold, the implementation is wrong, not the budget.

## 11. SSE client behavior

- `EventSource` to `/brain/firing` with auth header (use Tauri's existing API bridge, not raw fetch).
- Auto-reconnect with backoff: 250 ms, 500 ms, 1 s, 2 s, 4 s, capped at 4 s.
- Events buffered into a 256-deep FIFO; consumed by the idle simulator (which suppresses fakes) and the firing engine.
- On disconnect, idle simulator continues — viz never goes "dead".

## 12. Testing

### 12.1 Unit (Vitest)

- `Satellites.test.js`: tier shell layout deterministic on cluster_id, position counts match input.
- `ClusterPalette.test.js`: golden-angle hue is reproducible, distinct neighbors > 30° apart.
- `Beams.test.js`: pool reuse — 100 sequential fires reuse ≤ 64 slots; expired slots reclaimable.
- `IdleSimulator.test.js`: real event resets idle timer; fake suppression while real events arrive within window.
- `easing.test.js`: ported from v1.
- `FiringClient.test.js`: backoff schedule, FIFO drop on overflow.

### 12.2 Perf

- Headless `gl` package: build full scene with 150 satellites + 64 active beams, render 600 frames, assert avg `renderer.info.render.calls` ≤ 25 and total CPU time within budget.

### 12.3 Visual regression

- Puppeteer snapshot of static frame (auto-rotate paused, seeded data, idle simulator off). Compared via `pixelmatch` 0.15 threshold against checked-in baseline.

### 12.4 Manual smoke (recorded in PR)

- 60 fps median sustained 60 s on reference machine (idle).
- 60 fps median sustained 60 s with daemon firing 10 events/s.
- Click-pin panel populated correctly on satellite click.
- Right-click clears selection.
- Drag pauses auto-rotate; resumes after 8 s.
- Firing event triggers camera spotlight; camera resumes orbit.
- 12 s idle triggers simulated beams; real event suppresses next fake.
- SSE reconnect after daemon bounce.

## 13. Open questions

- **Cluster recency in tooltip** — show "last activity" timestamp? Defer.
- **Telemetry on perf** — emit fps to `/admin` for collection? Defer.
- **Cluster split / merge events** — does the daemon ever reassign members? Likely yes; future work.

## 14. Risks

- **Daemon emit work in `crystallize.rs`** — that file is 800+ LOC and the consolidation pipeline is non-trivial. Risk: misplaced emits leak memory text or fire too often. Mitigation: review with eyes on, throttle at SSE-side as backup.
- **Cluster cold-start gap** — if a fresh user has zero clusters and we render only the loose-memory ring, the constellation looks sparse. Mitigation: cold-start fallback in §4.3.
- **Cluster ID instability** — if the daemon ever recycles cluster IDs (e.g. on a recompute), per-cluster hues would shuffle. Mitigation: confirm with ownership of `crystallize.rs` that IDs are stable; if not, hash on `(label, owner_id)` instead.
- **EventSource in Tauri webview** — confirm Tauri's webview supports `EventSource` natively. Fallback: long-poll over the existing IPC bridge.

## 15. Acceptance criteria

- v2 renders: counter-rotating core, three-tier shells, per-cluster colors, no v1 anatomy / lattice / orbital rings / reticle / crosshair.
- 60 fps median on the reference machine, idle and active.
- Real `/brain/firing` events trigger the correct visual response (cluster glow, member-added beams, link-inferred arcs, recall pulses).
- 12 s idle threshold engages simulated firing; real event suppresses.
- Click-pin + right-click deselect both work.
- Camera spotlight + auto-rotate resume both work.
- v1 BrainVisualizer code, `react-force-graph-3d`, `postprocessing`, and all `brain/*` files are gone.
- `/brain/firing` endpoint mounted, scoped to owner, emits the five event types from §6.2.
- Bundle size for the BrainVisualizer chunk drops vs v1 (target: ≤ 600 KB / 200 KB gzip).
- All Vitest tests pass; perf harness asserts draw call ≤ 25.
- Existing 2D fallback path still triggers when WebGL2 is unavailable.

---

## Appendix — Locked grill-me decisions

| # | Decision | Picked |
|---|----------|--------|
| Q1 | Purpose | (c) Showcase / Jarvis aesthetic primary |
| Q2 | Scale | (c) Aesthetic abstraction (~150 nodes) |
| Q2′ | Real firing | Only actual firing nodes show, not all data |
| Q3a | Data path | (b) New `/brain/firing` SSE w/ anonymized payload |
| Q4 | Render tech | (a) Hand-rolled Three.js, drop force-graph |
| Q5 | Visual idiom | (b) Centered core + orbital satellites |
| Q6 | Idle | (d) Breathing + simulated firing only when no real activity for 12 s |
| Q7 | Satellites | (d) Cluster centroids + decisions + recent loose memories |
| Q8 | Interaction | (c) Hover tooltip + click-pin + right-click deselect |
| Q9 | HUD | (b) Slim strip + FIRING ticker |
| Q10 | Color | (b) Per-cluster hue (golden-angle) |
| Q11 | Camera | (d) Auto-rotate + spotlight on firing |
| Q12 | Daemon emits | (c) Consolidation pipeline + recall |
| Q13 | Perf | (b) 60 fps target, no two-mode fallback |
| Q14 | V1 disposal | (a) Delete wholesale |
| Q15 | Bloom | (b) Sprite halos, no postprocessing |
| Q16 | Beams | (c) Curved arcs + traveling pulse |
| Q17 | Core | (c) Counter-rotating dual icosahedrons + halo |
| Q18 | Satellite layout | (c) Three-tier shells |

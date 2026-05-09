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

If `memory_clusters` is empty (new install), the mid ring renders the top 80 memories by score using cyan hue (no per-cluster identity yet). When the first real cluster materializes, the mid ring **crossfades** to cluster mode over 1.5 s — same 80-slot InstancedMesh stays allocated for the lifetime of the scene; only per-instance attributes (color, position, scale) lerp. No InstancedMesh rebuild, no allocation, no flicker. Slots whose semantics change run an alpha 1→0→1 fade through the transition.

### 4.4 Idle behavior

- **Always**: core counter-rotation, halo breathing (1.5 s sin cycle, ±8% intensity), satellites bob ±2% of radius on individual phase offsets.
- **Auto-rotate**: camera orbits Y axis at 0.04 rad/s starting from first paint. User drag pauses; resume after 8 s of no input.
- **Real-event override**: any SSE event resets the idle timer.
- **Simulated firing**: when no real event arrives for 12 s, fire one fake beam every 4–8 s (random satellite → core, decay 600 ms). Real event arrival immediately suppresses any in-flight fakes.
- **Reproducibility**: the simulator uses a seedable mulberry32 PRNG. Default seed is `Date.now()`; tests pass an explicit seed so fake-firing schedules are reproducible.

### 4.5 Beam aesthetics

- Beams are **curved arcs** (16 segments, bezier with midpoint lifted radially outward from origin).
- Traveling pulse via merged `BufferGeometry` + GLSL `ShaderMaterial` with `uTime`, `uHeadPos`, per-vertex `aProgress`, `aBeamId`, and per-beam activation read from a `DataTexture` — same pattern that worked in v1 (port ~50 LOC).
- Beam life: 600 ms total. Rise 80 ms (cubic), exponential decay τ=280 ms.
- Beam color: source satellite's hue, fading to white at the head.

### 4.6 Camera spotlight on firing

When any real `/brain/firing` event fires from satellite *S*: spotlight engages for 800 ms, then resumes auto-rotate.

Spotlight math: target position is `camera.position + 0.15 * (S.worldPosition - camera.position)` (lerp factor 0.15 of the camera→satellite vector — pulls the camera 15 % closer to the satellite, never re-centers). Apply via `camera.position.lerp(target, easeOutCubic(t))` where `t` ramps 0→1 over 800 ms. `controls.target` lerps independently from origin to `0.15 * S.worldPosition` over the same envelope, then returns to origin in 400 ms after the 800 ms hold.

Multiple simultaneous events: only the most recent spotlight is honored — the in-flight envelope is hard-cut and a fresh 800 ms ramp begins from the current camera state. No queue.

### 4.7 Color palette

- Background: `#040812`
- Decisions: `#ffd166` (amber, unchanged from v1)
- **Cluster hue**: derived from a stable hash of the cluster's centroid bytes (NOT the row id). The daemon stores cluster `centroid` as a Float32 vector blob; the desktop client computes `palette_seed = fnv1a32(centroid_bytes)`, then `hue = (palette_seed * 137.508 / 0x100000000) * 360°` mod 360, S=70%, L=58%. Centroids drift slowly (only on consolidation passes) so hue is stable; if the daemon reincarnates a cluster under a new id with the same centroid the hue is unchanged. If the centroid genuinely changes (members merged/split), a hue shift is the correct visual signal.
- Loose memories: `#22d3ee` (cyan)
- Core halo: `#40e0ff`
- Beam head: `#f8fbff`
- Selected node: `#ffffff` body + 1.4× halo

## 5. Interaction model

- **Orbit + zoom + pan** via `THREE.OrbitControls` (vanilla three.js examples). `zoomSpeed=0.7`, no damping (simpler), origin-locked target.
- **Hover**: pointer raycast against the satellite InstancedMesh. **rAF-throttled** — pointermove handlers store the latest screen-space cursor position; the next render frame performs at most one raycast against the InstancedMesh. Native pointer rates (up to 1 kHz) collapse to ≤ 60 raycasts/s. Highlights nearest satellite, shows DOM tooltip with `label`, `member_count` (clusters), `agent`.
- **Click**: pins detail panel. Body color → white, halo → 1.4× scale. Other satellites do NOT dim (showcase priority — keep the constellation visible).
- **Right-click**: deselects. Suppresses native context menu.
- **Drag**: pauses auto-rotate. Resumes after 8 s idle.

## 6. Data: `/brain/firing` SSE endpoint

### 6.1 Endpoint

`GET /brain/firing?token=<token>` on the daemon (port 7437). Token must match the runtime auth token. **Auth via query string is mandatory** — the browser/Tauri webview's `EventSource` does not support custom headers, so the token rides on the URL. The handler resolves the caller's `owner_id` from the token and rejects with HTTP 401 if absent.

### 6.2 Event types

```jsonc
// Consolidation pipeline
{ "type": "consolidation_started", "ts": "..." }                                                      // visual: core halo pulse 1.0 → 1.2 → 1.0 over 800 ms
{ "type": "member_added",       "ts": "...", "cluster_id": 42, "member_id": "mem-123" }              // visual: beam from member satellite → cluster centroid
{ "type": "cluster_finalized",  "ts": "...", "cluster_id": 42, "member_count": 7, "owner_id": 1 }    // visual: cluster halo flash + cage scale pulse
{ "type": "link_inferred",      "ts": "...", "a": "mem-1", "b": "mem-9", "score": 0.81 }              // visual: beam between A and B
{ "type": "recall",             "ts": "...", "node_ids": ["mem-1","mem-3","crystal-7"] }              // visual: each node halo pulse + thin beam to core
```

Every event payload also carries an `owner_id` field (omitted from the JSON shown above for some types but present in the Rust struct). The SSE handler filters by it before forwarding — see §6.4.

### 6.3 Throttle / coalescing

The Brain channel coalesces by *batching*, not by merging. Events arriving in a 50 ms window are grouped into a single SSE message whose `data` field is a **JSON array** of event objects:

```
event: brain_batch
data: [{"type":"recall",...},{"type":"member_added",...}]
```

If the broadcast channel's per-subscriber backlog exceeds 256 events, the oldest are dropped (this is `tokio::sync::broadcast`'s natural behavior; we expose the lag so the client can log it).

### 6.4 Payload privacy + owner scoping

- No raw memory text on the wire.
- Only: cluster IDs, node IDs, member counts, scores, timestamps, `owner_id`.
- The new `BrainFiringEvent` broadcast channel is **global**; per-subscriber owner filtering happens inside `handle_brain_firing_stream`: each event is checked against the caller's resolved `owner_id` and dropped before the SSE write if it doesn't match. **Fail-closed**: if the handler can't resolve `owner_id` it forwards nothing. Mirrors the existing pattern at `crystallize.rs:594` (`search_crystals_filtered`).
- IDs map to local data the user already has via `/dump` cache.
- Public `/events/stream` scrub policy unchanged.

### 6.5 Daemon emit work

- `daemon-rs/src/state.rs`: add `pub brain_firing: broadcast::Sender<BrainFiringEvent>` parallel to `pub events`. Define `BrainFiringEvent { kind: BrainKind, payload: Value, owner_id: Option<i64> }` and `BrainKind` enum mirroring §6.2.
- `daemon-rs/src/crystallize.rs`: emit `consolidation_started`, `member_added`, `cluster_finalized`, `link_inferred` at the appropriate code points (currently no events here). Each emit goes to `state.brain_firing.send(...)` — **not** the existing scrubbed `events` channel.
- `daemon-rs/src/handlers/recall.rs`: after the recall result is computed, emit `recall { node_ids, owner_id }` to `state.brain_firing`.
- `daemon-rs/src/handlers/events.rs`: add `handle_brain_firing_stream(State, Query)` mounted at `GET /brain/firing`, subscribes to `state.brain_firing`, validates the `?token=` query param, resolves caller's `owner_id`, filters per §6.4, batches per §6.3, emits SSE without scrubbing.

## 7. Render pipeline

- **Single `THREE.Scene`**. No composer, no postprocessing.
- **Cameras**: one `PerspectiveCamera` (fov=55, near=1, far=2000), positioned at `(0, 60, 380)` initially.
- **Renderer**: `THREE.WebGLRenderer` with `antialias=true`, `alpha=false`, `powerPreference="high-performance"`.
- **Tonemapping**: `THREE.LinearToneMapping` (default). Do **not** set `renderer.toneMapping = THREE.ACESFilmicToneMapping` — v1 lost frames there. Asserted by perf test.
- **Draw call ceiling**: target ≤ 25. Inventory: 2× core cages (separate meshes for counter-rotation; not merged), 1× core halo sprite, 1× satellite InstancedMesh body (all 150 satellites), 1× satellite halo InstancedMesh sprite, 1× merged beam `LineSegments` w/ pulse shader, ≤ 5 HUD text quads, slack 14. Asserted by perf test.
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
- **Bottom-left FIRING ticker**: scrolling text feed of last 5 events. Entry format: `cluster_finalized · 7 members · 42ms ago`. **rAF-batched**: SSE events queue into a ref; the next render frame applies at most one DOM update that swaps the entry list and runs `transform: translateY()` for entry advance — no layout thrash. Entries fade via `opacity` (compositor-only, no reflow) after 6 s.
- **Click-pin detail panel** appears in top-left when a satellite is selected: label, type, agent, member_count (clusters), recall count last 24 h, list of linked node IDs (top 5).
- **No legend, no MANUAL/AUTO toggle button** — the auto-rotate state is in the strip and toggles automatically on drag.

## 10. Performance budget

- **Reference machine**: Windows 11, Intel Core i7-12700H, integrated Iris Xe, Chrome stable, 1920×1080 viewport.
- **Target**: 60 fps median sustained, idle and active.
- **Draw calls**: ≤ 25 (asserted in unit test by sampling `renderer.info.render.calls`).
- **Vertex count**: ≤ 5 000 (icosahedrons + InstancedMesh + beams + halos).
- **No two-mode fallback.** If perf can't hold, the implementation is wrong, not the budget.

## 11. SSE client behavior

- Native `EventSource` to `/brain/firing?token=<token>` (URL-encoded). Tauri's webview supports `EventSource`; if a future Tauri build doesn't, fall back to `fetch` + `ReadableStream` SSE parser. The existing `api-client.js` IPC bridge (`invoke("fetch_cortex" / "post_cortex")`) is one-shot and not used here.
- **Reconnect**: native `EventSource` already auto-reconnects with browser-controlled backoff. We do **not** layer a manual backoff on top — that fights the browser. We log lifecycle (`open`, `error`, `close`) for diagnostics only.
- Each SSE message is a `brain_batch` event whose `data` is a JSON array of events (see §6.3). Client parses array and dispatches each event individually.
- Events buffered into a 256-deep FIFO in the renderer; consumed by the idle simulator (which suppresses fakes) and the firing engine.
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

- Snapshot harness uses headless `gl` (npm package) — **not a real GPU** — so output is deterministic across CI and local. Static frame: auto-rotate paused, seeded PRNG (idle simulator off), fixed cluster fixture. `pixelmatch` threshold 0.05. Baseline checked in after manual eyeball approval in PR.

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

- **Daemon emit work in `crystallize.rs`** — that file is 800+ LOC and the consolidation pipeline is non-trivial. Risk: misplaced emits leak memory text or fire too often. Mitigation: events carry only IDs/counts, never `text`; reviewer verifies emit sites; SSE batching in §6.3 caps fan-out.
- **Cluster cold-start gap** — if a fresh user has zero clusters and we render only the loose-memory ring, the constellation looks sparse. Mitigation: cold-start fallback in §4.3, with a same-slot crossfade (no rebuild) when first cluster materializes.
- **Cluster identity drift** — daemon may reincarnate a cluster under a new row ID with the same conceptual content (`find_existing_crystal(label)` keys by label, which can drift). Mitigation: hue is derived from a hash of the **centroid bytes**, not the row ID (§4.7). Same content → same hue, regardless of row recycling. A genuine centroid change correctly produces a hue shift.
- **`/forget` cascade** — `cluster_members` has `ON DELETE CASCADE`; user-initiated forget can orphan clusters. Mitigation: client treats absent cluster as "removed" — fade satellite out 400 ms; no stale palette mapping persisted client-side beyond the lifetime of the visible satellite.
- **SSE auth in Tauri** — token rides as `?token=` query param (browser `EventSource` cannot send custom headers). The token is ephemeral runtime-only, not a long-lived secret; logged URLs would still be sensitive — mitigation: never include path-with-query in any user-visible log or telemetry.
- **EventSource in Tauri webview** — Tauri uses platform webview (WebKit on macOS, WebView2 on Windows). Both support `EventSource`. If a future Tauri config disables it, fall back to `fetch` + `ReadableStream` SSE parser.
- **EventSource auto-reconnect** — already covered in §11; we do not fight the browser's built-in reconnect.
- **Bundle size after dropping force-graph + postprocessing** — current BrainVisualizer chunk ~1.42 MB / 383 KB gzip. Target post-rewrite: ≤ 600 KB / 200 KB gzip. Verified by `vite build` size diff in P7.
- **WebGL2 unavailability** — `hasWebGLSupport()` already exists in BrainVisualizer.jsx and gates a 2D fallback grid. Preserved in v2 (§8 layout). Detection covers WebGL1 + WebGL2 + experimental contexts.

## 15. Acceptance criteria

**Scene**

- v2 renders: counter-rotating core (two cages, opposite axes), single core halo sprite, three-tier shells (decisions inner R=80, clusters mid R=140, loose memories outer R=180-220 jittered), per-cluster centroid-hashed hue, no v1 anatomy / lattice / orbital rings / reticle / crosshair / orbital ellipses.
- Halo breathing: core halo intensity oscillates ±8 % on a 1.5 s sin cycle.
- Satellites bob ±2 % of their tier radius on individual phase offsets.
- Tooltip on hover shows label + type + agent + (for clusters) member_count.
- Click-pin detail panel shows label, type, agent, member_count, recall count last 24 h, top-5 linked node IDs.

**Behavior**

- Auto-rotate at 0.04 rad/s on Y axis from first paint; user drag pauses; resumes 8 s after last input.
- Real `/brain/firing` events trigger the correct visual response per §6.2 (consolidation_started → core halo pulse 1.0→1.2→1.0/800ms; member_added → beam member→centroid; cluster_finalized → cluster halo flash + cage scale pulse; link_inferred → A↔B beam; recall → per-node halo pulse + thin beam to core).
- Beam life: 600 ms (rise 80 ms cubic, exponential decay τ=280 ms).
- Camera spotlight: lerp factor 0.15 of camera→satellite vector over 800 ms cubic, then 400 ms return; only most recent event honored.
- 12 s idle threshold engages simulated firing; real event arrival immediately suppresses any in-flight fakes.
- Click-pin + right-click deselect both work; right-click suppresses native context menu.

**Daemon**

- `/brain/firing` endpoint mounted at `GET /brain/firing?token=...`.
- Owner scoping: events filtered against caller's resolved `owner_id`; fail-closed on missing owner.
- Public `/events/stream` scrub policy unchanged — verify by reading `handle_events_stream` after PR and asserting payload shape unchanged.
- Five event types (consolidation_started, member_added, cluster_finalized, link_inferred, recall) emitted from `crystallize.rs` and `handlers/recall.rs` to the `brain_firing` channel — not the existing `events` channel.
- SSE coalescing: `brain_batch` events whose `data` is a JSON array; max 50 ms window per batch.

**Disposal**

- v1 BrainVisualizer code, `react-force-graph-3d`, `postprocessing`, and all `brain/*` files (except ports to `brain-v2/`) are gone.
- Bundle size for the BrainVisualizer chunk: ≤ 600 KB raw / ≤ 200 KB gzip — measured by `vite build`.

**Perf**

- 60 fps median on the reference machine, idle and active under 10 events/s firing.
- Draw call count ≤ 25, vertex count ≤ 5 000 — both asserted by perf harness.
- Renderer tonemapping is `THREE.LinearToneMapping`; perf test asserts no ACES.
- Hover raycast rAF-throttled to ≤ 60 raycasts/s regardless of pointer rate.
- FIRING ticker DOM updates rAF-batched to ≤ 1 update/frame.

**Tests**

- All Vitest tests pass.
- Perf harness asserts draw call + vertex ceilings.
- Snapshot harness uses headless `gl` w/ pixelmatch threshold 0.05.

**Fallback**

- Existing 2D fallback path still triggers when WebGL2 is unavailable; trigger gated by the existing `hasWebGLSupport()` helper.

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

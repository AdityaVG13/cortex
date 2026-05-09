# Cortex Brain v2 — Implementation Plan

**PRD:** `specs/2026-05-09-cortex-brain-v2-prd.md`
**Date:** 2026-05-09
**Status:** Ready for execution
**Approach:** Phased delivery, ≤ 5 files per phase (daemon Rust + desktop JS counted together), verify after each, commit per phase, push after each.

## Phase ordering

```
P1 Daemon: /brain/firing channel + handler + emits
P2 Desktop: delete v1, scaffold brain-v2/, Scene + Core
P3 Desktop: Satellites (3 tiers + halos + cluster palette)
P4 Desktop: Beams (merged geometry + GLSL traveling pulse)
P5 Desktop: FiringClient (SSE) + IdleSimulator
P6 Desktop: Hover + Click-pin + Camera spotlight + HUD strip + Ticker
P7 Tests + perf harness + bundle size assertion + cleanup
```

P1 ships isolated (daemon-only); rest builds on top. Each phase verifies with cargo / npm scripts and a manual smoke note. Commit and push after green.

---

## Phase 1 — Daemon `/brain/firing` SSE

**Goal:** New SSE endpoint up, channel wired, emits in `crystallize.rs` and `recall.rs`. Public `/events/stream` unchanged.

**Files (≤ 5):**
1. `daemon-rs/src/state.rs` — EDIT. Add `pub brain_firing: broadcast::Sender<BrainFiringEvent>` parallel to `events`. Add `BrainFiringEvent { kind: BrainKind, payload: Value, owner_id: Option<i64> }` and `BrainKind` enum (`ConsolidationStarted | MemberAdded | ClusterFinalized | LinkInferred | Recall`). Initialize in `new_runtime_state`.
2. `daemon-rs/src/handlers/events.rs` — EDIT. Add `handle_brain_firing_stream(State<RuntimeState>, Query<BrainQuery>)` function: validate `?token=` against runtime token, resolve `owner_id`, subscribe to `state.brain_firing`, batch into 50 ms windows, filter by owner, emit `event: brain_batch / data: [...]` SSE messages without scrubbing. Fail-closed on missing/mismatched token (HTTP 401).
3. `daemon-rs/src/server.rs` — EDIT. Mount `GET /brain/firing` route → `handle_brain_firing_stream`. Add to allowlist at line ~1086 alongside `/events/stream`.
4. `daemon-rs/src/crystallize.rs` — EDIT. Insert `state.brain_firing.send(...)` calls at:
   - top of `run_consolidation` → `ConsolidationStarted`
   - inside member-insert loop in `update_cluster_members` → `MemberAdded { cluster_id, member_id }`
   - end of cluster row INSERT/UPDATE → `ClusterFinalized { cluster_id, member_count }`
   - any link-inference call (search & flag if there's no obvious site; if not present, defer `link_inferred` to P5 fake-only and revisit)
5. `daemon-rs/src/handlers/recall.rs` — EDIT. After computing the recall result + just before returning, `state.brain_firing.send(BrainFiringEvent { kind: Recall, payload: json!({"node_ids": ids}), owner_id })`.

**Verify:**
- `cargo test -p cortex-daemon` (or workspace test) passes.
- `cargo build` clean.
- `curl -N "http://127.0.0.1:7437/brain/firing?token=$TOKEN"` while triggering a recall via `/peek` → see `event: brain_batch` with array containing `{"type":"recall",...}`.
- `curl -N` with bad token → HTTP 401.
- `curl -N "http://127.0.0.1:7437/events/stream?token=$TOKEN"` still emits scrubbed `{type, timestamp}` only.

**Commit:** `feat(daemon): add /brain/firing SSE — owner-scoped, full payload, emits in crystallize + recall`

---

## Phase 2 — Desktop: delete v1, scaffold `brain-v2/`, Scene + Core

**Goal:** v1 gone, fresh `brain-v2/` tree mounting an empty scene with the counter-rotating core. Tab loads, no errors, no nodes yet.

**Files (≤ 5):**
1. `desktop/cortex-control-center/package.json` — EDIT. `npm uninstall react-force-graph-3d postprocessing`. Run `npm install` to update lockfile.
2. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — REWRITE. Becomes a thin wrapper: WebGL detection (preserve existing helper), 2D fallback path (preserve), otherwise mount `<BrainV2 />`. Remove all v1 useEffect blocks, refs, and ForceGraph3D usage.
3. `desktop/cortex-control-center/src/brain-v2/Scene.js` — NEW. Builds `THREE.Scene`, `PerspectiveCamera`, `WebGLRenderer` (antialias, alpha=false, high-performance, LinearToneMapping), `OrbitControls` (zoomSpeed 0.7, target origin). Single rAF loop calling registered ticks. Exports `createScene({ container })` returning `{ scene, camera, renderer, controls, registerTick(fn), dispose() }`.
4. `desktop/cortex-control-center/src/brain-v2/Core.js` — NEW. `createCore()` returns a `THREE.Group` containing two wireframe icosahedron `LineSegments` (radii 25, subdivision 1) + one halo `Sprite` (radius 80) using the shared halo texture. Exports `tickCore(group, t)` that counter-rotates outer +0.18 rad/s on Y, inner -0.32 rad/s on X+Y, and modulates halo intensity ±8 % on a 1.5 s sin cycle.
5. `desktop/cortex-control-center/src/brain-v2/Halo.js` — NEW. Pre-builds the radial-gradient RGBA texture once via offscreen `<canvas>` (64×64, white core fading to alpha 0). Exports `getHaloTexture()` (memoized).

**Removed in same phase (delete files):**
- `desktop/cortex-control-center/src/brain/ShellGeometry.js`
- `desktop/cortex-control-center/src/brain/ShellLayout.js`
- `desktop/cortex-control-center/src/brain/RenderLayers.js`
- `desktop/cortex-control-center/src/brain/PostFx.js`
- `desktop/cortex-control-center/src/brain/PulseShader.js`
- `desktop/cortex-control-center/src/brain/EdgeMesh.js`
- `desktop/cortex-control-center/src/brain/RippleEngine.js`
- `desktop/cortex-control-center/src/brain/__tests__/RippleEngine.test.js`
- `desktop/cortex-control-center/src/brain/__tests__/easing.test.js`

**Preserved (ported into brain-v2):**
- `desktop/cortex-control-center/src/brain-v2/util/easing.js` (NEW, ports v1's clamp01 + easeOutCubic + expDecay + riseDecay).

**Verify:**
- `npm test --run` green (existing tests need to drop assertions on deleted v1 code; update `brain-visualizer.test.js` to assert v2 wrapper + scene scaffolding).
- `npm run web:build` green; chunk size for BrainVisualizer drops sharply (verify in build output).
- Manual smoke: launch desktop app → Brain tab → see counter-rotating core w/ halo on near-black backdrop. Console clean.

**Commit:** `refactor(brain): delete v1, scaffold brain-v2 (Scene + Core + Halo)`

---

## Phase 3 — Satellites (3 tiers + halos + cluster palette)

**Goal:** ~150 satellites visible across three tiers (decisions inner / clusters mid / loose memories outer) with per-cluster centroid-hashed hue. Click + hover later; this phase is render-only.

**Files (≤ 5):**
1. `desktop/cortex-control-center/src/brain-v2/ClusterPalette.js` — NEW. `paletteForCluster(centroidBytes)` → `{ hue, saturation, lightness, color: THREE.Color }` via FNV-1a32 hash of bytes. Memoized by hash key.
2. `desktop/cortex-control-center/src/brain-v2/Satellites.js` — NEW. Builds a single `InstancedMesh` (sphere geometry r=1, count=150) for bodies + a separate `InstancedMesh` of additive sprite quads for halos. Exports `createSatellites({ scene })` returning `{ group, setData(payload), tick(t) }`. `setData` accepts `{ decisions[], clusters[], looseMemories[] }` and writes per-instance position/scale/color via `setMatrixAt` + `setColorAt`. Tier shells: decisions Fibonacci-on-sphere R=80, clusters R=140, loose R=180-220 with radial jitter via seeded PRNG. Bobbing applied per-frame on individual phase offsets ±2 % of tier radius.
3. `desktop/cortex-control-center/src/brain-v2/Tiers.js` — NEW. Pure data builder. `buildTiers(dump)` reads existing `/dump` payload, returns `{ decisions[], clusters[], looseMemories[] }` with size + position attributes. Implements §4.2 sizing (`clamp(log2(member_count+1)*1.4, 1.4, 4.0)` for clusters). Cold-start fallback (§4.3) handled: if `clusters` is empty, top 80 memories by score populate the mid ring with cyan placeholder palette.
4. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT. Wire `BrainV2` to call `fetchBrainData()` (existing API helper), pass to `Tiers.buildTiers`, hand to `Satellites.setData`.
5. `desktop/cortex-control-center/src/brain-v2/util/fnv1a.js` — NEW. 32-bit FNV-1a hash, used by `ClusterPalette`.

**Verify:**
- `npm test --run` green; new unit tests in P7 will cover layouts.
- `npm run web:build` green.
- Manual smoke: launch app → see core + 3 rings of satellites. Clusters in distinct hues. Decisions amber. Loose memories cyan, smaller. No interactions yet.
- `console.log(renderer.info.render.calls)` (temporary) ≤ 6 (core x2, halo, body InstancedMesh, halo InstancedMesh, +1).

**Commit:** `feat(brain): add Satellites — three-tier shells + per-cluster centroid-hashed hue + halos`

---

## Phase 4 — Beams (merged geometry + GLSL traveling pulse)

**Goal:** Beams visible when `fire(sourceId, targetId)` is called (manually wired in dev), with curved arcs + traveling pulse + decay.

**Files (≤ 5):**
1. `desktop/cortex-control-center/src/brain-v2/Beams.js` — NEW. Pre-allocates a 64-slot beam pool: single merged `LineSegments` BufferGeometry with 1024 verts (16 segments × 64 slots), attributes `aProgress` + `aBeamId` + `aLifetime`. Exports `createBeams({ scene })` returning `{ mesh, fire({ from, to, color, life }), tick(now) }`. `fire` finds an idle slot, writes 16 vertex positions along a bezier arc with midpoint lifted radially outward from origin, sets activation in the per-beam DataTexture, marks slot active. `tick` advances `uTime`, decays activation per slot via `riseDecay(t, 80, 280)`, reclaims slots when activation < 0.01.
2. `desktop/cortex-control-center/src/brain-v2/PulseShader.js` — NEW. Port v1's pulse shader (vertex passes vProgress + samples activation DataTexture; fragment computes pulse head shape via smoothstep + decays trail). Tweak: beam color is per-source hue, fading to white at the head — uniform `uHeadColor` + per-vertex hue attribute.
3. `desktop/cortex-control-center/src/brain-v2/util/bezierArc.js` — NEW. `bezierArc(from, to, segments, lift)` returns 17 points along a quadratic bezier whose control point is `(midpoint + midpoint.normalized() * lift * midLength)`. Used by Beams.
4. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT. Mount Beams in scene. Expose dev-only `window.__brainFire(fromId, toId)` for manual smoke until P5 wires the SSE.
5. `desktop/cortex-control-center/src/brain-v2/util/easing.js` — EDIT (already created in P2). Confirm `riseDecay` exported and used here.

**Verify:**
- `npm test --run` green.
- `npm run web:build` green.
- Manual smoke: open app → DevTools console → `window.__brainFire("decision-1","cluster-3")` → curved beam appears, traveling head, decays in ~600 ms. Multiple rapid calls reuse pool, no allocation.
- Verify draw calls still ≤ 8 after Beams add.

**Commit:** `feat(brain): add Beams — pooled merged-geometry pulse shader, bezier arcs`

---

## Phase 5 — FiringClient (SSE) + IdleSimulator

**Goal:** Real `/brain/firing` events drive Beams + Core halo + Satellite halos. Idle simulator covers gaps after 12 s of silence.

**Files (≤ 5):**
1. `desktop/cortex-control-center/src/brain-v2/FiringClient.js` — NEW. Wraps native `EventSource("/brain/firing?token=...")`. Logs `open / error / close` to console (lifecycle only — no manual backoff). Parses `brain_batch` events whose `data` is a JSON array. Dispatches each event to a registered handler. Exports `createFiringClient({ url, token, onEvent })` returning `{ disconnect() }`.
2. `desktop/cortex-control-center/src/brain-v2/IdleSimulator.js` — NEW. Tracks last real event timestamp. After 12 s, schedules fake beams every 4–8 s (mulberry32 PRNG, default seed `Date.now()`). Real event arrival cancels any pending fake (`clearTimeout`). Exports `createIdleSimulator({ onFakeBeam, getSatelliteIds, seed })` returning `{ noteRealEvent(), dispose() }`.
3. `desktop/cortex-control-center/src/brain-v2/EventDispatcher.js` — NEW. Routes parsed firing events to scene effects:
   - `consolidation_started` → `Core.pulseHalo()` 1.0→1.2→1.0 / 800 ms
   - `member_added` → `Beams.fire({ from: memberSat, to: clusterSat })`
   - `cluster_finalized` → `Satellites.flashHalo(clusterSat)` + cage scale pulse
   - `link_inferred` → `Beams.fire({ from: A, to: B })`
   - `recall` → for each node, `Satellites.pulseHalo(nodeSat)` + thin beam to core
4. `desktop/cortex-control-center/src/brain-v2/util/mulberry32.js` — NEW. Seeded PRNG (mulberry32). Exports `mulberry32(seed) → () => float in [0,1)`.
5. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT. Instantiate FiringClient + IdleSimulator + EventDispatcher; wire to existing `Beams`, `Satellites`, `Core` from prior phases. Pass auth token from existing `authToken` prop.

**Verify:**
- `npm test --run` green; new tests in P7 cover client + simulator.
- `npm run web:build` green.
- Manual smoke: launch app + daemon → Brain tab. Run a `/recall` from another agent → recall pulses + beams appear within 200 ms. Wait 12 s with no activity → simulated beams begin. Trigger a recall → fakes pause immediately.
- DevTools Network tab: `EventSource` connection to `/brain/firing?token=...`, status 200, `text/event-stream`.

**Commit:** `feat(brain): wire SSE firing client + 12s idle simulator + event dispatcher`

---

## Phase 6 — Hover + Click-pin + Camera spotlight + HUD strip + Ticker

**Goal:** All interaction wired. Auto-rotate by default, drag pauses, camera spotlights firing, hover tooltip, click-pin detail panel, slim HUD strip, FIRING ticker.

**Files (≤ 5):**
1. `desktop/cortex-control-center/src/brain-v2/Hover.js` — NEW. rAF-throttled raycast against the body InstancedMesh. Caches latest cursor position from pointermove. On render frame: at most one `Raycaster.intersectObject(instancedMesh)` call. Exports `createHover({ camera, instancedMesh, onHoverChange })` → `{ tick(), dispose() }`.
2. `desktop/cortex-control-center/src/brain-v2/Camera.js` — NEW. Auto-rotate (Y-axis 0.04 rad/s) with drag-pause + 8 s resume. `spotlight(satelliteWorldPos)` runs the §4.6 envelope (lerp 0.15, 800 ms easeOutCubic, 400 ms return). Hard-cuts in-flight spotlight on new event. Exports `createCamera({ camera, controls })` → `{ tick(now), spotlight(pos), pauseAutoRotate(), dispose() }`.
3. `desktop/cortex-control-center/src/brain-v2/Hud.jsx` — NEW. React component rendering:
   - top-right slim strip (NODES · CLUSTERS · DECISIONS · MEM · DEC · FPS · auto-rotate state)
   - bottom-left FIRING ticker (5 entries, rAF-batched updates, opacity fade 6 s)
   - top-left click-pin detail panel (label, type, agent, member_count, recall24h, top-5 linked)
   Exposes `pushFiringEntry(line)` and `setSelected(node)` via imperative ref.
4. `desktop/cortex-control-center/src/styles.css` — EDIT. Add `.brain-v2-hud`, `.brain-v2-ticker`, `.brain-v2-detail`, `.brain-v2-tooltip` rules. Use `transform` + `opacity` only for animations.
5. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT. Wire Hover, Camera, Hud. Click on hovered satellite → pin selection; right-click anywhere → clear selection + suppress context menu. Pass FPS rolling-median into HUD strip via `getInfo` callback. Camera.spotlight called from EventDispatcher on real events.

**Verify:**
- `npm test --run` green.
- `npm run web:build` green; bundle size measured + recorded in commit.
- Manual smoke: orbit + zoom feel smooth; auto-rotate runs; drag pauses then resumes 8 s; hover → tooltip; click → pin; right-click → deselect; firing event → camera spotlights briefly. Ticker scrolls in real time. HUD strip live.
- `Performance` tab: 60 fps median on reference machine, idle and active.

**Commit:** `feat(brain): interaction layer — hover, click-pin, camera spotlight, HUD strip + ticker`

---

## Phase 7 — Tests + perf harness + bundle size assertion + cleanup

**Goal:** Complete coverage, verify perf claims, lock the bundle size, sweep dead code.

**Files (≤ 5):**
1. `desktop/cortex-control-center/src/brain-v2/__tests__/Tiers.test.js` — NEW. Layout determinism (same input → same positions), tier population invariants, cold-start path (clusters empty → top-80 memories on mid ring), §4.2 sizing.
2. `desktop/cortex-control-center/src/brain-v2/__tests__/ClusterPalette.test.js` — NEW. FNV-1a32 reproducibility, golden-angle hue distribution (any two consecutive distinct hashes ≥ 30° apart on average), centroid bytes → same hue across runs.
3. `desktop/cortex-control-center/src/brain-v2/__tests__/Beams.test.js` — NEW. Pool reuse (100 sequential fires reuse ≤ 64 slots), expired slot reclaim, additivity (two fires on same slot index never alias).
4. `desktop/cortex-control-center/src/brain-v2/__tests__/IdleSimulator.test.js` — NEW. Real event resets timer; fake suppression on real arrival; 12 s threshold; mulberry32 reproducibility with fixed seed.
5. `desktop/cortex-control-center/src/brain-v2/__tests__/perf.test.js` — NEW. Headless `gl` package: build full scene (150 satellites, 64 active beams), render 600 frames, assert `renderer.info.render.calls ≤ 25`, vertex count ≤ 5 000, `renderer.toneMapping === THREE.LinearToneMapping`. Snapshot test (auto-rotate paused, seeded) → pixelmatch threshold 0.05.

**Plus daemon-side test (counted as P1's verify):**
- `daemon-rs/src/handlers/events.rs` — extend tests to assert `/brain/firing` returns 401 on bad token, filters by owner_id, batches in 50 ms windows. (If P1's tests already covered this, skip.)

**Cleanup pass (no new files):**
- Drop temporary `window.__brainFire` dev hook.
- Drop temporary `console.debug` of `renderer.info.render.calls`.
- Confirm v1 imports gone via `rg "react-force-graph-3d|postprocessing|brain/(ShellGeometry|ShellLayout|RenderLayers|PostFx|PulseShader|EdgeMesh|RippleEngine)"` → 0 hits in `desktop/cortex-control-center/src`.
- Run `npm run web:build` and capture chunk size; assert `BrainVisualizer-*.js ≤ 600 KB / ≤ 200 KB gzip` (commit message records the actual numbers).

**Verify:**
- `npm test --run` green.
- `npm run web:build` green; bundle within budget.
- Manual smoke: full reference-machine session — idle for 60 s, then 60 s under load (an automated `/recall` script firing every 5 s). 60 fps median in both. No console errors.

**Commit:** `test(brain): unit + perf + snapshot + bundle size assertion (cleanup pass)`

---

## Cross-cutting rules

- `rtk` prefix on every shell command per CLAUDE.md.
- No `git add .` — only stage files listed per phase.
- Re-read before editing. Max 3 edits per file between verification reads.
- Commit per phase, push per phase, never amend.
- Verify gate after each phase — type-check (Rust + JS), tests, manual smoke. No "deferred fixes" between phases.
- Run on the reference machine for any fps claim.
- Failures escalate after 3 attempts — stop, report, do not loop silently.

## Risk reserves

- **P1 daemon emit sites unclear** — `crystallize.rs` is 800+ LOC; if `link_inferred` has no obvious site, defer it (event type stays defined; client tolerates absence). Budget: 30 min decision.
- **P3 InstancedMesh + per-instance color** — Three.js requires `setColorAt` with a buffer; if instanced color attr setup fails, fall back to grouping satellites by tier into 3 separate InstancedMesh — adds 2 draw calls (still ≤ 25). Budget: 1 hour.
- **P4 GLSL compile failures on a target driver** — fall back to vanilla `THREE.LineBasicMaterial` for beams (no traveling pulse, additive line + opacity decay). Visually weaker but functional. Budget: 30 min decision.
- **P5 EventSource not supported in some Tauri build** — fall back to `fetch` + `ReadableStream` SSE parser. Budget: 1 hour.
- **P6 raycast against InstancedMesh hit testing wrong** — Three.js requires assigning each instance an `instanceMatrix`; verify hit test returns `instanceId`. If not, manual ray vs sphere test against per-instance positions. Budget: 1 hour.

## Out of scope (carry forward)

- Audio cues (Tone.js sub-pluck per beam).
- Mobile / touch.
- Saving / replaying firing sequences.
- Cluster split / merge events.
- Telemetry on fps to admin.
- Cinematic intro fly-in.
- Two-mode quality settings.

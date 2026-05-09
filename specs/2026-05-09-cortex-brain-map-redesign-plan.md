# Cortex Brain Map — Implementation Plan

**Spec:** `specs/2026-05-09-cortex-brain-map-redesign.md`
**Date:** 2026-05-09
**Status:** Ready for execution
**Approach:** Phased delivery, ≤ 5 files per phase, verify after each, commit per phase.

## Phase ordering

```
P1 Geometry foundation        →  P2 Render pipeline & layers
P2                             →  P3 Edge shader + merged geometry
P3                             →  P4 Ripple engine (BFS + DataTexture)
P4                             →  P5 Click flow + camera ease + dim/restore
P5                             →  P6 HUD overlay + SPLIT toggle + ripple path
P6                             →  P7 Tests (unit + perf + snapshot)
P7                             →  P8 Cleanup + flag wiring + removals
```

Each phase is independently buildable and visually verifiable in the running desktop app. Verify gate: type-check, lint, run tests, manually open Brain tab, smoke-test the phase's added behavior. Commit on green.

---

## Phase 1 — Geometry foundation

**Goal:** Replace anatomical hemisphere layout with shell-based positions. App renders nodes on outer/inner geodesic shells with orbital rings, no anatomy, no ripple yet.

**Files (≤ 5):**
1. `desktop/cortex-control-center/src/brain/ShellGeometry.js` — NEW. Builds outer icosphere (R=140, subdivision 2), inner icosphere (R=80), 3 ellipse rings (reuse current `ellipseRing` math), reticle ring, center crosshair. Exports `createConstellationShells()` returning a single `THREE.Group` named `cortex-constellation-shell`.
2. `desktop/cortex-control-center/src/brain/ShellLayout.js` — NEW. Exports `applyShellLayout(nodes, { useShellSplit })` projecting each node to its shell surface using a deterministic seeded hash. Memory → outer if `useShellSplit`, else outer for both. Decisions → inner if `useShellSplit`, else outer. Returns nodes with `{ x, y, z, shellRadius, brainRegion: 'outer'|'inner' }`.
3. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT. Replace `applyBrainLayout` import with `applyShellLayout`. Replace shell injection useEffect to use `createConstellationShells` instead of `createJarvisBrainShell`. Drop the `createBrainShapeForce` registration; add `createShellProjectionForce` that pulls each node's `(x,y,z)` toward `shellRadius * normalize(node.pos)`. Keep `useShellSplit` as `useState(true)`.
4. *(unused this phase)*
5. *(unused this phase)*

**Anatomical removal scope (this phase):** delete `BRAIN_REGIONS`, `brainRegionForNode`, hemisphere math in `brainLayoutPoint`, `cortexPath`, `hemisphereOutline`, `createJarvisBrainShell`. Keep `seededUnit`, `hashString` (reused by ShellLayout).

**Verify:**
- `pnpm tsc --noEmit` clean.
- `pnpm lint` clean.
- Open desktop app → Brain tab → see two faceted wireframe shells + 3 rings + reticle. Nodes scattered on shell surfaces. No two-blob brain shape at any zoom.
- HUD copy and stats unchanged.

**Commit:** `refactor(brain): replace anatomical layout with shell-based constellation geometry`

---

## Phase 2 — Render pipeline & layers

**Goal:** Selective bloom on emissive elements only, layer assignment, tonemapping, draw call ceiling enforced.

**Files:**
1. `desktop/cortex-control-center/src/brain/RenderLayers.js` — NEW. Exports `BRAIN_LAYERS = { BASE: 0, BLOOM: 1 }` and helper `assignLayer(object3d, layer)` walking children.
2. `desktop/cortex-control-center/src/brain/PostFx.jsx` — NEW. React component wrapping `<EffectComposer>` + selective `<Bloom>` from `@react-three/postprocessing`. Props: `enabled`, `intensity`, `threshold`, `smoothing`. Manages a 1-second rolling frame-time window via `useFrame`; calls `onAutoDegrade(disabled: boolean)` when median crosses thresholds (≥33.3 ms disable, ≤22 ms re-enable sustained 3 s).
3. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT. Add `bloomEnabled` state (default `true`). Mount `<PostFx>` inside the graph wrapper. Set `gl.toneMapping = ACESFilmicToneMapping` and `gl.toneMappingExposure = 1.0` via `ForceGraph3D` ref. Assign shells/rings/reticle to layer 0; nodes (instanced material) to layer 1.
4. `desktop/cortex-control-center/package.json` — EDIT. Add `@react-three/postprocessing` and `@react-three/fiber` (peer) if missing. Run `pnpm install`.
5. *(unused this phase)*

**Verify:**
- Bloom visible: nodes glow softly, shell wireframes stay crisp (no halo bleed).
- Open Chrome devtools perf → record 5 s of orbiting → median frame time < 22 ms on reference machine.
- Inject artificial 50 ms `setTimeout` stall in a scratch hook for 1.5 s → bloom disables; remove → bloom re-enables ≤ 4 s. (Manual smoke; perf test added later in P7.)
- `renderer.info.render.calls < 50` logged once after first paint (temporary `console.debug`, removed in P8).

**Commit:** `feat(brain): add selective bloom + auto-degrade + layer assignment`

---

## Phase 3 — Edge shader + merged geometry

**Goal:** Replace `react-force-graph-3d` per-link rendering with a single merged `BufferGeometry` driven by a custom `ShaderMaterial`. Static look only — no activation animation yet.

**Files:**
1. `desktop/cortex-control-center/src/brain/PulseShader.js` — NEW. Exports `createPulseMaterial({ baseColor, pulseColor })` returning a `THREE.ShaderMaterial` with uniforms `uTime`, `uActivation` (`DataTexture`, placeholder this phase), `uHeadPos`. Vertex shader passes `aEdgeId`, `aProgress` (0..1 along edge), `aActivation` from texture sample. Fragment shader: `gl_FragColor = mix(base, pulse, smoothstep(uHeadPos-0.05, uHeadPos, vProgress) * (1.0 - smoothstep(uHeadPos, uHeadPos+0.15, vProgress))) * (vActivation + 0.05)`.
2. `desktop/cortex-control-center/src/brain/EdgeMesh.js` — NEW. Builds a single merged `BufferGeometry` from `graphData.links`. Each link contributes 16 quad segments along its straight or arched path; attributes: `aPosition`, `aProgress`, `aEdgeId`. Exports `buildEdgeMesh(links, nodesById)` and `disposeEdgeMesh(mesh)`.
3. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT. Disable `react-force-graph-3d`'s built-in link rendering (`linkVisibility={() => false}`). Add the merged edge mesh to the scene as a sibling of the InstancedMesh. Rebuild only when `graphData.links` reference changes.
4. *(unused this phase)*
5. *(unused this phase)*

**Verify:**
- Edges still render, indistinguishable from prior look at static rest (just constant color).
- Draw call count drops measurably (shells 2 + rings 3 + nodes 1 + edges 1 + reticle 1 + crosshair 1 + HUD ≤ 6 = ≤ 14 < 50).
- No regression in hover/click selection (edges aren't clickable yet — not a regression; was already low-priority).
- Type-check, lint, existing tests pass.

**Commit:** `feat(brain): replace per-link rendering with merged geometry + pulse shader scaffold`

---

## Phase 4 — Ripple engine (BFS + DataTexture)

**Goal:** When a node is clicked, BFS runs and writes activation values to the edge `DataTexture`. Shader reads them and animates the traveling pulse along activated edges.

**Files:**
1. `desktop/cortex-control-center/src/brain/RippleEngine.js` — NEW. Class `RippleEngine` with:
   - `buildAdjacency(links)` — Map<nodeId, Array<{ neighborId, edgeIndex }>>.
   - `fire(nodeId, now)` — runs BFS depth ≤ 2, schedules activations into a Float32Array indexed by edge index, with `firstSeen[edgeIndex] = clickTime + depth × 110ms`.
   - `tick(now, gl)` — single rAF entry point: for each edge, computes `t = now - firstSeen[i]`; if `t >= 0`, writes `min(activation + risePart, 1.0) × exp(-(t-riseMs)/280)` into the DataTexture. Multiple ripples accumulate in `Float32Array activations` then clamp to 1.0 on flush.
   - Owns `THREE.DataTexture(activations, edgeCount, 1, RedFormat, FloatType)`; `texture.needsUpdate = true` on flush.
2. `desktop/cortex-control-center/src/brain/easing.js` — NEW. Exports `easeOutCubic(t)`, `expDecay(t, tau)`, `clamp01(x)`.
3. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT. Instantiate `RippleEngine` once on `graphData` change. Wire its `DataTexture` into `PulseShader`'s `uActivation`. Run `engine.tick(performance.now(), gl)` inside an `useFrame` hook (added via `<RippleTicker>` child component because `ForceGraph3D` doesn't expose `useFrame`).
4. `desktop/cortex-control-center/src/brain/RippleTicker.jsx` — NEW. Tiny component using `useFrame` to call `engine.tick()` each frame. Wired into the post-fx tree from P2.
5. *(unused this phase)*

**Hook into click:** keep this phase shader-only. Click already exists (`selectGraphNode`); add `engineRef.current.fire(node.id, performance.now())` inside it. Visible result: clicked node's outgoing edges glow with a traveling head.

**Verify:**
- Click a node → its outgoing edges show a pulse traveling outward, decaying within ~500 ms.
- Two rapid clicks on adjacent nodes → both ripples visible, edges shared by both glow brighter (additive, clamped at 1.0).
- No memory leak after 100 clicks (`renderer.info.memory.geometries` and `.textures` stable).
- Type-check, lint, existing tests pass.

**Commit:** `feat(brain): ripple engine — BFS-timed activation drives pulse shader`

---

## Phase 5 — Click flow polish: source pulse, secondary pop-in, camera ease, dim/restore

**Goal:** Round out the click-fire experience to match §5 of the spec.

**Files:**
1. `desktop/cortex-control-center/src/brain/NodeFx.jsx` — NEW. React component rendering source-node halo (additive sprite, scale pulse 1.0→1.4→1.0, 600 ms) and secondary ripple rings (additive sprites at depth-1/depth-2 nodes, 400 ms expand+fade). Driven by `RippleEngine` callbacks.
2. `desktop/cortex-control-center/src/brain/RippleEngine.js` — EDIT. Add observer pattern: `onSourceFire(cb)`, `onNodeReached(cb)`. `NodeFx` subscribes.
3. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT.
   - Replace `focusGraphNode` (full dolly) with `easeCameraToward(camera, target, 0.15)`.
   - Add dim/restore: when ripple active, drive `nodeOpacity` per-instance to 0.25 for non-affected nodes, 1.0 for affected, ease 200 ms in / 400 ms out.
   - Mount `<NodeFx />` inside the graph tree.
4. `desktop/cortex-control-center/src/brain/cameraEase.js` — NEW. Exports `easeCameraToward(camera, target, fraction)`.
5. *(unused this phase)*

**Verify:**
- Click node with several neighbors:
  - Source pulses 1.0→1.4→1.0 over 600 ms.
  - Edges show traveling glow.
  - Depth-1 nodes pop ripple ring on packet arrival; depth-2 nodes do too.
  - Non-affected nodes fade to ~25 % opacity, restore within 400 ms after ripple completes.
  - Camera eases ~15 % toward node, no re-center, user can still orbit.
- Total ripple visibly complete within ~900 ms.

**Commit:** `feat(brain): source pulse, secondary ripple, camera ease, dim/restore`

---

## Phase 6 — HUD overlay + SPLIT toggle + ripple path trace

**Goal:** Add Jarvis HUD chrome (corner glyph clusters, radar reticle), wire `useShellSplit` toggle, populate "RIPPLE PATH" mini-trace in the selection panel.

**Files:**
1. `desktop/cortex-control-center/src/brain/HudOverlay.jsx` — NEW. Four corner glyph clusters (DOM, absolutely positioned over canvas) with tick dials + status readouts: TL = nodes/links, TR = mem/dec/agents/last-fire timestamp, BL = bearing/zoom/depth, BR = small radar reticle (SVG arc + bearing needle reading from camera azimuth).
2. `desktop/cortex-control-center/src/styles.css` — EDIT. Add `.brain-hud-corner`, `.brain-hud-radar`, `.brain-hud-tick`, `.brain-split-toggle`. Keep existing brain HUD classes intact.
3. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT.
   - Mount `<HudOverlay>` inside `.brain-container`.
   - Add SPLIT pill button next to AUTO/MANUAL toggle; flips `useShellSplit`. Passes through to `applyShellLayout` on next layout pass.
   - Extend selection panel: render `selectedFlow.flowLinks` (already computed) as a labeled "RIPPLE PATH" tree showing depth-1 hop names + depth-2 hop counts.
4. *(unused this phase)*
5. *(unused this phase)*

**Verify:**
- Four corners populated, radar needle rotates with camera bearing.
- SPLIT pill toggles outer/inner-shell vs single-shell-color-only modes; layout re-applies cleanly without losing the camera.
- Selection panel shows RIPPLE PATH listing actual neighbors when a node is selected.
- Existing top-left card and top-right stats unchanged.

**Commit:** `feat(brain): jarvis HUD overlay, SPLIT toggle, ripple path trace`

---

## Phase 7 — Tests

**Goal:** Cover every §14 acceptance bullet with automated tests where possible.

**Files:**
1. `desktop/cortex-control-center/src/brain/__tests__/ShellLayout.test.js` — NEW. Deterministic positions across runs and across `graphData` reference changes that don't touch node ids.
2. `desktop/cortex-control-center/src/brain/__tests__/RippleEngine.test.js` — NEW. BFS depth cap = 2, additivity (clamp at 1.0), camera ease (15 %, no re-center), dim/restore (0.25 ± 0.01, 400 ms restore).
3. `desktop/cortex-control-center/src/brain/__tests__/easing.test.js` — NEW. `easeOutCubic` boundaries + monotonic, `expDecay(τ × ln(6)) ≈ 0.16`.
4. `desktop/cortex-control-center/src/brain/__tests__/perf.test.js` — NEW. Headless WebGL via `gl` npm package: render 1000 nodes / 300 links, assert `renderer.info.render.calls < 50`. Bloom auto-degrade trigger: inject 50 ms stalls for 1.5 s, assert disable; remove, assert re-enable within 4 s.
5. `desktop/cortex-control-center/src/brain/__tests__/snapshot.test.js` — NEW. Puppeteer launches the desktop app dev build, navigates to Brain tab, seeds fixed data, **disables bloom** in the harness, captures PNG, compares via `pixelmatch` 0.1 against `__snapshots__/constellation.png`. First run writes the baseline (manual approval gate in PR).

**Verify:**
- `pnpm test --run` green.
- `pnpm test perf.test.js` green on reference machine.
- Snapshot baseline reviewed visually before being checked in.

**Commit:** `test(brain): unit + perf + snapshot coverage for constellation lattice`

---

## Phase 8 — Cleanup + flag wiring + removals

**Goal:** Wire build-time flag, delete dead code, remove temporary debug logs.

**Files:**
1. `desktop/cortex-control-center/vite.config.js` — EDIT. Wire `CORTEX_BRAIN_LATTICE` env var; default `1`.
2. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT.
   - At entry: if `import.meta.env.VITE_CORTEX_BRAIN_LATTICE === '0'`, render the legacy renderer for the QA overlap window. (Else: lattice path.) Note: legacy renderer is already retained on disk only if Phase 1 deletion was deferred — given the spec says "delete in same PR", we delete now and ship lattice unconditionally; this Phase 8 file edit then becomes a no-op flag plumb that we keep for one release before removing.
   - Decision based on user signal at PR time: keep flag plumb or drop it. Default plan: drop, since spec says no shim.
   - Remove temporary `console.debug` for draw call count from P2.
3. `desktop/cortex-control-center/src/BrainVisualizer.jsx` — EDIT. Final removals if any anatomy code still lingered (audit pass).
4. `CHANGELOG.md` — EDIT. Add `## v0.6.x — Brain Map: Constellation Lattice` entry summarizing the redesign.
5. *(unused this phase)*

**Verify:**
- `rg -n "BRAIN_REGIONS|brainRegionForNode|createJarvisBrainShell|cortexPath|hemisphereOutline|applyBrainLayout" desktop/cortex-control-center/src/` returns zero hits.
- Build passes: `pnpm build`.
- Final manual smoke pass on reference machine: 1000-node load < 2 s, click-to-fire latency < 100 ms, ripple within 900 ms, 60 fps median, RIPPLE PATH HUD populated, SPLIT toggle works, 2D fallback still triggers when WebGL is forced off.
- All P1–P7 commits land cleanly on top of master.

**Commit:** `chore(brain): remove anatomy code, wire flag, changelog`

---

## Cross-cutting rules

- **No `git add .`** — only stage files listed per phase.
- **Re-read before editing.** Max 3 edits per file between verification reads.
- **Verify gate after each phase** — type-check, lint, tests, manual smoke. No "deferred fixes" between phases.
- **Commit per phase.** New commits, never amend.
- **Run on reference machine** for any fps claim.
- **Failures escalate after 3 attempts** — stop, report, do not loop silently.

## Risk reserves

- If P3 merged-geometry path causes hover flicker (the existing `nodeLabel` hover relies on per-link raycast that no longer exists): build a minimal raycaster against the InstancedMesh nodes only — links don't need hover. Spec doesn't require link hover. Budget: 2 hours.
- If P4 shader compile fails on a target driver: fall back to 2D grid (already specified in §13). Budget: 30 min decision.
- If P7 perf test in headless `gl` is too flaky: gate the draw-call assertion only; manual fps measurement on reference machine remains canonical.

## Out of scope (carry forward)

- Audio cues (Tone.js sub-pluck per hop).
- Mobile/touch ripple gestures.
- Saving/replaying ripple sequences.
- Any change to `/dump` endpoint or downstream Cortex APIs.

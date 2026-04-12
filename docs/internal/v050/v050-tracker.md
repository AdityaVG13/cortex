# Cortex v0.5.0 -- Completed Phases Tracker

Compressed record of all completed v0.5.0 work. Each entry includes commit hash, agent, and deliverables. Full task details were in `v050-implementation-plan.md` before being archived here.

---

## Phase 0: Baseline Benchmark -- DONE
- **Commit:** `6bdf63e` | **Agent:** CX (Codex)
- Ran full recall benchmark on v0.4.1. Baseline: GT precision 0.552, MRR 0.692, hit rate 0.900, avg latency 97.5ms. Results in `baseline-v041.md`, `baseline-v041-benchmark.json`.

## Phase 0A: Duplicate Purge -- DONE
- **Commit:** (reported by Codex in final response) | **Agent:** CX
- 29 memories purged, 0 decisions. Post-purge: GT precision 51.3%, MRR 0.74, hit rate 85.0%. FTS orphan check: pass.

## Phase 0C: Boot Savings Baseline Bug -- DONE
- **Commit:** `a6e5d9d` | **Agent:** SN (Sonnet)
- **Branch:** `feat/v050-task-0c`
- Replaced filesystem-scanning `estimate_raw_baseline()` with DB-based baseline (SQL SUM of all active memory/decision text). Boot savings now correct for all agents regardless of CWD.

## Phase 1: Tiered Retrieval + RRF Fusion -- DONE
- **Commits:** `ed1d3a7` (1.1-1.2), `33def16` (1.3), `082f275` (1.4), `ad74a92` (1.5) | **Agents:** SN, D5, CC
- **Branch:** `feat/v050-phase-1-retrieval`
- Full tiered retrieval: Tier 0/1 query cache, FTS5 field boosting + synonym expansion, RRF fusion (k=60), compound scoring (BM25*0.6 + importance*0.2 + recency*0.2). 81 tests passing.

## Phase 3A: Schema Versioning -- DONE
- **Commit:** `145766b` | **Agent:** D4 (GLM-4.7)
- `schema_migrations` table, named migration runner on startup, `cortex doctor` CLI for schema verification.

## Phase 3B: TTL / Hard Expiration -- DONE
- **Commit:** `4b617a3` | **Agent:** CX (Codex)
- Added explicit TTL expiration coverage for stored decisions so `ttl_seconds` produces an `expires_at` timestamp while entries without TTL remain persistent.
- Hardened recall coverage to ensure expired memories and decisions are filtered out of active search results once their TTL has passed.
- Added shared expired-row cleanup in the Rust DB layer and wired it into the daemon's startup / 6-hour maintenance loop so expired rows are eventually deleted instead of bloating the SQLite file.

## Phase 3C.3: Clippy CI Gate + Warning Cleanup -- DONE
- **Commits:** `fdd3f25` (warning cleanup), `8b6ed2c` (CI gate) | **Agent:** CX (Codex)
- Fixed the current `cargo clippy --all-targets` warning set so the daemon crate is clean under strict warning enforcement.
- Added a dedicated GitHub Actions clippy job that runs `rtk cargo clippy --all-targets -- -D warnings` alongside the existing Rust checks, turning warnings into a CI failure.

## Phase 5A: Startup Integrity Gate -- DONE
- **Commits:** `3576c5c` (5A.1), `a82d747` (5A.2-5A.3) | **Agent:** CC
- **Branch:** `feat/v050-phase-5a`
- `PRAGMA integrity_check` on startup, `PRAGMA quick_check` every 30m background task, auto-repair via dump-and-rebuild.

## Phase 5B: Rolling Backups -- DONE
- **Commit:** `980f66b` | **Agent:** CC
- **Branch:** `feat/v050-phase-5bc`
- Rolling daily backups on WAL checkpoint. `cortex backup` and `cortex restore <file>` CLI commands.

## Phase 5C: Crash-Safe WAL Handling -- DONE
- **Commit:** `980f66b` | **Agent:** CC
- **Branch:** `feat/v050-phase-5bc`
- WAL checkpoint every 10s (was 60s), startup WAL recovery, `PRAGMA synchronous` verification.

## Phase 5E: Storage Compression + Retention Policy -- DONE
- **Commits:** `b4878f0` (5E.1), `cb3459d` (5E.2), `2a619f5` (5E.3), `513b708` (5E.4), `c4d35e1` (5E.5), `5fbfb39` (5E.6), `1cb0db5` (5E.7) | **Agent:** CX (Codex)
- Startup now keeps only the 3 most recent `~/.cortex/backups/*.db` files by modified time and logs the retention cleanup count.
- Added schema-version-gated cleanup for legacy `~/.cortex/bridge-backups/` once `PRAGMA user_version >= 5`.
- Added startup log rotation for oversized `.cortex` log files, keeping only one `.1` generation per tracked log.
- Restored MCP write-buffer draining and compaction so `write_buffer.jsonl` is truncated after successful replay instead of growing forever.
- Replaced the old stale-daemon kill path with dead-process detection that removes stale `cortex.pid` / `cortex.lock` files without killing live processes.
- Added `cortex cleanup --dry-run` / `cortex cleanup` to preview or execute backup, log, bridge-backup, and stale PID cleanup with one-line actionable output.
- Added `/health` storage metrics: `storage_bytes`, `backup_count`, and `log_bytes`.

## Phase 7A: MCP Proxy Session Re-registration -- DONE
- **Commit:** `7081dc1` | **Agent:** D4
- `POST /session/start` on daemon respawn in `mcp_proxy.rs`. Agents panel auto-repopulates. Reconnect flow hardened with session telemetry.

## Phase 7B: Immediate UI State Reflection -- DONE
- **Commit:** `d5d58cd` | **Agent:** D4
- Stop/Start buttons immediately update UI. Agents panel clears on stop, shows "Starting..." then "Running" on health check. Lifecycle commands non-blocking, port cached.

## Phase 7C: Connectivity + Auth Hardening -- DONE
- **Commit:** `9d9b318` | **Agent:** CX (Codex)
- Fixed MCP/HTTP health drift by moving `cortex_health` onto the same payload builder as `/health`, including degraded/db/runtime fields.
- Fixed Codex setup docs and installer flow to use current `codex mcp add cortex -- <exe> mcp` syntax and documented that MCP servers added mid-session require a new Codex session.
- Fixed HTTP usage docs and smoke coverage so protected endpoints consistently include `Authorization: Bearer <token>` and `X-Cortex-Request: true`.
- Relaxed SSRF header parsing so any non-empty `X-Cortex-Request` value is accepted in new builds, preventing false 403s from header-value casing differences.
- Made direct `cortex mcp` startup ensure the daemon without polluting stdio output, while keeping `plugin ensure-daemon` port output for existing callers.

## Phase 7D: CLI Troubleshooting Entry Point -- DONE
- **Commit:** `76f305d` | **Agent:** CX (Codex)
- Added a troubleshooting section to `cortex --help` so users can discover `cortex doctor`, the required HTTP auth headers, the Codex MCP hot-attach limitation, and the app-hosted daemon restart path without reading repo docs first.
- Added README guidance pointing users to `cortex --help`, `cortex doctor`, and `Info/connecting.md` as the primary recovery path for connectivity and auth issues.

## Phase 7E: Review Fixes for Feed + Desktop Auth Retry -- DONE
- **Commit:** `261574e` | **Agent:** CX (Codex)
- Fixed `GET /feed?unread=true` so a stale `feed_acks.last_seen_id` no longer suppresses every unread item after feed TTL pruning removes the anchor row.
- Added daemon regression tests covering both the stale-ack fallback path and the normal "after ack, skip self entries" unread path.
- Fixed Cortex Control Center POST requests to refresh and retry once after missing/stale auth tokens, matching the existing GET behavior during daemon token rotation.
- Added desktop regression tests covering POST token refresh before first call and retry-after-401 flows for both IPC and browser fallback.

## Phase 7F: Duplicate Serve Startup Lock Guard -- DONE
- **Commit:** `286d781` | **Agent:** CX (Codex)
- Fixed direct `cortex serve` startup so the daemon acquires the singleton lock before state initialization, preventing a second launch from rotating `~/.cortex/cortex.token` and then dying on port bind.
- Kept stale PID / lock recovery in the startup path by folding the existing dead-process cleanup into the pre-start lock acquisition instead of removing stale-file cleanup entirely.
- Added a daemon regression test covering duplicate runtime lock acquisition so a second `serve` attempt fails before mutating shared auth state.

## Phase 7G: Desktop + Plugin Lifecycle Hardening -- DONE
- **Commit:** `0b0b0fc` | **Agent:** CX (Codex)
- Fixed Cortex Control Center auth handling so protected panels stop retrying forever on the same stale token, surface one combined panel-auth error, and keep the new live-session work surface normalized in the desktop app.
- Replaced the desktop daemon reachability check with a real Cortex `/health` probe, preserved managed sidecar state on failed/timed-out stops, and added regression coverage for the stricter health-signature checks.
- Scrubbed `/events` payloads down to safe metadata only, made MCP bridge teardown explicitly end sessions, and tightened the Claude plugin startup path so it validates real Cortex health responses and registers as `claude-code` instead of a generic `mcp` agent.

## Phase 7H: Desktop Auth Banner Cleanup -- DONE
- **Commit:** `4d108a6` | **Agent:** CX (Codex)
- Removed the redundant first-run Messages / Activity refresh race in Cortex Control Center so those panels stop issuing duplicate protected requests during the initial dashboard hydrate.
- Cleared transient desktop auth warnings after the next successful protected refresh so stale sidebar messages like "Activity could not authenticate" do not linger once the daemon and token are healthy again.

## Phase 7I: Desktop Dev Build Target Isolation -- DONE
- **Commit:** `eebcca2` | **Agent:** CX (Codex)
- Moved Control Center daemon builds off the shared `daemon-rs/target/{debug,release}` paths so `npm run dev` and `tauri build` stop failing when a live Cortex process has the workspace executable locked on Windows.
- Taught the Tauri sidecar copy step and desktop runtime binary discovery to prefer the new isolated control-center targets first, so dev/release app builds still launch the freshly-built daemon instead of an older shared binary.

## Phase 7J: Desktop Dev Watcher + Warning Cleanup -- DONE
- **Commit:** `0245aed` | **Agent:** CX (Codex)
- Marked the daemon event payload bus field as intentionally retained-but-redacted so Control Center dev builds stop emitting the noisy unused-field warning on every startup.
- Added a Tauri dev watcher ignore for `src-tauri/binaries/` and stopped the sidecar copy step from rewriting identical binaries, so desktop dev runs no longer self-trigger a redundant rebuild just because the sidecar artifact was refreshed.

## Phase 7K: Desktop Dev Runtime Copy Launcher -- DONE
- **Commit:** `b02cd31` | **Agent:** CX (Codex)
- Changed the Control Center sidecar launcher so debug desktop sessions no longer spawn Cortex directly from the workspace `daemon-rs` build output, which was still locking the dev target binary while the app was open on Windows.
- Added a managed runtime-copy path under the user's Cortex runtime directory plus stale-copy cleanup, so debug sessions can relaunch Cortex from a disposable copy while packaged release behavior stays unchanged.

## Phase 7L: Desktop Runtime Copy Session Isolation -- DONE
- **Commit:** `ef77822` | **Agent:** CX (Codex)
- Scoped each debug Control Center runtime-copy directory to the current app session instead of sharing one global temp folder, so one desktop window can no longer delete another session's managed daemon copy during cleanup.
- Added regression coverage for the session-scoped runtime path so the Windows dev-only launcher keeps the build-unlock behavior from 7K without introducing cross-session temp-copy collisions.

## Phase 7M: Runtime Auth Token-Path Isolation -- DONE
- **Commit:** `5567cec` | **Agent:** CX (Codex)
- Fixed daemon startup so auth token generation and reads use the resolved runtime home path instead of the shared default `~/.cortex/cortex.token`.
- Prevented override-home daemons, benchmark temp-home runs, and the app-managed desktop daemon from clobbering each other's auth state.
- Unblocked `tests/recall_benchmark.rs` from hanging on token waits; it now reaches the real benchmark-threshold assertion instead of deadlocking.

## Phase 7N: Control Center Live-Surface Stabilization -- DONE
- **Commit:** `7cca348` | **Agent:** CX (Codex)
- Finished the modern Work surface wiring around the real daemon endpoints, including persisted operator selection plus claim, complete, unlock, message, and feed-ack flows.
- Added the mock-backed `expect-cli` smoke harness and the faster work-scoped smoke path so desktop verification can be rerun without the slower full browser pass every iteration.
- Stabilized the shell and renderer surfaces by replacing glyph-dependent icons with deterministic inline SVGs, reflecting offline state cleanly after Stop, auto-collapsing the sidebar on narrow viewports, and making the Brain legend/layout pass the overview smoke on mobile.

## Phase 7O: Control Center Lifecycle Recovery Hardening -- DONE
- **Commit:** `6930a2c` | **Agent:** CX (Codex)
- Hardened long-lived Control Center and plugin recovery paths so stale token rotation, delayed auth-token writes, daemon restarts, and externally managed daemon states no longer strand clients on stale auth or partial-disconnect failures.
- Tightened daemon-state truthfulness end to end by surfacing `managed` / `authTokenReady` status from Tauri, validating real `/health` payload shape in the MCP proxy and mock harness, and fixing Work-surface operator canonicalization plus 375px mobile overflow/accessibility smoke blockers.

## Phase 7P: Control Center Runtime Truth + Motion Polish -- DONE
- **Commit:** `2840688` | **Agent:** CX (Codex)
- Added automatic recovery retries plus smoother panel/topbar transitions so long-lived desktop sessions recover more cleanly during daemon startup, token rotation, and panel changes without the shell feeling as snappy or misleading.
- Surfaced live daemon/runtime truth in the shell by normalizing app/daemon `0.5.0` version metadata, showing precise runtime-version mismatch recovery guidance, and disabling invalid Start/Stop actions when the live daemon state makes them nonsensical.
- Fixed the Monte Carlo projection chart to scale correctly at `375x812`, and hardened debug binary selection so the Control Center prefers the freshest available workspace daemon build instead of blindly reusing a stale isolated target.

## Phase 7Q: Runtime Ownership + Stable MCP Binary Paths -- DONE
- **Commit:** `0211aac` | **Agent:** CX (Codex)
- Stopped long-lived desktop and plugin sessions from pinning workspace daemon build artifacts by making daemon lifecycle respawns launch from a disposable runtime copy when the current executable lives under the repo `daemon-rs/` tree.
- Changed Control Center editor setup and CLI `cortex setup` to refresh and register a stable `~/.cortex/bin/cortex(.exe)` path instead of wiring MCP clients to workspace `target*/cortex` outputs during dev.
- Updated MCP registration flows to rewrite stale Cortex entries in place, so rerunning setup repairs existing editor configs instead of leaving older workspace-binary commands untouched.

## Phase 7R: Control Center Panel Recovery Cleanup -- DONE
- **Commit:** `090baef` | **Agent:** CX (Codex)
- Stabilized the recall-quality headline so undersampled live days no longer drag the primary metric off a one-query miss spike, while still preserving the live daily trend in the strip below.
- Removed the animated hidden-topbar path on Overview and kept the Brain panel mounted after first visit, which cuts the worst panel-switch hitch from re-entering Brain and from Overview-to-surface swaps.
- Gated Brain resize/auto-rotate work behind active visibility so the 3D surface stops doing background animation work while another panel is in focus.

## Phase 7S: Brain Agent-Color Readability -- DONE
- **Commit:** `69344ca` | **Agent:** CX (Codex)
- Reworked 3D brain node materials so each node reads as its agent/provider color at default zoom instead of collapsing into a uniform bright wash.
- Added a brighter inner nucleus and toned the outer shell/glow balance so the color identity remains visible without having to zoom all the way into the graph.

## Phase 6A: Public README + Research Redesign -- DONE
- **Commit:** `8a6fdcc` | **Agent:** CX (Codex)
- Rebuilt `README.md` into a stronger landing page with clearer product framing, proof-driven sections, benchmark-backed metrics, sharper nav, and proper `Research` / `Code of Conduct` surfacing in repo-controlled navigation.
- Expanded `Info/research.md` from a paper list into a public design record with richer per-reference adaptation notes, stronger `Inspired by` wording for open-source influences, and explicit shipped / planned / deferred status.
- Added `assets/proof-surface.svg` and `assets/research-lineage.svg` so the public docs now carry a consistent premium visual language instead of relying on plain text alone.

## Phase 6B: Research Visual Refinement -- DONE
- **Commit:** `501be51` | **Agent:** CX (Codex)
- Replaced the original research lineage graphic with a cleaner, more legible developer-tool layout: less diagram noise, stronger hierarchy, clearer research-to-product mapping, and a more premium black / emerald / mono aesthetic.
- Tightened the README research section so the graphic is explicitly framed as the readable overview, while `Info/research.md` remains the full paper-by-paper adaptation record.

## Phase 6C: Analytics Visual Redesign -- DONE
- **Commit:** `aedf364` | **Agent:** CX (Codex)
- Rebuilt the Analytics surface in Cortex Control Center around a clearer product hierarchy: premium header, stronger metric cards, a Monte Carlo projection hero, tighter secondary charts, improved heatmap styling, and balanced lower list panels.
- Added deterministic client-side Monte Carlo projection logic so the analytics page can visualize 30-day cumulative savings bands without needing new daemon endpoints.
- Refined the chart styling system with cleaner SVG geometry, projection bands, lighter grid treatment, and responsive layout rules so the page reads more like a polished developer product surface and less like a generic dashboard stack.

## Phase 6D: README Monte Carlo Proof Surface -- DONE
- **Commit:** `632da53` | **Agent:** CX (Codex)
- Added a dedicated Monte Carlo proof graphic to the public README so the compounding-savings story now appears on the landing page instead of living only inside the app analytics surface.
- Created a tracked SVG proof asset under `assets/` so the README uses a repo-owned visual rather than depending on local benchmark imagery or screenshots.

## Phase 6E: README Landing Page Polish -- DONE
- **Commit:** `43a0bd8` | **Agent:** CX (Codex)
- Tightened the README hero copy and simplified the top navigation so the landing page reads more like a premium product surface and less like a raw doc index.
- Added a second tracked SVG system visual and reworked the top-half section flow so Proof, Monte Carlo, and the Cortex operating model all read as one designed narrative.

## Phase 6F: README Real Visual Rewrite -- DONE
- **Commit:** `edd39a3` | **Agent:** CX (Codex)
- Replaced the synthetic README proof surfaces with real product proof: the cleaner Control Center analytics screenshot and a restrained animated Monte Carlo savings reveal built from benchmark imagery.
- Rebuilt the README flow around a proof-first narrative, cleaner quickstart, corrected public doc links, and tighter product copy so the page sells Cortex before it starts enumerating internals.
- Removed the earlier placeholder SVG surfaces from the shipped asset set so the public repo only carries visuals that match the current product direction.

## Phase 6G: README Render Correction -- DONE
- **Commit:** `ea151df` | **Agent:** CX (Codex)
- Removed the GitHub-unsafe HTML and Markdown mixing that leaked raw `</td>` tags into the public README render.
- Fixed the broken badge URL in the hero, restored the support link near the top of the page, and converted the affected sections to render-safe Markdown lists and tables.
- Kept the real analytics screenshot and Monte Carlo proof asset, but stripped out the layout patterns that looked broken or off-center in GitHub.

## Phase 6H: README Art Direction Pass -- DONE
- **Commit:** `d729b98` | **Agent:** CX (Codex)
- Moved the install path higher in the README so visitors can see how to start before they hit the deeper product documentation.
- Reworked the middle of the page into a clearer product story: "without Cortex / with Cortex," simpler shipped-capabilities copy, and a less table-heavy research summary.
- Kept the real proof assets, but tightened the pacing so the README now reads more like a product landing page and less like a technical inventory.

## Phase 6I: Hero Proof Strip + Monte Carlo Motion Polish -- DONE
- **Commit:** `8ec93c0` | **Agent:** CX (Codex)
- Replaced the top README metric table with a cropped proof strip from the real analytics surface so the hero now carries product UI instead of generic stat cards.
- Rebuilt the Monte Carlo GIF so the chart stays static while the forecast lines animate, then enlarged the end-state callout numbers for better readability on GitHub.
- Kept the README render-safe while pushing the hero zone closer to a premium product-launch presentation.

## Phase 6J: Hero Minimalism Pass -- DONE
- **Commit:** `0b98ccd` | **Agent:** CX (Codex)
- Re-cropped `assets/hero-proof-strip.png` taller after GitHub review showed the metric cards were being clipped too aggressively in the hero.
- Simplified the hero copy and metadata treatment: stronger headline, fewer top-level links, and a quieter metadata line in place of badge-heavy chrome.
- Kept the same real analytics surface, but adjusted the crop and pacing so the top of the README reads more like a product launch and less like a standard GitHub template.

## Phase 6K: Proof Surface Simplification -- DONE
- **Commit:** `904c35c` | **Agent:** CX (Codex)
- Removed the cropped hero proof strip after review showed it was weaker than a simple centered metric-box layout.
- Replaced the Monte Carlo GIF with the verified static PNG and removed the rejected animated asset from the shipped README.
- Kept the README focused on cleaner static proof elements while continuing the broader Apple-style cleanup pass.

## Phase 6L: Middle Section Card Layout Pass -- DONE
- **Commit:** `9b7df79` | **Agent:** CX (Codex)
- Rebuilt the middle of the README around centered text-only card grids for the "why," "stack," "what ships," and "documentation" sections.
- Avoided the prior GitHub rendering failure mode by keeping the box cells free of embedded markdown headings and list syntax.
- Kept the proof assets static and verified while shifting the page body closer to a calmer, more premium product layout.

## Phase 6M: Middle Section Readability Correction -- DONE
- **Commit:** `fcc9be3` | **Agent:** CX (Codex)
- Reduced the middle-section card layouts from denser three-column blocks to larger two-column rows where needed.
- Removed the tiny `sub` styling from card copy so the text reads at a normal size on GitHub.
- Kept the centered-box treatment, but shifted the emphasis from compression to legibility.

## Phase 6N: README Section Header Bars -- DONE
- **Commit:** `eac54d0` | **Agent:** CX (Codex)
- Added five custom SVG section bars to the public README so the page can carry its own visual identity instead of relying only on GitHub-native white markdown headers.
- Used the new bars selectively on the proof, why, stack, shipping, and documentation sections, keeping the body copy render-safe and the rest of the layout intact.
- Rendered the bars through a local browser preview sheet before commit so the shipped visuals were verified as images instead of approved from source alone.

## Phase 6O: README Header Bar Removal -- DONE
- **Commit:** `40f04dd` | **Agent:** CX (Codex)
- Removed the custom SVG section bars after review showed they were fighting the body copy and making the README feel less natural.
- Restored standard GitHub headers for the proof, why, stack, shipping, and documentation sections while keeping the simpler plain-language copy underneath.
- Deleted the now-unused section-bar assets so the public repo no longer carries abandoned README chrome.

## Phase 6P: README Copy Simplification -- DONE
- **Commit:** `69b3f32` | **Agent:** CX (Codex)
- Rewrote the hero, proof, compatibility, shipping, and documentation copy in plainer language so the README reads like a product people can use, not an internal AI tooling explainer.
- Simplified the top metadata and stat labels to reduce jargon while keeping the same proof points.
- Kept the existing visuals and layout, focusing this pass only on wording, readability, and pacing.

## Phase 6Q: README Proof Context Correction -- DONE
- **Commit:** `be3c5fa` | **Agent:** CX (Codex)
- Replaced the personalized hero stat row so the top of the README now shows product-level signals instead of one machine's cumulative savings.
- Updated the analytics caption and source note to make clear that the screenshot is one real active install, not a guaranteed day-one result for every user.
- Kept the proof section intact while removing the most misleading "works on my machine" implication from the page.

## Phase 6R: README Monte Carlo Redesign -- DONE
- **Commit:** `be3c5fa` | **Agent:** CX (Codex)
- Replaced the old dual-axis Monte Carlo chart with a cleaner single-axis fan chart built from Aditya's live Cortex savings history.
- Added a tracked generator script so the README Monte Carlo asset is reproducible instead of being a one-off exported image.
- Labeled the chart and README copy explicitly as an example projection based on Aditya's own data, not a promise of every new user's day-one savings.

## Phase 6S: README Stack Compatibility Clarification -- DONE
- **Commit:** `07a2c81` | **Agent:** CX (Codex)
- Updated the "Works with your stack" section so it no longer reads like Cortex only works with a short named list of tools.
- Clarified that MCP works anywhere it is supported, and HTTP works for any AI or tool that can call an API.
- Kept the named examples, but shifted them into examples of the integration surface instead of an exhaustive compatibility list.

## Phase 6T: README Hero Simplification -- DONE
- **Commit:** `fd3cc82` | **Agent:** CX (Codex)
- Simplified the top of the README to one clear promise: one shared memory across tools, without the extra hero chrome competing for attention.
- Removed the top support and mission copy, tightened the CTA row, and replaced the pseudo-dashboard metrics with three quieter trust boxes.
- Verified the new hero composition with a local preview before commit so the spacing and box layout were checked as rendered, not just in source.

## Phase 6U: README Support Link Restoration -- DONE
- **Commit:** `ff58675` | **Agent:** CX (Codex)
- Restored the donation link near the top of the README after the hero simplification pass removed it.
- Kept the link quieter than the old version by placing it under the trust boxes instead of turning it back into a competing hero element.
- Verified the updated hero composition with a local rendered preview before commit.

---

## Branches Awaiting Merge to Master
| Branch | Phase | Status |
|--------|-------|--------|
| `feat/v050-task-0c` | 0C | Done, merge ready |
| `feat/v050-phase-5a` | 5A | Done, merge ready |
| `feat/v050-phase-5bc` | 5B+5C | Done, merge ready |
| `feat/v050-phase-1-retrieval` | 1 | Done, merge ready |

---

*Last updated: 2026-04-11*

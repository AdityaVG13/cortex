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

---

## Branches Awaiting Merge to Master
| Branch | Phase | Status |
|--------|-------|--------|
| `feat/v050-task-0c` | 0C | Done, merge ready |
| `feat/v050-phase-5a` | 5A | Done, merge ready |
| `feat/v050-phase-5bc` | 5B+5C | Done, merge ready |
| `feat/v050-phase-1-retrieval` | 1 | Done, merge ready |

---

*Last updated: 2026-04-10*

# Architecture

Cortex is a private, local-first memory library for AI tools. `cortex-kernel` owns SQLite semantics; the `cortex` executable provides in-process CLI, hooks and MCP stdio. Asupersync owns runtime capabilities, locks, channels and timers. No HTTP listener or proxy is built. The separate Control Center and HTTP SDKs are legacy clients for earlier tagged releases, not clients of this runtime.

Clock-Quorum Recall (CQR) remains the production semantic retrieval engine. The unreleased V5 observation path adds separately labeled exact any-cue retrieval; it does not promote captured prose into CQR facts. The daemon does not download, load, or run language, embedding, or reranking models. Older databases may still contain inert `embeddings` rows; they are not read. `~/.cortex/models` is neither required nor created.

Current version: **0.6.0**.

---

## Shipped vs proposed

This file describes shipped behavior. The design pack under
`docs/architecture/next/` is a proposal path; its ideas that have landed are
listed in `docs/architecture/next/README.md` with the contract that holds
each one. Guides for users, developers and operators live in `docs/guides/`.

## Products in this tree

| Product | Path | Role |
|---------|------|------|
| Kernel | `crates/kernel` (`cortex-kernel`) | The embeddable brain: `CortexRuntime::open / deposit / lens / boot` in-process over SQLite — CQR engines, store path, boot compiler, schema, Reflex, hooks. No axum, no server, no port. Rust hosts (Outfit) path-dep this crate |
| CLI / MCP edge | `crates/daemon` (`cortex-daemon`, binary `cortex`) | Direct kernel access, MCP stdio, hooks, and an optional bounded maintenance worker. No network listener or service manager. |
| Logic | `crates/logic` (`cortex-logic`) | Deterministic types: clocks, graph, traces, conflict, budgets |
| Tests | `tests/` (`cortex-tests`) | Public contracts. Production crates have no inline tests |
| Legacy Control Center | `desktop/cortex-control-center` | Separate workspace; requires the earlier HTTP daemon |
| Plugin | `plugins/cortex-plugin` | Invokes local MCP stdio and in-process hooks |
| Legacy SDKs | `sdks/python`, `sdks/typescript` | HTTP clients for earlier releases; not supported by this runtime |

---

## Runtime surfaces

| Surface | How to start | Notes |
|---------|--------------|-------|
| Maintenance worker | `cortex serve` | Optional; capped outbox slices, no network listener |
| MCP stdio | `cortex mcp --agent <name>` | Lifetime bound to the host connection; opens SQLite directly |
| CLI operations | `cortex op <operation>` | Same kernel semantics and explicit caller scope |
| Status | `cortex status --json` | Opens the local brain; no token or running service required |

Local process/file access is the trust boundary. Async APIs receive the host-owned `&asupersync::Cx`; cancellation errors propagate. Existing owner IDs still scope domain operations, but remote authentication and HTTP headers no longer exist.

---

## V5 observation cycle (unreleased)

`runtime/observation.rs` owns explicit source grants and atomic occurrence/cursor/receipt intake. `inventory.rs` snapshots registered file candidates and resumes bounded base population without crawling unregistered paths. `cycle.rs` maintains scoped postings and reverse any-cue subscriptions, revalidates current source permissions on reads, and qualifies delivery against coverage, size limits and explicit context presence. Exact sources remain independently retrievable after derived-index replacement or retraction.

`host_capture.rs` provides a version-pinned Claude-shaped fixture subset and an origin firewall for live/history overlap. The optional plugin invocation sidecar selects capture-and-prepare without model-generated memory commands; missing native identities and unresolved origin fail closed. `associations.rs` adds opt-in lineage-deduplicated local routes and reversible named usefulness assessments. Learned navigation neither changes source authority nor proves causal utility.

These surfaces have native contract coverage, not installed-host or reader-quality certification. The CLI is `cortex capture`; see README and `crates/kernel/CONTRACT.md` for commands, bounds and no-claim boundaries. No host configuration is automatically installed, and no full V5 release-performance or power-loss claim follows from transactional fixtures.

## Two layers of truth

Source of truth is the written row, not a vector or a summary.

| Layer | Tables | Role |
|-------|--------|------|
| Facts | `memories`, `decisions` | Text, status, retention, TTL, validity windows, owner, visibility |
| Provenance | `traces`, `versions`, `head_state` | Every store is a trace + version. Rollback orphans later versions |
| Search cache | `memories_fts`, `decisions_fts` | FTS5, trigger-maintained |
| Identity graph | `entities`, `entity_aliases`, `entity_mentions` | Deterministic mentions (`auth service`, tickets, paths) |
| Clocks | `clock_anchors`, `clock_anchor_evidence`, `clock_links`, `clock_meta` | Derived handles. Rebuild with `cortex rebuild-anchors` |
| Coordination | `locks`, `sessions`, `tasks`, `messages`, `feed`, `focus` | Multi-agent conductor |
| Governance | `decision_conflicts`, `recall_feedback`, `agent_feedback`, `client_permissions` | Jaccard conflicts, use/harm signals |
| Inert | `embeddings` | Schema leftover. Not read |

Retention classes: **durable** (no TTL), **operational** (90d), **audit** (365d), **ephemeral** (14d).

---

## Store path

```text
client
  -> CortexRuntime::deposit / MCP cortex_commit
  -> redact secrets
  -> classify retention / TTL
  -> Jaccard conflict vs recent decisions
       AGREES / CONTRADICTS / REFINES / UNRELATED
  -> insert (or dispute / refine / merge)
  -> FTS trigger
  -> record trace + HEAD version
  -> ingest entities / aliases / mentions
  -> project clock anchors and co-occurrence links
```

Projection extracts inspectable handles from the text (and from explicit `paths` / `symbols` / `anchors` on the request):

- Hard-capable: path, `path::symbol`, ticket, error code, citation
- Named: entity, quoted phrase, acronym, flag, URL host, rare term
- Morphological variants of term anchors (`cache` also stores `caching`)
- Path ancestors (`src/auth.rs` also evidence-links `src`)

Origin is `explicit` if the client sent anchors, else `deterministic_extract`. Query expansion on read never writes new facts.

---

## Recall path: Clock-Quorum Recall

Every recall surface uses the same engine: `/recall`, `/recall/semantic`, `/recall/budget`, `/peek`, `/as-of`, MCP `cortex_recall` / `cortex_semantic_recall`. The name `semantic` is a compatibility surface. No query vector is used.

### 1. Parse a query frame

Terms, quoted phrases, path/symbol/session/goal context, temporal mode (`current` | `historical` | `explicit_as_of` | `any`), ACL owner, HEAD id.

### 2. Expand (never a hard admit)

`expand_query_frame` may add, all at low specificity:

| Handle | Closes | Is not |
|--------|--------|--------|
| Porter-like stem | `cache` ↔ `caching` | WordNet |
| Closed developer lexicon | `authenticate` ↔ `oauth`; `webhook` ↔ `callback` | a self-growing thesaurus |
| Sibling anchors on the same stored row | this-corpus co-occurrence | an LLM rewriter |
| Entity re-resolve | expanded terms can pick up mentions | a walk of the whole graph |

Caps: 16 extra terms, 6 siblings, 32 anchors. Common words still cannot admit a hit.

### 3. Collect six arms

| Arm | Clock | Seeds |
|-----|-------|-------|
| write | lexical / FTS | unigram `OR`; `write=2` only on quoted hit or unique stem/cluster hit |
| anchor | identity | `clock_anchors` specificity ≥ 2; ≥ 3 is hard |
| truth | entity | `entity_mentions`; entity hit is hard |
| task | work context | paths / symbols, or query path/symbol anchors ≥ 2 |
| history | use | `used_with` links / feedback |
| hop | neighborhood | FTS/anchor seeds; if empty, entity mentions, then ≤2 hops |

SQL gates run here: status, expiry, validity windows, orphaned versions, ACL, HEAD.

### 4. Admit, then rank

A row is admitted if:

1. a hard anchor matches, or
2. two independent clocks are nonzero, or
3. strong lexical write (`write ≥ 2`) holds.

Otherwise it is dropped. Empty is a valid answer.

Rank is a deterministic tuple: hard anchor → clock count → strength → specificity → fewer hops → FTS → use score → recency (`created_at`, not last access) → type → id.

`why` is machine-readable: clocks, anchors, links, filters. `validAt` is the requested as-of instant or `"current"` — not wall-clock now. As-of reports the row's stored `status` and validity windows.

Unconstrained paraphrase with no shared stem, cluster, alias, path, or co-occurring anchor remains empty on purpose.

---

## Boot path

`GET /boot` is an extractive compiler. No model summarizes.

Packed today:

1. **Identity** — durable constraints and platform facts
2. **Delta** — conflicts, tasks, focus, messages, locks, agents, recent decisions, feed, activity since last boot
3. **TRUTH** — top-N current facts ranked by retention × recency × relevance × activity, with `FACT!` / `FACT?` / `FACT~` sigils (legacy boot-capsule projection; the operations surface exposes the same records as Cards with an explicit epistemic status — see `docs/guides/user-guide.md`)

Then token-pack against the budget. Savings vs a raw dump are logged.

Named capsules SCARS / WAKE / SKILLS / BOARD are **not** separate compilers yet. Tasks, locks, and focus already exist as data and appear inside delta.

---

## Time, HEAD, ACL

Three independent gates, all SQL:

- **Validity windows** on the row (`valid_from` / `valid_until` / `expires_at`)
- **HEAD** via `versions` + `head_state` — rollback hides later stores
- **ACL** — team caller, `owner_id`, visibility

`/as-of` is not a costume that stamps every hit `historical`.

---

## Surrounding subsystems

| Subsystem | Job |
|-----------|-----|
| Conflict | Jaccard on store; CONTRADICTS opens a dispute |
| Focus | Checkpoint on start; stores append; end consolidates a summary row |
| Conductor | File locks, sessions, tasks, agent messages |
| Feed | Activity stream + ack |
| Feedback | Recall `used_with` / reject; agent outcome stats |
| Aging | Compress → archive; GC low score. Does not re-embed |
| Crystallize | Cluster similar rows by Jaccard |
| Compaction | Storage governor, archived blobs |
| Budgets | `~/.cortex/budgets.toml` per store / recall / boot / MCP |

---

## Crate map

```text
crates/logic/src/clockwork/
  anchors.rs        kinds, extraction, morph variants on persist
  query.rs          QueryFrame, temporal mode
  morph.rs          stem / variants / hay_has_lexical
  bridge.rs         expand_query_frame
  evidence.rs       ClockEvidence, ClockWhy
  quorum.rs         admit + RankKey
  links.rs          project, hops, used_with, DDL

crates/logic/src/graph/     entities, closed synonym clusters
crates/logic/src/traces/    traces, versions, HEAD
crates/logic/src/conflict/  Jaccard classes

crates/kernel/src/handlers/recall/engine_clockwork.rs   six arms + gates
crates/kernel/src/handlers/store/                       write + project
crates/kernel/src/compiler/                             boot pack
crates/kernel/src/db/                                   schema, FTS, migrations
crates/kernel/src/runtime/                              CortexRuntime (embed entry)
```

Admission math lives in `cortex-logic`. Candidate SQL lives in the kernel; the daemon only adds transport. Rebuild projections without changing admit.

---

## Configuration

| Source | Fields |
|--------|--------|
| CLI | `--home`, `--db`, `--port`, `--bind` |
| Environment | `CORTEX_HOME`, `CORTEX_DB`, `CORTEX_PORT`, `CORTEX_BIND` |
| Defaults | `~/.cortex`, `cortex.db`, `cortex.token`, port `7437` |
| Budgets | `~/.cortex/budgets.toml` |

Operator-critical environment variables (meanings derived from the definition sites):

| Variable | Meaning |
|----------|---------|
| `CORTEX_HOME` | Root of all state: db, token, pid, lock, tls (`auth/keys.rs:9`) |
| `CORTEX_DB` | SQLite database path override (`auth/paths.rs:33`) |
| `CORTEX_PORT` | Daemon listen port; default `7437` (`auth/paths.rs:36`) |
| `CORTEX_BIND` | Daemon bind address; default localhost (`auth/paths.rs:38`) |
| `CORTEX_TLS_CERT` | TLS certificate path; required for team mode (`tls.rs:10`) |
| `CORTEX_TLS_KEY` | TLS private-key path; required for team mode (`tls.rs:13`) |
| `CORTEX_ALLOW_INSECURE_REMOTE` | `=1` explicitly allows serving plain HTTP on non-local binds; temporary override (`server/runtime.rs:79`) |
| `CORTEX_API_KEY` | Client-side API key for remote daemon targets (`cli/common.rs:92`) |
| `CORTEX_API_BASE` / `CORTEX_BASE_URL` | Client-side base URL of a remote daemon (`cli/common.rs:91`) |
| `CORTEX_RATE_LIMIT_REQUESTS_PER_MIN` | Per-minute request budget; over-budget requests get 429 (`crates/logic/src/rate_limit/mod.rs:97`) |
| `CORTEX_RECALL_{FAST,BALANCED,DEEP}_BUDGET` | Per-policy recall token budgets (`handlers/recall/engine.rs:438-440`) |
| `CORTEX_IDLE_SHUTDOWN_SECS` | Idle-auto-shutdown threshold (`cli/daemon/startup.rs:38`) |

The complete `CORTEX_*` variable surface is enumerated in the source (`grep CORTEX_ crates/`); undocumented variables are internal tuning knobs with no stability guarantee (declared no-claim boundary — they may change or disappear without notice).

Removed from the runtime: `CORTEX_EMBEDDING_MODEL`, `CORTEX_EMBED_SESSION_POOL_SIZE`, `CORTEX_RERANK_*`. Historical changelog and benchmark text may still mention them.

The daemon crate does not depend on `ort`, `tokenizers`, `sqlite-vec`, or `cortex-models`. That crate is gone.

---

## Tests

| Kind | Location |
|------|----------|
| Rust contracts | `tests/contracts/` — including `clock_quorum.rs` |
| Desktop | `desktop/cortex-control-center` Vitest |
| First-run smoke | `tests/scripts/first-run-smoke.sh` |

Production crates have no inline tests. CQR, store, conflict, temporal, and history contracts are the recall bar.

---

## Safety

- Lock cancellation propagates through the caller capability; committed SQLite effects are not undone by later cancellation.
- Secret redaction runs before anchor extraction.
- ACL, HEAD, validity, and expiry are SQL gates during candidate generation.
- Empty evidence is returned rather than a neighbor guess.

See [Info/security-rules.md](Info/security-rules.md) for the threat model.

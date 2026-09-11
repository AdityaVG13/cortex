# Architecture

Cortex is a private, local-first memory library for AI tools. `cortex-kernel` owns SQLite semantics; the `cortex` executable provides in-process CLI, hooks and MCP stdio. Asupersync owns runtime capabilities, locks, channels and timers. No HTTP listener or proxy is built. The separate Control Center and HTTP SDKs are legacy clients for earlier tagged releases, not clients of this runtime.

Clock-Quorum Recall (CQR) is the explicit semantic engine (`cortex op`, `cortex boot`, MCP commit/query). Host hooks use the observation cycle: exact attributed capture and any-cue preparation. Observation does not promote captured prose into CQR facts. The daemon does not download, load, or run language, embedding, or reranking models. Older databases may still contain inert `embeddings` rows; they are not read. `~/.cortex/models` is neither required nor created.

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

## Automatic observation cycle

Host live events enter through one command: `cortex hook <kind>`. The plugin script only resolves the binary and relays stdin/stdout. `CORTEX_CAPTURE` is read inside that command. If the sidecar selects this event, the kernel runs `host_capture` (exact intake + prepare). If the sidecar omits this event, or is unset, the command stays silent. CQR orient / tool-result deposit / checkpoint stay on `cortex hook-boot`, `process()`, `cortex op`, and MCP. There is no `hook-event` command and no second env name for the sidecar.

`cortex capture` is the operator surface for the same observation store: register/enable/put/tail/file/get, inventory/bootstrap/reconcile, subscribe/prepare/query/require, and flagged `host-register` / `host-put` / `host-tail` / `host-cycle`. `host-cycle` is the explicit-flag form used by operators and contracts; the plugin does not spawn it.

`runtime/observation.rs` owns source grants and atomic occurrence/cursor/receipt intake. `inventory.rs` snapshots registered file candidates, resumes bounded bootstrap, and reconciles a sealed revision against later grants without adding those grants to the old denominator. `cycle.rs` maintains scoped postings and reverse any-cue subscriptions, applies exclude cues, walks required-child closure, revalidates current source permissions on reads, and qualifies delivery against coverage, size limits and explicit context presence.

`host_capture.rs` accepts the fixture string-tool subset plus structured Bash, Read, Edit, Write and MultiEdit reports. PreToolUse prepares from tool input and does not store the request. PreCompact records a silent checkpoint. Origin policy is operator-owned, never parsed from hook stdin as authority. `associations.rs` is opt-in local routing with reversible named usefulness assessments. `assembly.rs` stores exact revision membership plus an attributed learning ledger; cue routes stay off until rebuild and never change epistemic status. After rebuild, `orient` / `query` compile evidence-closed assembly bundles next to CQR Cards, and hooks inject that brief. Learned navigation neither changes source authority nor proves causal utility.

These surfaces have native contract coverage, not installed-host or reader-quality certification. See README and `crates/kernel/CONTRACT.md` for bounds and no-claim language.

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

Origin is `explicit` if the client sent anchors or project `paths` / `thread`, else `deterministic_extract`. Query expansion on read never writes new facts.

A query or boot that names project paths treats those explicit roots as a filter: a fact stored under `/Users/x/repoa` is ineligible for `/Users/x/repob`. A fact with no stored root stays eligible (ignorance never demotes). The same paths are a hard task-clock, so a cwd-only orient can admit the repo's facts without a ticket in the sentence.

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

Three Rust crates. Brain types live in `cortex-kernel` or `cortex-logic`. Import those crates directly. `cortex-daemon` is the process edge only (CLI, MCP stdio, plugin spawn, setup).

```text
cortex-logic          pure types, no SQLite
  adapter             EventKind, decide(), hook protocol
  clockwork/          CQR admit math: anchors, query, morph, quorum, links
  capture             deterministic tool-result facts
  protocol, traces, conflict, graph, lens, budgets, presence, recipe, eval

cortex-kernel         the brain (SQLite + engines)
  runtime/open        CortexRuntime::open / deposit / lens / boot
  runtime/observation exact source grants, observe, tail, get
  runtime/cycle       needs, prepare, require, exclude
  runtime/inventory   file inventory, bootstrap, reconcile
  runtime/host_capture host shapes, origin firewall, hook sidecar resolve
  runtime/associations opt-in learned routes
  runtime/assembly    exact membership, learning ledger, opt-in cue routes
  runtime/deposit     CQR fact write
  hook_event          cortex hook: observation only; CQR stays on process() / hook-boot
  handlers/           store, recall (CQR SQL), operations, redaction
  compiler, db, reflex, indexer, crystallize, compaction, store_spi

cortex-daemon         process edge only
  main + cli/         cortex binary: capture, hook, boot, op, mcp, serve
  mcp_native          MCP stdio
  hook_boot           SessionStart orient
  setup, prompt_inject
  handlers/mcp        MCP dispatch

cortex-tests          public contracts in tests/contracts/
plugins/cortex-plugin hook-event.cjs -> cortex hook <kind>
                      hook-boot.cjs  -> cortex hook-boot
```

Host vs operator:

| Intent | Command |
|--------|---------|
| Live host event | `cortex hook <kind>` (plugin always) |
| SessionStart CQR boot | `cortex hook-boot` |
| Observation admin | `cortex capture …` |
| Semantic CQR | `cortex op`, `cortex boot`, MCP |

Admission math lives in `cortex-logic`. Candidate SQL lives in the kernel. Rebuild projections without changing admit.

---

## Configuration

| Source | Fields |
|--------|--------|
| CLI | `--home`, `--db` |
| Environment | `CORTEX_HOME`, `CORTEX_DB`, `CORTEX_CAPTURE` (live hook sidecar) |
| Defaults | `~/.cortex`, `cortex.db` |
| Budgets | `~/.cortex/budgets.toml` |

Operator-critical environment variables (meanings derived from the definition sites):

| Variable | Meaning |
|----------|---------|
| `CORTEX_HOME` | Root of local state: db, token, pid, lock |
| `CORTEX_DB` | SQLite database path override |
| `CORTEX_CAPTURE` | JSON sidecar for `cortex hook`; grant, host_version, origins, and opt-in native_* flags |
| `CORTEX_PLUGIN_AGENT` | Agent id the plugin passes to `--agent` |

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

# Architecture

Cortex is a private, local-first memory system for AI tools. A Rust daemon owns one SQLite brain and exposes it over HTTP and MCP. The Control Center is a Tauri desktop app that supervises that daemon.

Clock-Quorum Recall (CQR) is the only production retrieval engine. The daemon does not download, load, or run language, embedding, or reranking models. Older databases may still contain inert `embeddings` rows; they are not read. `~/.cortex/models` is neither required nor created.

Current version: **0.6.0**.

---

## Products in this tree

| Product | Path | Role |
|---------|------|------|
| Daemon | `crates/daemon` (`cortex-daemon`, binary `cortex`) | HTTP API, MCP stdio proxy, SQLite, CQR collection, boot compiler |
| Logic | `crates/logic` (`cortex-logic`) | Deterministic types: clocks, graph, traces, conflict, budgets |
| Tests | `tests/` (`cortex-tests`) | Public contracts. Production crates have no inline tests |
| Control Center | `desktop/cortex-control-center` | Operator UI, daemon supervisor, budgets, brain view |
| Plugin | `plugins/cortex-plugin` | Claude Code attach-only MCP bridge |
| SDKs | `sdks/python`, `sdks/typescript` | Thin HTTP clients |

---

## Runtime surfaces

| Surface | How to start | Notes |
|---------|--------------|-------|
| HTTP daemon | `cortex serve` | Default `127.0.0.1:7437` |
| MCP stdio | `cortex mcp --agent <name>` | Proxies onto the running daemon; does not spawn a second one from plugin paths |
| Control Center | Desktop installer / `npm run desktop:dev` | Owns daemon lifecycle in app-managed mode |
| Status | `cortex status --json` | Readiness without starting another daemon |

Protected HTTP requires `Authorization: Bearer …` and `X-Cortex-Request` (SSRF guard). Solo mode uses `~/.cortex/cortex.token`. Team mode uses Argon2id-hashed `ctx_` keys.

---

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
  -> POST /store
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
3. **TRUTH** — top-N current facts ranked by retention × recency × relevance × activity, with `FACT!` / `FACT?` / `FACT~` sigils

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

crates/daemon/src/handlers/recall/engine_clockwork.rs   six arms + gates
crates/daemon/src/handlers/store/                       write + project
crates/daemon/src/compiler/                             boot pack
crates/daemon/src/db/                                   schema, FTS, migrations
```

Admission math lives in `cortex-logic`. Candidate SQL lives in the daemon. Rebuild projections without changing admit.

---

## Configuration

| Source | Fields |
|--------|--------|
| CLI | `--home`, `--db`, `--port`, `--bind` |
| Environment | `CORTEX_HOME`, `CORTEX_DB`, `CORTEX_PORT`, `CORTEX_BIND` |
| Defaults | `~/.cortex`, `cortex.db`, `cortex.token`, port `7437` |
| Budgets | `~/.cortex/budgets.toml` |

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

- Handler panics become HTTP 500 via `CatchPanicLayer`.
- Secret redaction runs before anchor extraction.
- ACL, HEAD, validity, and expiry are SQL gates during candidate generation.
- Empty evidence is returned rather than a neighbor guess.

See [Info/security-rules.md](Info/security-rules.md) for the threat model.

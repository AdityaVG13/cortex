<p align="center"><a href="../README.md">← Back to README</a></p>

# Research

Papers and systems that shaped Cortex, and where Cortex went its own way.

**Last reviewed:** 2026-08-30

Academic papers are research inputs, not claims of a line-for-line implementation. Open-source references use “inspired by” for design influence, not code reuse.

---

## How to read this page

| Label | Meaning |
|-------|---------|
| **Shipped** | Materially shapes current Cortex behavior |
| **Historical** | Shaped a previous release; no longer the production path |
| **Open** | Interesting, not committed |
| **Rejected** | Looked at and declined for the hot path |

Cortex is a **store of traces with admission**, not a chatbot memory extractor. Retrieval is Clock-Quorum Recall (CQR): inspectable handles, independent clocks, and an empty result when evidence is missing.

---

## Current engine

| Area | References | Cortex |
|------|------------|--------|
| **Admission** | Quorum / multi-evidence IR | Hard anchor, two clocks, or strong lexical write. Otherwise empty. |
| **Morphology** | Porter 1980, Krovetz 1993 | Closed technical stem. `cache` ↔ `caching`. No WordNet. |
| **Lexicon** | ESA (Gabrilovich & Markovitch, 2007) | Short owned developer clusters, not Wikipedia vectors. |
| **Co-occurrence** | KAR ([2410.13765](https://arxiv.org/abs/2410.13765)) | Sibling clock anchors on the same stored row. No LLM rewriter. |
| **Neighborhood** | ARK ([2601.13969](https://arxiv.org/abs/2601.13969)) | FTS/anchors first; entity mentions then ≤2 hops if seeds are empty. No LLM tool-chooser. |
| **Hops** | Haveliwala PPR, PRA | Entity-seeded bounded hops. Not learned path weights. |
| **Daemon memory** | MemGPT / Letta ([2310.08560](https://arxiv.org/abs/2310.08560)), MaaS ([2506.22815](https://arxiv.org/abs/2506.22815)) | Memory as a local process with tools. Cortex is not the agent runtime. |
| **Decay** | MemoryBank ([2305.10250](https://arxiv.org/abs/2305.10250)) | Use strengthens; aging compresses. No re-embed. |
| **Episodic traces** | Episodic Memory ([2502.06975](https://arxiv.org/abs/2502.06975)) | Raw traces stay; validity windows and HEAD are first-class. |
| **Dedup** | agentmemory (Jaccard) | Store-path Jaccard conflict classes. |

---

## Shipped into CQR

### Porter (1980) / Krovetz (1993)

**Key idea:** Inflection is identity, not similarity. Stemming is a closed suffix table.

**Cortex:** `morph_stem` / `morph_variants` / `hay_has_lexical`. Used on persist and on query. No dictionary download.

### Explicit Semantic Analysis (2007)

**Key idea:** Meaning as weights over a **named concept inventory**, not a latent neighbor.

**Cortex:** `SYNONYM_CLUSTERS` in `crates/logic/src/graph` — auth, cache, payments, webhook, and similar developer clusters. A pair is added when a real domain miss shows up. Not WordNet.

### KAR — Knowledge-Aware Query Expansion ([2410.13765](https://arxiv.org/abs/2410.13765))

**Key idea:** Expand a query using document/graph relations, not only bag-of-words. The paper uses an LLM rewriter plus a knowledge graph.

**Cortex:** The graph half only. If a query term already sits on a stored row, pull a few other strong anchors from that same row. Specificity clamped to 1–2. Never a hard admit by itself.

### ARK — Adaptive Retriever of Knowledge ([2601.13969](https://arxiv.org/abs/2601.13969))

**Key idea:** Alternate global lexical search with one-hop neighborhood. The paper lets an LLM choose the tool.

**Cortex:** FTS + anchors first. If those seeds are empty, seed hops from `entity_mentions`. No model on the chooser.

### MemGPT / Letta ([2310.08560](https://arxiv.org/abs/2310.08560))

**Key idea:** Core vs archival memory; the agent pages context in through tools.

**Cortex:** Boot is the always-on pack; `/recall` is archival search. Cortex does not become the agent loop.

### Memory as a Service ([2506.22815](https://arxiv.org/abs/2506.22815))

**Key idea:** Memory as governed infrastructure, not an implementation detail inside one assistant.

**Cortex:** One daemon, HTTP + MCP, many clients.

### MemoryBank ([2305.10250](https://arxiv.org/abs/2305.10250))

**Key idea:** Recalled memories strengthen; unused ones fade.

**Cortex:** Decay, aging, compaction. Feedback `used_with` exists; it does not yet strip lexical admission on harm.

---

## Historical (v0.5 embedding era)

These shaped the previous retrieval stack. They are **not** the production path.

| Paper | What Cortex took then | Status now |
|-------|----------------------|------------|
| Reciprocal Rank Fusion (2009) | Fuse keyword + dense ranks | Unused on the hot path |
| Rethinking Hybrid Retrieval ([2506.00049](https://arxiv.org/abs/2506.00049)) | Compact local embeddings | Model crate deleted |
| ByteRover ([2604.01599](https://arxiv.org/abs/2604.01599)) | Cheap path first | CQR arms replace staged neural retrieval |
| HyDE ([2212.10496](https://arxiv.org/abs/2212.10496)) | Hypothetical-document expansion | Rejected (needs a generator on query) |
| DS@GT Fusion ([2601.15518](https://arxiv.org/abs/2601.15518)) | Sparse/dense + rerank | Reranker removed |

`CHANGELOG.md` still records that v0.5.0 / v0.6.0 **shipped** MiniLM/BGE and a shadow reranker. That is history. The live daemon does not load them.

---

## Open — interesting, not committed

These are the papers that should change Cortex’s *shape* if we take them, not the retriever.

| Paper | Steal | Do not steal |
|-------|-------|--------------|
| Recuris ([2608.24876](https://arxiv.org/abs/2608.24876)) | Verified working memory; retrieve at execution events; component-scoped patches | LLM Meta-Agent on the hot path |
| A-MEM ([2502.12110](https://arxiv.org/abs/2502.12110)) | Atomic notes + explicit links | LLM-written links as source of truth |
| HippoRAG / HippoRAG 2 ([2405.14831](https://arxiv.org/abs/2405.14831)) | PPR over a corpus graph | LLM NER as the only index |
| MemoryOS ([2506.06326](https://arxiv.org/abs/2506.06326)) | Short / mid / long tiers | Hidden promotion |
| LightMem / RAPTOR ([2401.18059](https://arxiv.org/abs/2401.18059)) | Offline consolidate; summary tree as cache | Summarizer on `/store` or `/recall` |
| A-MAC ([2603.04549](https://arxiv.org/abs/2603.04549)) | Scored write admission | Opaque learned policy |
| Mem0 ([2504.19413](https://arxiv.org/abs/2504.19413)) | Entity linking; ADD-only (don’t clobber) | LLM extractor + embeddings as truth |
| MemRL / Mem-alpha | Outcome-gated ranking | RL in the daemon |

Product-level comparison (Mem0, Supermemory, Letta, Eidetic Engine, OptMem, Recuris) lives in [memory-landscape.md](memory-landscape.md).

---

## Rejected for the hot path

- **Embeddings / ONNX / rerank as recall.** They mix “related in English” with “this is the fact we stored,” cannot abstain cleanly, and force a second truth beside the traces.
- **LLM query rewrite.** KAR/ARK use one. Cortex keeps expansion inspectable.
- **WordNet-scale synonym growth.** Closed lexicon only; add a pair when a real miss appears.
- **Closing unconstrained paraphrase.** `we mint JWTs in the gateway` vs `how do cats authenticate` stays empty.
- **Hosted-service assumptions.** Local-first.
- **Cortex as the agent runtime.** That is Letta’s job. Cortex is the shared brain.

---

## Keeping this page honest

- When a research-backed feature ships, mark it **Shipped** in the same change.
- When a paper is no longer the production path, move it to **Historical** rather than deleting it.
- Use “inspired by” for repos, talks, and blogs.
- Rejected ideas stay in “Rejected” with a reason.

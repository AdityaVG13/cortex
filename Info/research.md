<p align="center"><a href="../README.md">← Back to README</a></p>

# Research

> Which papers influenced shipped behavior, which ideas are mapped to the roadmap, and where Cortex intentionally went its own way.

**Last reviewed:** 2026-04-10

---

## How to read this page

| Label | Meaning |
|-------|---------|
| **Implemented** | The idea materially shaped shipped Cortex behavior |
| **Planned** | Tied to a named roadmap phase |
| **Deferred** | Important, but not committed to the current build plan |

Academic papers are research inputs, not claims of direct implementation. Open-source references use "Inspired by" wording — that documents design influence, not code reuse.

---

## Research → Product map

| Area | Key references | Cortex status |
|------|---------------|---------------|
| **Retrieval** | ByteRover, RRF, Rethinking Hybrid Retrieval, SmartSearch, DS@GT Fusion, HyDE, agentmemory | Phase 1 shipped hybrid retrieval + fusion; reranking and short-query expansion are future work |
| **Structure / dedup** | Episodic Memory, MemoryBank, Memori, A-Mem | Raw traces + durable knowledge split; semantic triples and deeper dedup planned |
| **Lifecycle** | A-MAC, MemoryOS, FluxMem, RGMem, ProMem, Mem-alpha, MemFactory | Admission control, tiers, probabilistic crystallization planned; learned policies deferred |
| **Multi-agent** | MaaS, MemGPT, Collaborative Memory, MIRIX, MAGMA | Shared daemon shipped; provenance, routing, memory-view separation planned |
| **Compression** | LLMLingua-2, Active Context Compression, LazyLLM, RAPTOR, MemWalker | Boot compression shipped; active compression and tree retrieval future-facing |

---

## What Cortex changes when it adapts research

- **Local-first.** Papers assuming hosted services get translated into SQLite, ONNX, and local process boundaries.
- **Coding workflows.** Retrieval, decay, and provenance are optimized for engineering, not general consumer chat.
- **MCP + HTTP.** Memory is exposed through standard interfaces instead of being embedded in one assistant.
- **Human in the loop.** Research ideas assuming full autonomy are narrowed into auditable, operator-visible systems.
- **Reliability over completeness.** Deferred papers are strategically attractive but not yet worth the complexity.

---

## Shipped

### ByteRover: Agent-Native Memory Through LLM-Curated Hierarchical Context (2026)

[arxiv.org/abs/2604.01599](https://arxiv.org/abs/2604.01599)

**Key idea:** Retrieval improves when cheap paths handle easy queries first; expensive logic is reserved for the hard ones. Memory as staged retrieval + maturity lifecycle.

**Cortex adaptation:** Phase 1 adopted progressive retrieval and field-aware ranking. The maturity-tier idea moved into later lifecycle planning. &nbsp; `Implemented — Phase 1, v0.5.0`

### Reciprocal Rank Fusion (Cormack, Clarke, Buettcher, 2009)

[dl.acm.org/doi/10.1145/1571941.1572114](https://dl.acm.org/doi/10.1145/1571941.1572114)

**Key idea:** Simple reciprocal-rank fusion often beats any single retriever. Brutally pragmatic — better ranking with almost no theory-heavy machinery.

**Cortex adaptation:** Used inside the local hybrid stack, combining keyword and semantic signals without a separate reranking service. &nbsp; `Implemented — Phase 1, v0.5.0`

### Rethinking Hybrid Retrieval (2025)

[arxiv.org/abs/2506.00049](https://arxiv.org/abs/2506.00049)

**Key idea:** Compact embeddings plus good architecture beats "bigger model, bigger index." Validates strong retrieval from MiniLM-class local models.

**Cortex adaptation:** Doubled down on MiniLM-centered local retrieval. Heavier reranking deferred until justified inside the local-first budget. &nbsp; `Implemented — Phase 1`

### Episodic Memory is the Missing Piece for Long-Term LLM Agents (2025)

[arxiv.org/abs/2502.06975](https://arxiv.org/abs/2502.06975)

**Key idea:** Long-term agents need episodic traces with temporal context, not just decontextualized facts.

**Cortex adaptation:** Raw session traces kept separate from durable summaries. Conflict and supersession handling layered on top. &nbsp; `Implemented — v0.4.1`

### MemoryBank: Long-Term Memory for LLMs (2024)

[arxiv.org/abs/2305.10250](https://arxiv.org/abs/2305.10250)

**Key idea:** Useful memories strengthen when recalled; stale ones fade. Memory quality as lifecycle, not capacity.

**Cortex adaptation:** Justified decay, reinforcement, and cleanup pressure. Grounded in auditable local store. &nbsp; `Implemented — v0.4.1`

### Memory as a Service / MaaS (2025)

[arxiv.org/abs/2506.22815](https://arxiv.org/abs/2506.22815)

**Key idea:** Memory as governed infrastructure instead of internal implementation detail.

**Cortex adaptation:** Daemon boundary central from the start. MCP + HTTP so different tools share one brain. &nbsp; `Implemented — v0.4.1`

### MemGPT / Letta: LLMs as Operating Systems (2023)

[arxiv.org/abs/2310.08560](https://arxiv.org/abs/2310.08560)

**Key idea:** Long-term memory via explicit operations, not prompt stuffing.

**Cortex adaptation:** Memory through tools and APIs. Operating-system idea translated into local daemon with observable behavior. &nbsp; `Implemented — v0.4.1`

### agentmemory (GitHub)

[github.com/rohitg00/agentmemory](https://github.com/rohitg00/agentmemory)

**Inspired by:** Triple-stream retrieval, quality scoring, Jaccard dedup. Apache-2.0.

**Cortex adaptation:** Retrieval stack composition and dedup mindset. Built local-first, daemon-backed, conflict-aware with MCP/HTTP surfaces. &nbsp; `Implemented — Phase 1 influence`

---

## Planned

### Memori (2026) &nbsp; `→ Phase 2`

[arxiv.org/abs/2603.19935](https://arxiv.org/abs/2603.19935) — Semantic triples + strong dedup. Dramatically less context while preserving answer quality. Cortex Phase 2: stronger semantic structure and dedup for coding memory.

### A-MAC: Adaptive Memory Admission Control (2026) &nbsp; `→ Phase 4A`

[arxiv.org/abs/2603.04549](https://arxiv.org/abs/2603.04549) — Scored admission (novelty, utility, confidence, recency). Cortex: admission logic on `cortex_store`, kept explainable and operator-visible.

### MemoryOS (2025) &nbsp; `→ Phase 4B`

[arxiv.org/abs/2506.06326](https://arxiv.org/abs/2506.06326) — Short/mid/long-term as explicit tiers with promotion rules. Cortex: maturity tiers grounded in local daemon observability.

### FluxMem: Choosing How to Remember (2026) &nbsp; `→ Phase 4C`

[arxiv.org/abs/2602.14038](https://arxiv.org/abs/2602.14038) — Probabilistic fusion gates over brittle thresholds. Cortex: replace crystallization thresholds with BMM-style gates after current system is benchmarked.

### Collaborative Memory (2025) &nbsp; `→ Phase 4D`

[arxiv.org/abs/2505.18279](https://arxiv.org/abs/2505.18279) — Provenance, ownership, dynamic access control for shared memory. Cortex: stronger provenance + shared/private boundaries in a local-first product.

---

<details>
<summary><b>Deferred research</b> — 16 papers shaping long-term direction</summary>

### Memory in the Age of AI Agents (2025)
[arxiv.org/abs/2512.13564](https://arxiv.org/abs/2512.13564) — Taxonomy of agent memory forms, functions, lifecycle stages. Used as framing reference for future metadata, routing, and lifecycle work.

### A-Mem: Agentic Memory for LLM Agents (2025)
[arxiv.org/abs/2502.12110](https://arxiv.org/abs/2502.12110) — Structured note-like memories with auto link generation. Future co-occurrence and structure updates, once link synthesis is reliable for engineering data.

### RGMem: Renormalization Group-inspired Memory Evolution (2024)
[arxiv.org/abs/2510.16392](https://arxiv.org/abs/2510.16392) — Multi-scale consolidation with dominant patterns + correction terms. Blueprint for future hierarchical crystal system.

### MemRL: Self-Evolving Agents via Runtime RL (2026)
[arxiv.org/abs/2601.03192](https://arxiv.org/abs/2601.03192) — Utility ranking learned from downstream outcomes. Future feedback-shaped ranking for coding tasks.

### Mem0: Production-Ready AI Agents with Scalable Long-Term Memory (2025)
[arxiv.org/abs/2504.19413](https://arxiv.org/abs/2504.19413) — Proves the category is product-real. Reinforced treating latency and operations as first-class concerns.

### HippoRAG: Neurobiologically Inspired Long-Term Memory (2024)
[arxiv.org/abs/2405.14831](https://arxiv.org/abs/2405.14831) — Associative graph traversal for multi-hop recovery. Backlog for future multi-hop recall across decisions, files, sessions.

### MAGMA: Multi-Graph Agentic Memory Architecture (2026)
[arxiv.org/abs/2601.03236](https://arxiv.org/abs/2601.03236) — Multiple memory projections (semantic, temporal, causal, entity). Future architecture reference for separate memory views.

### Mem-alpha: Learning Memory Construction via RL (2025)
[arxiv.org/abs/2509.25911](https://arxiv.org/abs/2509.25911) — Trainable memory policy. Deferred until explainable heuristics are mature enough to benchmark against.

### ProMem: Beyond Static Summarization (2026)
[arxiv.org/abs/2601.04463](https://arxiv.org/abs/2601.04463) — Self-questioning and iterative refinement. Future revisitable crystals, pending better instrumentation.

### MemSearcher: Training LLMs to Search and Manage Memory via RL (2025)
[arxiv.org/abs/2511.02805](https://arxiv.org/abs/2511.02805) — Better memory systems beat bigger models. Strategic support for smarter local memory over larger context windows.

### RAPTOR: Recursive Abstractive Processing for Tree-Organized Retrieval (2024)
[arxiv.org/abs/2401.18059](https://arxiv.org/abs/2401.18059) — Bottom-up summary trees for multi-level retrieval. Reference for future hierarchical crystals.

### LLMLingua-2: Prompt Compression (2024)
[arxiv.org/abs/2310.05736](https://arxiv.org/abs/2310.05736) — Compression as signal design. Informs future signal-aware context shaping.

### MIRIX: Multi-Agent Memory System (2025)
[arxiv.org/abs/2507.07957](https://arxiv.org/abs/2507.07957) — Recall as a routing problem. Future active-retrieval router for different memory types.

### MemWalker: Walking Down the Memory Maze (2023)
[arxiv.org/abs/2310.05029](https://arxiv.org/abs/2310.05029) — Navigable summary tree for long-context search. Supporting evidence for future broad-to-specific navigation.

### LazyLLM: Dynamic Token Pruning (2024)
[arxiv.org/abs/2407.14057](https://arxiv.org/abs/2407.14057) — Lazy context loading as relevance clarifies. Future streaming recall and progressive context injection.

### Additional deferred references

- **Multi-Layered Memory Architectures** (2026) — [arxiv.org/abs/2603.29194](https://arxiv.org/abs/2603.29194) — Bounded context growth and regularized retention.
- **SmartSearch** (2026) — [arxiv.org/abs/2603.15599](https://arxiv.org/abs/2603.15599) — Ranking quality over architecture theater.
- **MemFactory** (2026) — [arxiv.org/html/2603.29493](https://arxiv.org/html/2603.29493) — RL across the whole memory lifecycle.
- **DS@GT Fusion** (2026) — [arxiv.org/abs/2601.15518](https://arxiv.org/abs/2601.15518) — Sparse/dense retrieval strengthened by reranking.
- **Active Context Compression** (2026) — [arxiv.org/abs/2601.07190](https://arxiv.org/abs/2601.07190) — Task-aware live compression.
- **HyDE** (2022) — [arxiv.org/abs/2212.10496](https://arxiv.org/abs/2212.10496) — Hypothetical document generation for short queries.

</details>

---

## What Cortex hasn't shipped yet

- **Learned policies** — Admission, retention, and crystallization are heuristic. Learned control is future work.
- **Full reranking** — Phase 1 improved retrieval substantially, but ranking quality is still an active frontier.
- **Query routing** — Current recall is stronger, but less differentiated than the best research systems.
- **Hierarchical summaries** — Multi-graph recall and summary trees remain backlog work.
- **Team memory governance** — Provenance-aware team controls are planned, not complete.

## What Cortex rejected (for now)

- **Hosted-service assumptions** — Stripped out. Cortex is local-first.
- **Full autonomy** — Delayed for operator visibility. Show why memory was admitted, fused, or disputed.
- **Copying reference implementations** — Inspiration is documented. Architecture is built around local constraints.
- **Benchmark overfitting** — Some ideas look strong in evaluation but don't justify operational complexity.

---

## Keeping this page honest

- When a research-backed feature ships → update entry to `Implemented`.
- When a new paper influences design → add link + interesting idea + adaptation note in the same change.
- Use "Inspired by" for open-source repos, talks, blogs, non-paper references.
- Intentionally rejected ideas go in "What Cortex rejected" with reasoning.

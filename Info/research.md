# Cortex Research Map

This page is the public digest of the internal v0.5.0 memory-intelligence survey plus the extra retrieval references that directly shaped shipped Cortex features.

Each entry answers four questions:
- what the paper showed
- what Cortex took from it
- where that idea maps into the product
- whether that work is already shipped, planned, or still deferred

## Implemented in Cortex

### ByteRover: Agent-Native Memory Through LLM-Curated Hierarchical Context (2026)
**Link:** https://arxiv.org/abs/2604.01599
**Key insight:** Progressive multi-stage retrieval and field-aware ranking can answer most queries quickly without heavyweight external infrastructure.
**What we took:** Cortex Phase 1 adopted tiered retrieval, stronger keyword weighting, and a cheaper-first retrieval pipeline; ByteRover's AKL maturity model also informs the later Phase 4B roadmap.
**Status:** Implemented (v0.5.0)

### Reciprocal Rank Fusion (Cormack, Clarke, Buettcher) (2009)
**Link:** https://dl.acm.org/doi/10.1145/1571941.1572114
**Key insight:** A simple `1 / (k + rank)` fusion rule consistently beats relying on any single retriever.
**What we took:** Cortex Phase 1 uses RRF to combine multiple retrieval signals instead of trusting one ranking pass.
**Status:** Implemented (v0.5.0)

### Rethinking Hybrid Retrieval (2025)
**Link:** https://arxiv.org/abs/2506.00049
**Key insight:** Well-tuned hybrid retrieval with compact embedding models and reranking beats brute-force bigger-model retrieval.
**What we took:** Cortex Phase 1 doubled down on hybrid retrieval around local MiniLM-class embeddings; the paper's reranking ideas remain a deferred follow-on rather than a shipped dependency.
**Status:** Implemented (v0.5.0)

### agentmemory (GitHub project)
**Link:** https://github.com/rohitg00/agentmemory
**Key insight:** Triple-stream retrieval fusion plus Jaccard-style memory hygiene produces better long-horizon agent recall than any one search mode alone.
**What we took:** Cortex Phase 1 shipped fused retrieval inspired by this work, and its dedup thresholds continue to inform planned Phase 2 semantic-cleanup work.
**Status:** Implemented (v0.5.0)

### Episodic Memory is the Missing Piece for Long-Term LLM Agents (2025)
**Link:** https://arxiv.org/abs/2502.06975
**Key insight:** Long-term agents need episodic traces, temporal context, interference management, and consolidation into more durable knowledge.
**What we took:** This reinforced Cortex's split between raw session traces and crystallized knowledge, plus the need for conflict, supersession, and consolidation mechanics.
**Status:** Implemented (v0.4.1)

### MemoryBank: Enhancing Large Language Models with Long-Term Memory (2024)
**Link:** https://arxiv.org/abs/2305.10250
**Key insight:** Long-term memory works better when recall strength changes over time instead of treating every stored fact as equally fresh forever.
**What we took:** Cortex's decay-based scoring and reinforcement-through-recall direction are directly aligned with this line of work.
**Status:** Implemented (v0.4.1)

### Memory as a Service (MaaS) (2025)
**Link:** https://arxiv.org/abs/2506.22815
**Key insight:** Memory becomes more useful when it is decoupled from one agent and exposed as a governed service that multiple agents can call.
**What we took:** Cortex is built as a local daemon plus MCP/HTTP interface rather than a memory feature trapped inside any one assistant.
**Status:** Implemented (v0.4.1)

### MemGPT / Letta: LLMs as Operating Systems (2023/2024)
**Link:** https://arxiv.org/abs/2310.08560
**Key insight:** Agents work better when long-term memory lives outside the prompt and is managed through explicit memory operations.
**What we took:** Cortex exposes memory as agent-facing tools instead of trying to solve persistence by stuffing ever larger prompts into context.
**Status:** Implemented (v0.4.1)

## Planned on the v0.5 Roadmap

### Memori (2026)
**Link:** https://arxiv.org/abs/2603.19935
**Key insight:** Semantic triples plus aggressive dedup can preserve accuracy while using a tiny fraction of the original context.
**What we took:** Cortex Phase 2 is planned around semantic-triple extraction and stronger duplicate/supersession handling.
**Status:** Planned (Phase 2)

### A-MAC: Adaptive Memory Admission Control (2026)
**Link:** https://arxiv.org/abs/2603.04549
**Key insight:** Memory admission should be a scored decision using future utility, confidence, novelty, recency, and content type, not an "accept everything" pipeline.
**What we took:** Cortex Phase 4A is planned to add five-factor admission scoring to `cortex_store`.
**Status:** Planned (Phase 4A)

### MemoryOS: Memory Operating System of AI Agent (2025)
**Link:** https://arxiv.org/abs/2506.06326
**Key insight:** Short-, mid-, and long-term memory tiers work best when they have explicit consolidation rules between them.
**What we took:** Cortex Phase 4B is planned around memory maturity tiers and lifecycle-aware promotion from raw traces to stable knowledge.
**Status:** Planned (Phase 4B)

### FluxMem: Choosing How to Remember (2026)
**Link:** https://arxiv.org/abs/2602.14038
**Key insight:** Probabilistic fusion gates beat brittle fixed similarity thresholds when deciding whether memories should merge.
**What we took:** Cortex Phase 4C is planned to replace fixed crystallization thresholds with a BMM-style probabilistic gate.
**Status:** Planned (Phase 4C)

### Collaborative Memory: Multi-User Memory Sharing with Dynamic Access Control (2025)
**Link:** https://arxiv.org/abs/2505.18279
**Key insight:** Shared memory needs provenance, ownership boundaries, and auditability if many agents or users write into the same system.
**What we took:** Cortex Phase 4D is planned to add provenance tracking and clearer shared/private memory boundaries.
**Status:** Planned (Phase 4D)

## Deferred and Future Research

### Memory in the Age of AI Agents (2025)
**Link:** https://arxiv.org/abs/2512.13564
**Key insight:** Agent memory is best understood as a taxonomy across forms, functions, and lifecycle dynamics rather than one flat storage bucket.
**What we took:** This survey shaped how Cortex thinks about factual, experiential, and working memory, but the full taxonomy is not yet encoded as first-class storage metadata.
**Status:** Deferred

### A-Mem: Agentic Memory for LLM Agents (2025)
**Link:** https://arxiv.org/abs/2502.12110
**Key insight:** Structured note-like memories with automatic link generation and memory evolution outperform a pile of isolated entries.
**What we took:** Cortex's co-occurrence tracking points in this direction, but full note evolution and automatic link synthesis are still future work.
**Status:** Deferred

### RGMem: Renormalization Group-inspired Memory Evolution (2024)
**Link:** https://arxiv.org/abs/2510.16392
**Key insight:** Memory consolidation works better as a multi-scale process with dominant patterns and explicit correction terms for important exceptions.
**What we took:** This is the clearest blueprint for a future Cortex crystallization tree beyond today's extractive cluster summaries.
**Status:** Deferred

### MemRL: Self-Evolving Agents via Runtime RL on Episodic Memory (2026)
**Link:** https://arxiv.org/abs/2601.03192
**Key insight:** Retrieval improves when semantic relevance is followed by utility-based ranking learned from downstream success.
**What we took:** Cortex wants this for future Q-value ranking and feedback-driven memory scoring, but the reward model is not built yet.
**Status:** Deferred

### Mem0: Production-Ready AI Agents with Scalable Long-Term Memory (2025)
**Link:** https://arxiv.org/abs/2504.19413
**Key insight:** A production memory layer can improve quality while reducing token cost and latency at the same time.
**What we took:** Mem0 validated the broader product direction behind Cortex, but it did not map to one specific new roadmap phase.
**Status:** Deferred

### HippoRAG: Neurobiologically Inspired Long-Term Memory (2024)
**Link:** https://arxiv.org/abs/2405.14831
**Key insight:** Associative graph traversal can recover connections that flat retrieval misses.
**What we took:** Cortex wants this for future graph-based multi-hop recall, especially for codebase and decision-chain questions.
**Status:** Deferred

### MAGMA: Multi-Graph Agentic Memory Architecture (2026)
**Link:** https://arxiv.org/abs/2601.03236
**Key insight:** Semantic, temporal, causal, and entity relationships should be separate memory views, not collapsed into one embedding space.
**What we took:** This is a future direction for Cortex once semantic retrieval alone stops being enough.
**Status:** Deferred

### Mem-alpha: Learning Memory Construction via Reinforcement Learning (2025)
**Link:** https://arxiv.org/abs/2509.25911
**Key insight:** What to store, keep, and crystallize can be learned from task success instead of hand-tuned heuristics.
**What we took:** Long-term, Cortex should replace manual thresholds with learned policies, but that is beyond the current roadmap.
**Status:** Deferred

### ProMem: Beyond Static Summarization (2026)
**Link:** https://arxiv.org/abs/2601.04463
**Key insight:** Self-questioning and iterative refinement recover more useful memory than one-shot summaries.
**What we took:** Cortex wants to move crystals from extractive summaries toward iterative refinement, but that work is still future-facing.
**Status:** Deferred

### MemSearcher: Training LLMs to Search and Manage Memory via RL (2025)
**Link:** https://arxiv.org/abs/2511.02805
**Key insight:** Small models with the right memory policy can outperform larger models with weaker search and retention behavior.
**What we took:** This supports a future local memory-manager model inside Cortex, but no such training pipeline exists yet.
**Status:** Deferred

### RAPTOR: Recursive Abstractive Processing for Tree-Organized Retrieval (2024)
**Link:** https://arxiv.org/abs/2401.18059
**Key insight:** Bottom-up summary trees let retrieval work at multiple levels of abstraction.
**What we took:** RAPTOR is a direct reference for future hierarchical crystals and project-level abstractions in Cortex.
**Status:** Deferred

### LLMLingua-2 (2024)
**Link:** https://arxiv.org/abs/2310.05736
**Key insight:** Compression can improve quality when it removes noise instead of just cutting tokens.
**What we took:** Cortex already compresses boot context aggressively, and this paper informs future density-aware context selection rather than a simple token cap.
**Status:** Deferred

### MIRIX: Multi-Agent Memory System for LLM-Based Agents (2025)
**Link:** https://arxiv.org/abs/2507.07957
**Key insight:** Agents perform better when a memory router chooses which memory type and search strategy a query should use.
**What we took:** Cortex wants an active-retrieval router for semantic, keyword, temporal, and co-occurrence paths, but that is not yet scheduled.
**Status:** Deferred

### MemWalker: Walking Down the Memory Maze (2023)
**Link:** https://arxiv.org/abs/2310.05029
**Key insight:** A navigable summary tree is a practical way to search long contexts without flattening everything into one retrieval pass.
**What we took:** This supports future broad-to-specific navigation over Cortex crystals and domain summaries.
**Status:** Deferred

### LazyLLM: Dynamic Token Pruning for Efficient Long Context (2024)
**Link:** https://arxiv.org/abs/2407.14057
**Key insight:** Context should be loaded lazily as the model discovers what matters, not all up front.
**What we took:** Cortex wants this for streaming recall and progressively injected memory instead of front-loading all context at boot.
**Status:** Deferred

### Multi-Layered Memory Architectures: Experimental Evaluation (2026)
**Link:** https://arxiv.org/abs/2603.29194
**Key insight:** Bounded context growth and retention regularization can preserve quality without unbounded memory sprawl.
**What we took:** This is a strong reference for future retention policies and bounded-context guarantees inside Cortex.
**Status:** Deferred

## Why this matters

Cortex is not a generic "AI memory" clone. The current product already ships work inspired by retrieval, fusion, decay, and service-architecture research, and the roadmap is deliberately tied to specific papers instead of vague future ideas.

If you want the short version:
- Phase 1 shipped retrieval ideas from ByteRover, RRF, hybrid retrieval research, and agentmemory.
- Phase 4 is where A-MAC, MemoryOS, FluxMem, and Collaborative Memory map into the roadmap.
- The rest of the survey is Cortex's backlog of serious memory ideas, not marketing filler.

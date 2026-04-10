# Cortex Research

This page is the public research map behind Cortex.

It exists for a simple reason: if Cortex claims a feature came from research, the paper or open-source reference should be visible, linked, and tied to a concrete product decision.

## How to read this page

- Academic papers use `What we took` to describe the design idea Cortex adopted, tested, or put on the roadmap.
- Open-source and non-paper references use `Inspired by` to avoid implying code reuse where this page is only documenting design influence.
- `Implemented` means the idea materially shaped shipped Cortex behavior.
- `Planned` means the idea is already mapped to a named roadmap phase.
- `Deferred` means the paper is important, but Cortex has not committed that work to the current roadmap.

## At a glance

| Area | Main references | Cortex mapping |
|---|---|---|
| Retrieval | ByteRover, RRF, Rethinking Hybrid Retrieval, agentmemory | Phase 1 shipped hybrid + fused retrieval |
| Admission and lifecycle | A-MAC, MemoryOS, Memori | Planned for Phase 2 and Phase 4 |
| Crystallization | FluxMem, RGMem, ProMem, RAPTOR | Partly shipped today, larger redesign deferred |
| Multi-agent memory | Collaborative Memory, MaaS, MIRIX | Shared daemon shipped, stronger provenance and routing planned |
| Compression and context budgets | LLMLingua-2, Active Context Compression, LazyLLM | Cortex already compresses boot context; finer-grained compression is future work |

## Already reflected in Cortex

### ByteRover: Agent-Native Memory Through LLM-Curated Hierarchical Context (2026)
**Link:** https://arxiv.org/abs/2604.01599  
**Key insight:** Retrieval quality improves when the system resolves easy queries cheaply first, then escalates only when needed. The paper also treats memory as something that matures over time rather than a flat bag of facts.  
**What we took:** Cortex Phase 1 adopted progressive retrieval and stronger field-aware ranking from this direction of work. ByteRover's memory maturity model also directly informs the planned Phase 4B lifecycle work.  
**Status:** Implemented (v0.5.0)

### Reciprocal Rank Fusion (Cormack, Clarke, Buettcher) (2009)
**Link:** https://dl.acm.org/doi/10.1145/1571941.1572114  
**Key insight:** Simple reciprocal-rank fusion consistently beats trusting any single retrieval signal on its own. It is a practical way to blend rankings without overfitting to one retriever's weaknesses.  
**What we took:** Cortex Phase 1 uses RRF-style fusion to combine keyword, semantic, and other retrieval signals into one final ranking. This was a foundational retrieval choice, not an incidental tuning pass.  
**Status:** Implemented (v0.5.0)

### Rethinking Hybrid Retrieval (2025)
**Link:** https://arxiv.org/abs/2506.00049  
**Key insight:** A well-tuned hybrid stack with compact embedding models plus reranking beats naive "bigger model, bigger index" retrieval. The paper is especially useful for teams that want strong recall without heavyweight infrastructure.  
**What we took:** Cortex doubled down on a local MiniLM-centered hybrid approach instead of assuming larger embedding models would solve retrieval quality. The reranking part remains a deferred follow-on, but the core hybrid lesson already shipped in Phase 1.  
**Status:** Implemented (v0.5.0)

### Episodic Memory is the Missing Piece for Long-Term LLM Agents (2025)
**Link:** https://arxiv.org/abs/2502.06975  
**Key insight:** Long-term agents need more than static facts; they need episodic traces with temporal context, interference management, and consolidation into more durable knowledge. The paper is especially strong on why one-shot facts alone do not recreate working context.  
**What we took:** Cortex's split between raw session traces and more durable summarized knowledge fits this framing closely. It also reinforced the need for conflict, supersession, and session-aware memory handling in the store.  
**Status:** Implemented (v0.4.1)

### MemoryBank: Enhancing Large Language Models with Long-Term Memory (2024)
**Link:** https://arxiv.org/abs/2305.10250  
**Key insight:** Memory should not age uniformly. Some memories strengthen when recalled, while others should decay or disappear if they stop being useful.  
**What we took:** Cortex's decay-oriented scoring and memory reinforcement direction are aligned with this line of work. MemoryBank helped validate the idea that persistence without lifecycle control becomes clutter, not intelligence.  
**Status:** Implemented (v0.4.1)

### Memory as a Service (MaaS) (2025)
**Link:** https://arxiv.org/abs/2506.22815  
**Key insight:** Memory becomes more powerful when it is exposed as a governed service rather than trapped inside one agent's private state. The paper also argues that different tasks need composable memory views, not one monolithic recall stream.  
**What we took:** Cortex is intentionally built as a local memory daemon with MCP and HTTP surfaces instead of embedding persistence inside one assistant integration. MaaS is one of the clearest validations of that architectural choice.  
**Status:** Implemented (v0.4.1)

### MemGPT / Letta: LLMs as Operating Systems (2023/2024)
**Link:** https://arxiv.org/abs/2310.08560  
**Key insight:** Agents benefit when long-term memory lives outside the prompt and is managed through explicit memory operations. The model should interact with memory as a system, not treat prompt length as the only persistence mechanism.  
**What we took:** Cortex exposes memory through tools and a daemon boundary rather than trying to solve persistence by growing the prompt forever. This paper helped justify the "memory as infrastructure" stance early in the project.  
**Status:** Implemented (v0.4.1)

## Open-source and non-paper influence

For open-source repositories and other non-paper references, this page uses `Inspired by` wording on purpose. That keeps the distinction clean: this is documenting design influence, not claiming code reuse. If Cortex ever incorporates copied code or vendored assets, that belongs in code-level attribution and license notices, not in this page.

### agentmemory (GitHub)
**Link:** https://github.com/rohitg00/agentmemory  
**Key insight:** Practical long-horizon memory systems can combine multiple retrieval streams, lightweight quality scoring, and Jaccard-style memory hygiene without needing a complicated platform. GitHub currently marks the repository as Apache-2.0.  
**Inspired by:** Cortex Phase 1 took inspiration from agentmemory's triple-stream retrieval mindset and its habit of treating dedup and supersession as first-class maintenance work. This page documents design influence only; it does not imply code import.  
**Status:** Implemented (v0.5.0)

## Planned roadmap work

### Memori (2026)
**Link:** https://arxiv.org/abs/2603.19935  
**Key insight:** Semantic triples and aggressive dedup can preserve answer quality while using dramatically less context. The paper is especially relevant for any system that wants to remember relationships instead of just raw snippets.  
**What we took:** Cortex Phase 2 is planned around stronger semantic structure and duplicate cleanup. Memori is the clearest reference for moving from flat stored text toward relationship-aware memory objects.  
**Status:** Planned (Phase 2)

### A-MAC: Adaptive Memory Admission Control (2026)
**Link:** https://arxiv.org/abs/2603.04549  
**Key insight:** Memory admission should be a scored decision across utility, confidence, novelty, recency, and content type instead of an "accept everything" policy. It is a practical answer to memory sprawl.  
**What we took:** Cortex Phase 4A is planned to add admission control to `cortex_store` so durable memory becomes selective, not automatic. A-MAC is the main reference for that gate.  
**Status:** Planned (Phase 4A)

### MemoryOS: Memory Operating System of AI Agent (2025)
**Link:** https://arxiv.org/abs/2506.06326  
**Key insight:** Agent memory works better when short-, mid-, and long-term storage are treated as explicit tiers with promotion rules between them. The operating-system analogy is useful because it turns memory lifecycle into an architecture problem, not just a ranking tweak.  
**What we took:** Cortex Phase 4B is planned around maturity tiers and clearer promotion rules from raw traces to validated and stable knowledge. MemoryOS is one of the strongest references for that shift.  
**Status:** Planned (Phase 4B)

### FluxMem: Choosing How to Remember (2026)
**Link:** https://arxiv.org/abs/2602.14038  
**Key insight:** Fixed similarity thresholds are fragile. Probabilistic fusion gates are more robust when deciding whether memories belong together.  
**What we took:** Cortex Phase 4C is planned to replace brittle crystallization thresholds with a BMM-style probabilistic gate. FluxMem is the direct reference for that change.  
**Status:** Planned (Phase 4C)

### Collaborative Memory: Multi-User Memory Sharing with Dynamic Access Control (2025)
**Link:** https://arxiv.org/abs/2505.18279  
**Key insight:** Shared memory only scales safely when provenance, ownership boundaries, and auditability are attached to each fragment. This matters even more when many agents write into the same system.  
**What we took:** Cortex Phase 4D is planned to add stronger provenance tracking and clearer shared/private boundaries. Collaborative Memory is the most directly relevant paper for that roadmap item.  
**Status:** Planned (Phase 4D)

## Deferred and future-facing research

### Memory in the Age of AI Agents (2025)
**Link:** https://arxiv.org/abs/2512.13564  
**Key insight:** Agent memory is a design space with multiple forms, functions, and lifecycle stages, not a single retrieval layer. The paper is especially useful as a taxonomy and field map.  
**What we took:** Cortex uses this as a strategic framing reference for future storage metadata, retrieval routing, and memory automation. It influenced the shape of the roadmap more than any one shipped feature.  
**Status:** Deferred

### A-Mem: Agentic Memory for LLM Agents (2025)
**Link:** https://arxiv.org/abs/2502.12110  
**Key insight:** Structured note-like memories with automatic link generation and memory evolution outperform a pile of isolated entries. This paper is strong on the idea that storing a new memory should trigger re-evaluation of related old ones.  
**What we took:** Cortex's co-occurrence work points in this direction, but full link synthesis and evolving note structures are still future work. A-Mem remains one of the clearest references for "memory that updates memory."  
**Status:** Deferred

### RGMem: Renormalization Group-inspired Memory Evolution (2024)
**Link:** https://arxiv.org/abs/2510.16392  
**Key insight:** Consolidation works better as a multi-scale process with dominant patterns and explicit correction terms for exceptions. It is one of the most principled alternatives to today's threshold-plus-summary style memory consolidation.  
**What we took:** Cortex treats RGMem as a blueprint for a future hierarchical crystal system beyond today's extractive cluster summaries. It is especially important for the long-term crystallization redesign.  
**Status:** Deferred

### MemRL: Self-Evolving Agents via Runtime RL on Episodic Memory (2026)
**Link:** https://arxiv.org/abs/2601.03192  
**Key insight:** Retrieval improves when semantic relevance is followed by utility ranking learned from downstream task success. This shifts memory from static ranking toward feedback-shaped usefulness.  
**What we took:** Cortex wants this for future Q-value scoring and feedback-driven memory ranking, especially for code tasks where some memories repeatedly help and others never do. The reward model is not built yet.  
**Status:** Deferred

### Mem0: Production-Ready AI Agents with Scalable Long-Term Memory (2025)
**Link:** https://arxiv.org/abs/2504.19413  
**Key insight:** Long-term memory can be productionized with measurable latency and token wins, not just benchmark novelty. It is a useful proof that memory can be a product feature instead of a lab demo.  
**What we took:** Mem0 validated the broader product direction behind Cortex's daemon-first architecture. It informed confidence in the category more than one specific roadmap phase.  
**Status:** Deferred

### HippoRAG: Neurobiologically Inspired Long-Term Memory (2024)
**Link:** https://arxiv.org/abs/2405.14831  
**Key insight:** Associative graph traversal can recover connections that flat retrieval misses, especially for multi-hop questions. Its main value is not raw storage, but better linking.  
**What we took:** Cortex wants this for future graph-based multi-hop recall across decisions, files, and sessions. HippoRAG remains a strong reference for that direction.  
**Status:** Deferred

### MAGMA: Multi-Graph Agentic Memory Architecture (2026)
**Link:** https://arxiv.org/abs/2601.03236  
**Key insight:** Semantic, temporal, causal, and entity relationships should be separate memory views rather than collapsed into one embedding space. This is especially compelling for agent systems that need to reason, not just retrieve.  
**What we took:** Cortex sees MAGMA as the clearest case for eventually expanding beyond pure semantic retrieval. It is a future direction once the current hybrid stack stops being enough.  
**Status:** Deferred

### Mem-alpha: Learning Memory Construction via Reinforcement Learning (2025)
**Link:** https://arxiv.org/abs/2509.25911  
**Key insight:** What to store, keep, and crystallize can be learned from task success rather than hand-tuned forever. The paper is important because it turns memory policy into something trainable.  
**What we took:** Long-term, Cortex should replace manual thresholds and rigid heuristics with learned memory policies. Mem-alpha is part of that long-horizon research case.  
**Status:** Deferred

### ProMem: Beyond Static Summarization (2026)
**Link:** https://arxiv.org/abs/2601.04463  
**Key insight:** Self-questioning and iterative refinement recover more useful memory than one-shot summaries. Static summaries miss future-relevant details because they are written before the future task is known.  
**What we took:** Cortex wants to move crystals from extractive summaries toward iterative refinement and lazy revisiting. ProMem is the strongest reference for that shift.  
**Status:** Deferred

### MemSearcher: Training LLMs to Search and Manage Memory via RL (2025)
**Link:** https://arxiv.org/abs/2511.02805  
**Key insight:** Search quality and memory policy matter as much as model size. The paper is especially useful as evidence that smaller systems can outperform larger ones when memory is managed well.  
**What we took:** This strengthens Cortex's bias toward smarter memory policy over simply loading more context or chasing bigger models. It is strategic validation for future local memory-manager work.  
**Status:** Deferred

### RAPTOR: Recursive Abstractive Processing for Tree-Organized Retrieval (2024)
**Link:** https://arxiv.org/abs/2401.18059  
**Key insight:** Bottom-up summary trees let retrieval operate at multiple levels of abstraction. This is a strong pattern for systems that need both broad summaries and specific evidence.  
**What we took:** RAPTOR is a major reference for future hierarchical crystals, domain summaries, and broad-to-specific retrieval inside Cortex.  
**Status:** Deferred

### LLMLingua-2 (2024)
**Link:** https://arxiv.org/abs/2310.05736  
**Key insight:** Compression can improve quality when it removes noise instead of simply minimizing token count. The practical lesson is that denser context can outperform longer context.  
**What we took:** Cortex already compresses boot context aggressively, but LLMLingua-2 informs future signal-aware context packing rather than blind budget trimming.  
**Status:** Deferred

### MIRIX: Multi-Agent Memory System for LLM-Based Agents (2025)
**Link:** https://arxiv.org/abs/2507.07957  
**Key insight:** Queries should be routed to the right memory type and search strategy instead of being pushed through one universal recall path. This is especially relevant for agentic systems with heterogeneous memory.  
**What we took:** Cortex wants a future active-retrieval router for semantic, keyword, temporal, and co-occurrence paths. MIRIX is one of the clearest references for that routing layer.  
**Status:** Deferred

### MemWalker: Walking Down the Memory Maze (2023)
**Link:** https://arxiv.org/abs/2310.05029  
**Key insight:** A navigable summary tree is a practical way to search long contexts without flattening everything into one pass. The important contribution is structured navigation, not just summarization.  
**What we took:** Cortex treats MemWalker as supporting evidence for future broad-to-specific navigation over crystals and project summaries.  
**Status:** Deferred

### LazyLLM: Dynamic Token Pruning for Efficient Long Context (2024)
**Link:** https://arxiv.org/abs/2407.14057  
**Key insight:** Context should be loaded lazily as relevance becomes clearer instead of front-loading every possibly useful token. This is a strong fit for agent workflows where task intent sharpens over time.  
**What we took:** Cortex wants this for streaming recall and progressive memory injection rather than loading all context at boot. The protocol changes are still future work.  
**Status:** Deferred

### Multi-Layered Memory Architectures: Experimental Evaluation (2026)
**Link:** https://arxiv.org/abs/2603.29194  
**Key insight:** Bounded context growth and retention regularization can preserve quality without unbounded memory sprawl. The main offering here is disciplined memory budgeting rather than one flashy algorithm.  
**What we took:** Cortex sees this as an important reference for future retention policy, cleanup pressure, and bounded-context guarantees.  
**Status:** Deferred

### SmartSearch (2026)
**Link:** https://arxiv.org/abs/2603.15599  
**Key insight:** Ranking quality matters more than clever structure if the final ordering still fails to surface the right evidence. The paper also argues that strong ranking can cut token usage substantially.  
**What we took:** Cortex treats SmartSearch as a deferred retrieval-quality reference for future reranking and token-efficiency work beyond the current Phase 1 fusion stack.  
**Status:** Deferred

### MemFactory (2026)
**Link:** https://arxiv.org/html/2603.29493  
**Key insight:** Memory operations can be optimized with reinforcement learning instead of static heuristics. The paper is especially relevant for admission, update, and consolidation decisions.  
**What we took:** Cortex uses MemFactory as part of the long-term case for learned memory policy and a future local memory manager.  
**Status:** Deferred

### DS@GT Fusion (2026)
**Link:** https://arxiv.org/abs/2601.15518  
**Key insight:** Hybrid retrieval becomes much stronger when sparse and dense search are followed by a better reranking stage. This is another reminder that retrieval quality often lives in the final ranking layer.  
**What we took:** Cortex treats this as a deferred retrieval-quality reference for a future reranking pass over the current hybrid stack.  
**Status:** Deferred

### Active Context Compression (2026)
**Link:** https://arxiv.org/abs/2601.07190  
**Key insight:** Compression should be an active, task-aware process instead of a one-time static trim. The paper is useful for systems that care about latency and context budgets at the same time.  
**What we took:** Cortex sees this as future work for more adaptive boot compression and post-retrieval context shaping.  
**Status:** Deferred

### HyDE: Precise Zero-Shot Dense Retrieval without Relevance Labels (2022)
**Link:** https://arxiv.org/abs/2212.10496  
**Key insight:** Generating a hypothetical document before retrieval can improve recall for short or under-specified queries. It is a useful trick when the original query is too sparse.  
**What we took:** Cortex keeps HyDE in the backlog as a possible future aid for vague or low-information recall requests.  
**Status:** Deferred

## Why this matters

Cortex is not a generic "AI memory" clone that name-drops papers after the fact.

The current product already ships retrieval choices influenced by real research, the roadmap is tied to specific references rather than vague aspirations, and the backlog is public enough that contributors can disagree with it, improve it, or replace parts of it with better work.

If you want the shortest possible version:
- Phase 1 shipped retrieval ideas from ByteRover, RRF, hybrid retrieval research, and open-source inspiration from agentmemory.
- Phase 2 and Phase 4 are where Memori, A-MAC, MemoryOS, FluxMem, and Collaborative Memory map into named roadmap work.
- The rest of this page is Cortex's open research backlog, not marketing filler.

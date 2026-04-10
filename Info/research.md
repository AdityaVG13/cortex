# Cortex Research

**Last reviewed:** 2026-04-10

Cortex is built in public. This page is the public design record behind the product: which papers influenced shipped behavior, which ideas are mapped to named roadmap phases, which references only shaped long-term direction, and where Cortex intentionally implemented ideas in its own way.

## How to read this page

- Academic papers are listed as research inputs, not claims of direct implementation.
- Open-source and non-paper references use `Inspired by` wording on purpose. That language documents design influence, not code reuse.
- `What we thought was cool` names the idea worth stealing intellectually.
- `How Cortex adapted it` explains how the idea changed once it hit Cortex's local-first, single-binary, MCP-native constraints.
- `Implemented` means the idea materially shaped shipped Cortex behavior.
- `Planned` means the idea is already tied to a named phase or roadmap direction.
- `Deferred` means the work is important, but not committed to the current build plan.

## Research -> Product Map

| Area | Main references | Cortex mapping |
|---|---|---|
| Retrieval | ByteRover, RRF, Rethinking Hybrid Retrieval, SmartSearch, DS@GT Fusion, HyDE, agentmemory | Phase 1 shipped hybrid retrieval and fusion; reranking and short-query expansion are still future work |
| Structure and dedup | Episodic Memory, MemoryBank, Memori, A-Mem | Raw traces plus durable knowledge are already split; semantic triples and deeper dedup are planned for Phase 2 |
| Lifecycle and crystallization | A-MAC, MemoryOS, FluxMem, RGMem, ProMem, Mem-alpha, MemFactory | Admission control, memory tiers, and probabilistic crystallization are planned; learned policies are deferred |
| Multi-agent memory | MaaS, MemGPT, Collaborative Memory, MIRIX, MAGMA | Cortex already runs as a shared daemon; stronger provenance, routing, and memory-view separation are planned |
| Compression and context budgets | LLMLingua-2, Active Context Compression, LazyLLM, RAPTOR, MemWalker | Cortex already compresses boot context; active compression and tree-style retrieval remain future-facing |

## What Cortex changes when it adapts research

- Cortex is local-first. Papers that assume hosted services or heavyweight orchestration get translated into SQLite, ONNX, and local process boundaries.
- Cortex is a memory system for coding workflows, not a general consumer chat product. Retrieval, decay, and provenance all have to work under engineering constraints.
- Cortex exposes memory through MCP and HTTP instead of embedding persistence inside one assistant. That changes how memory operations, auth, and observability are designed.
- Cortex keeps a human in the loop for disputes and governance. Research ideas that assume fully autonomous policy are usually narrowed into auditable, operator-visible systems first.
- Cortex optimizes for product reliability before research completeness. Many deferred papers are strategically attractive, but not yet worth the complexity cost.

## Shipped in Cortex Today

### ByteRover: Agent-Native Memory Through LLM-Curated Hierarchical Context (2026)
- **Link:** https://arxiv.org/abs/2604.01599
- **Key insight:** Retrieval gets better when cheap paths resolve easy queries first and expensive logic is reserved for the hard ones.
- **What we thought was cool:** The paper treats memory as a staged retrieval system and a maturity lifecycle problem instead of a flat vector store.
- **How Cortex adapted it:** Cortex Phase 1 adopted progressive retrieval and stronger field-aware ranking, while the maturity-tier idea moved into later lifecycle planning rather than shipping as-is in the first retrieval pass.
- **Status:** Implemented (Phase 1, v0.5.0)

### Reciprocal Rank Fusion (Cormack, Clarke, Buettcher) (2009)
- **Link:** https://dl.acm.org/doi/10.1145/1571941.1572114
- **Key insight:** Simple reciprocal-rank fusion often beats trusting any single retriever on its own.
- **What we thought was cool:** It is brutally pragmatic: better ranking with almost no theory-heavy machinery in the product path.
- **How Cortex adapted it:** Cortex uses the fusion mindset inside a local hybrid stack, combining keyword and semantic signals without needing a separate reranking service.
- **Status:** Implemented (Phase 1, v0.5.0)

### Rethinking Hybrid Retrieval (2025)
- **Link:** https://arxiv.org/abs/2506.00049
- **Key insight:** A disciplined hybrid stack with compact embeddings and better ranking beats naive "bigger model, bigger index" retrieval.
- **What we thought was cool:** It validates that strong retrieval quality can come from a compact local model plus good architecture, not just larger embeddings.
- **How Cortex adapted it:** Cortex doubled down on MiniLM-centered local retrieval and deferred the heavier reranking layer until it can be justified inside the local-first budget.
- **Status:** Implemented (Phase 1 foundation, reranking deferred)

### Episodic Memory is the Missing Piece for Long-Term LLM Agents (2025)
- **Link:** https://arxiv.org/abs/2502.06975
- **Key insight:** Long-term agents need episodic traces with temporal context, not just decontextualized facts.
- **What we thought was cool:** The paper makes the case that working memory quality depends on remembering how and when something happened, not only what was true.
- **How Cortex adapted it:** Cortex keeps raw session traces separate from more durable summaries and decisions, then layers conflict and supersession handling on top so coding workflows do not collapse into a single undifferentiated memory pool.
- **Status:** Implemented (v0.4.1)

### MemoryBank: Enhancing Large Language Models with Long-Term Memory (2024)
- **Link:** https://arxiv.org/abs/2305.10250
- **Key insight:** Useful memories should strengthen when recalled, while stale ones should fade instead of lingering forever.
- **What we thought was cool:** It treats memory quality as a lifecycle problem rather than a storage-capacity problem.
- **How Cortex adapted it:** Cortex uses the same intuition to justify decay, reinforcement, and cleanup pressure, but grounds it in an auditable local store rather than a hidden agent loop.
- **Status:** Implemented (v0.4.1 direction)

### Memory as a Service (MaaS) (2025)
- **Link:** https://arxiv.org/abs/2506.22815
- **Key insight:** Memory becomes more powerful when it is exposed as a governed service instead of being trapped inside one agent instance.
- **What we thought was cool:** The architecture itself becomes the product: memory as infrastructure rather than memory as an internal implementation detail.
- **How Cortex adapted it:** Cortex made the daemon boundary central from the start, with MCP and HTTP interfaces so different tools can share one local brain without sharing one UI or runtime.
- **Status:** Implemented (v0.4.1)

### MemGPT / Letta: LLMs as Operating Systems (2023/2024)
- **Link:** https://arxiv.org/abs/2310.08560
- **Key insight:** Long-term memory works better when it is managed with explicit operations instead of being stuffed into prompt context.
- **What we thought was cool:** The paper reframes memory as a system boundary problem, which maps well onto real engineering constraints.
- **How Cortex adapted it:** Cortex exposes memory through tools and APIs rather than trying to fake persistence with giant prompts. It keeps the operating-system idea, but translates it into a local daemon with observable behavior.
- **Status:** Implemented (v0.4.1)

## Open-Source and Non-Paper Influence

Open-source references are included because practical design influence matters. They are intentionally labeled as inspiration rather than adoption.

### agentmemory (GitHub)
- **Link:** https://github.com/rohitg00/agentmemory
- **What we thought was cool:** Triple-stream retrieval, simple quality scoring, and Jaccard-style dedup show how far a practical memory system can go without heavyweight infrastructure. GitHub currently labels the repository as Apache-2.0.
- **Inspired by:** Cortex took inspiration from the retrieval stack composition, dedup mindset, and the idea that memory hygiene should be first-class work instead of background cleanup nobody owns.
- **How Cortex implemented it differently:** Cortex keeps the design local-first, daemon-backed, and conflict-aware, with MCP and HTTP as first-class access surfaces and explicit product focus on coding workflows.
- **Status:** Implemented (Phase 1 influence)

## Planned Roadmap Work

### Memori (2026)
- **Link:** https://arxiv.org/abs/2603.19935
- **Key insight:** Semantic triples plus strong dedup can preserve answer quality while using dramatically less context.
- **What we thought was cool:** It moves memory from flat text blobs toward relationship-aware structure without requiring a giant ontology project up front.
- **How Cortex adapted it:** Cortex Phase 2 is planned around stronger semantic structure and duplicate cleanup, but adapted for coding memory where entities, files, decisions, and conventions need to coexist cleanly in one local store.
- **Status:** Planned (Phase 2)

### A-MAC: Adaptive Memory Admission Control (2026)
- **Link:** https://arxiv.org/abs/2603.04549
- **Key insight:** Memory admission should be a scored choice across novelty, utility, confidence, and recency, not an "accept everything" policy.
- **What we thought was cool:** It gives memory sprawl an actual control surface instead of pretending infinite storage is the same thing as intelligence.
- **How Cortex adapted it:** Cortex Phase 4A is planned to add admission logic to `cortex_store`, but keep it explainable and operator-visible instead of burying the decision inside a black-box policy.
- **Status:** Planned (Phase 4A)

### MemoryOS: Memory Operating System of AI Agent (2025)
- **Link:** https://arxiv.org/abs/2506.06326
- **Key insight:** Short-, mid-, and long-term memory work better as explicit tiers with promotion rules.
- **What we thought was cool:** It treats memory lifecycle the way systems engineers treat storage tiers and promotion, which is a much more operational way to think about it.
- **How Cortex adapted it:** Cortex Phase 4B plans to introduce maturity tiers and clearer promotion rules, but grounded in local daemon behavior and product observability rather than an academic simulator.
- **Status:** Planned (Phase 4B)

### FluxMem: Choosing How to Remember (2026)
- **Link:** https://arxiv.org/abs/2602.14038
- **Key insight:** Fixed thresholds are brittle; probabilistic fusion gates are more stable for deciding when memories belong together.
- **What we thought was cool:** It attacks one of the ugliest practical problems in memory systems: threshold tuning that works until it suddenly does not.
- **How Cortex adapted it:** Cortex Phase 4C is planned to replace brittle crystallization thresholds with a BMM-style probabilistic gate, but only after the current simpler system is benchmarked and instrumented well enough to justify the complexity.
- **Status:** Planned (Phase 4C)

### Collaborative Memory: Multi-User Memory Sharing with Dynamic Access Control (2025)
- **Link:** https://arxiv.org/abs/2505.18279
- **Key insight:** Shared memory needs provenance, ownership boundaries, and dynamic access control if multiple writers are involved.
- **What we thought was cool:** It treats memory governance as a first-class design problem, which is exactly what breaks once a project moves from solo use to team use.
- **How Cortex adapted it:** Cortex Phase 4D plans to add stronger provenance tracking and clearer shared/private boundaries, but in a local-first product where the operator can inspect what happened.
- **Status:** Planned (Phase 4D)

## Deferred and Future-Facing Research

### Memory in the Age of AI Agents (2025)
- **Link:** https://arxiv.org/abs/2512.13564
- **Key insight:** Agent memory is a broad design space with multiple forms, functions, and lifecycle stages.
- **What we thought was cool:** It is one of the strongest taxonomy papers in the field and works as a strategic map rather than a single algorithm pitch.
- **How Cortex adapted it:** Cortex uses it as a framing reference for future metadata, routing, and lifecycle work, but not as a direct implementation template.
- **Status:** Deferred

### A-Mem: Agentic Memory for LLM Agents (2025)
- **Link:** https://arxiv.org/abs/2502.12110
- **Key insight:** Structured note-like memories with automatic link generation outperform isolated entries.
- **What we thought was cool:** The paper makes memory evolution feel alive: a new memory should change how old memories are understood.
- **How Cortex adapted it:** Cortex wants that behavior for future co-occurrence and structure updates, but keeps the current system simpler until link synthesis can be made reliable for engineering data.
- **Status:** Deferred

### RGMem: Renormalization Group-inspired Memory Evolution (2024)
- **Link:** https://arxiv.org/abs/2510.16392
- **Key insight:** Consolidation works better as a multi-scale process with dominant patterns plus explicit correction terms.
- **What we thought was cool:** It offers a principled alternative to today's threshold-and-summary style memory consolidation.
- **How Cortex adapted it:** Cortex treats RGMem as a blueprint for a future hierarchical crystal system, but not something to ship before baseline crystallization metrics are stronger.
- **Status:** Deferred

### MemRL: Self-Evolving Agents via Runtime RL on Episodic Memory (2026)
- **Link:** https://arxiv.org/abs/2601.03192
- **Key insight:** Retrieval quality improves when semantic relevance is followed by utility ranking learned from downstream outcomes.
- **What we thought was cool:** It turns "was this memory useful?" into a learnable signal rather than a hand-waved intuition.
- **How Cortex adapted it:** Cortex wants future feedback-shaped ranking for coding tasks, but the current product is not yet ready to trust online learned policy in the critical path.
- **Status:** Deferred

### Mem0: Production-Ready AI Agents with Scalable Long-Term Memory (2025)
- **Link:** https://arxiv.org/abs/2504.19413
- **Key insight:** Long-term memory can be productionized with measurable latency and token wins, not just benchmark novelty.
- **What we thought was cool:** It proves the category is product-real, not just academically interesting.
- **How Cortex adapted it:** Mem0 mainly strengthened confidence in Cortex as a category bet and reinforced the need to treat latency and operations as first-class concerns.
- **Status:** Deferred

### HippoRAG: Neurobiologically Inspired Long-Term Memory (2024)
- **Link:** https://arxiv.org/abs/2405.14831
- **Key insight:** Associative graph traversal can recover links that flat retrieval misses, especially for multi-hop questions.
- **What we thought was cool:** The graph is doing real work here, not just acting as a fancy metadata layer.
- **How Cortex adapted it:** Cortex keeps this in the backlog for future multi-hop recall across decisions, files, and sessions once the current hybrid stack tops out.
- **Status:** Deferred

### MAGMA: Multi-Graph Agentic Memory Architecture (2026)
- **Link:** https://arxiv.org/abs/2601.03236
- **Key insight:** Semantic, temporal, causal, and entity relationships should not all be forced through one embedding view.
- **What we thought was cool:** It is a compelling case for multiple memory projections instead of one universal retrieval path.
- **How Cortex adapted it:** Cortex treats MAGMA as a future architecture reference for when separate memory views are worth the added product and storage complexity.
- **Status:** Deferred

### Mem-alpha: Learning Memory Construction via Reinforcement Learning (2025)
- **Link:** https://arxiv.org/abs/2509.25911
- **Key insight:** What to store, keep, and crystallize can be learned from task success instead of tuned forever by hand.
- **What we thought was cool:** It turns memory policy itself into something trainable, not just retrieval scoring.
- **How Cortex adapted it:** Cortex wants learned memory policy eventually, but will only move there after explainable heuristics and operator controls are mature enough to benchmark against.
- **Status:** Deferred

### ProMem: Beyond Static Summarization (2026)
- **Link:** https://arxiv.org/abs/2601.04463
- **Key insight:** Self-questioning and iterative refinement recover more future-useful memory than one-shot summarization.
- **What we thought was cool:** It directly attacks the weakness of static summaries: they decide what matters before the future task is known.
- **How Cortex adapted it:** Cortex wants future crystals to be revisitable and refinable instead of frozen summaries, but that work depends on better crystallization instrumentation first.
- **Status:** Deferred

### MemSearcher: Training LLMs to Search and Manage Memory via RL (2025)
- **Link:** https://arxiv.org/abs/2511.02805
- **Key insight:** Search quality and memory policy matter as much as model size.
- **What we thought was cool:** It is strong evidence that better memory systems can beat bigger models at the same task.
- **How Cortex adapted it:** Cortex uses this as strategic support for smarter local memory management instead of chasing larger and larger context windows.
- **Status:** Deferred

### RAPTOR: Recursive Abstractive Processing for Tree-Organized Retrieval (2024)
- **Link:** https://arxiv.org/abs/2401.18059
- **Key insight:** Bottom-up summary trees allow retrieval across multiple levels of abstraction.
- **What we thought was cool:** It offers a real path to broad-to-specific retrieval without flattening everything into one index.
- **How Cortex adapted it:** Cortex keeps RAPTOR as a reference for future hierarchical crystals and project-wide summary trees, but has not committed that complexity to the current roadmap.
- **Status:** Deferred

### LLMLingua-2: Data Distillation for Prompt Compression (2024)
- **Link:** https://arxiv.org/abs/2310.05736
- **Key insight:** Compression can improve quality when it removes noise instead of just cutting tokens.
- **What we thought was cool:** It makes compression feel like signal design rather than budget shaving.
- **How Cortex adapted it:** Cortex already compresses boot context aggressively, but LLMLingua-2 mainly informs future signal-aware context shaping beyond today's fixed boot pipeline.
- **Status:** Deferred

### MIRIX: Multi-Agent Memory System for LLM-Based Agents (2025)
- **Link:** https://arxiv.org/abs/2507.07957
- **Key insight:** Different query types should route to different memory types and search strategies.
- **What we thought was cool:** It reframes recall as a routing problem instead of assuming one universal memory search is enough.
- **How Cortex adapted it:** Cortex wants an active-retrieval router for semantic, keyword, temporal, and co-occurrence paths, but that remains future work.
- **Status:** Deferred

### MemWalker: Walking Down the Memory Maze (2023)
- **Link:** https://arxiv.org/abs/2310.05029
- **Key insight:** A navigable summary tree is a practical way to search long contexts without flattening everything into one pass.
- **What we thought was cool:** It makes memory navigation feel inspectable instead of magical.
- **How Cortex adapted it:** Cortex treats MemWalker as supporting evidence for future broad-to-specific navigation over crystals and project summaries.
- **Status:** Deferred

### LazyLLM: Dynamic Token Pruning for Efficient Long Context (2024)
- **Link:** https://arxiv.org/abs/2407.14057
- **Key insight:** Context should be loaded lazily as relevance becomes clearer instead of front-loading every possibly useful token.
- **What we thought was cool:** It matches real coding workflows, where intent sharpens as the task unfolds.
- **How Cortex adapted it:** Cortex wants this for streaming recall and progressive context injection rather than an all-at-once boot wall, but the protocol work is still ahead.
- **Status:** Deferred

### Multi-Layered Memory Architectures: Experimental Evaluation (2026)
- **Link:** https://arxiv.org/abs/2603.29194
- **Key insight:** Bounded context growth and regularized retention can preserve quality without unbounded sprawl.
- **What we thought was cool:** It is less flashy than many papers, but more operationally honest about what memory systems need in production.
- **How Cortex adapted it:** Cortex treats it as a reference for future retention policy and bounded-context guarantees once lifecycle controls are stronger.
- **Status:** Deferred

### SmartSearch (2026)
- **Link:** https://arxiv.org/abs/2603.15599
- **Key insight:** Ranking quality matters more than clever structure if the final ordering still fails to surface the right evidence.
- **What we thought was cool:** The paper is refreshingly direct about the value of ranking over architecture theater.
- **How Cortex adapted it:** Cortex keeps SmartSearch in the backlog as a reranking-quality reference for future retrieval upgrades beyond the current Phase 1 fusion stack.
- **Status:** Deferred

### MemFactory (2026)
- **Link:** https://arxiv.org/html/2603.29493
- **Key insight:** Memory operations can be optimized with reinforcement learning instead of frozen heuristics.
- **What we thought was cool:** It broadens RL from ranking into the whole memory lifecycle.
- **How Cortex adapted it:** Cortex uses MemFactory as part of the long-term case for learned memory policy, but defers it until simpler lifecycle controls are measurable and stable.
- **Status:** Deferred

### DS@GT Fusion (2026)
- **Link:** https://arxiv.org/abs/2601.15518
- **Key insight:** Sparse and dense retrieval become far stronger when followed by a better reranking stage.
- **What we thought was cool:** It shows how much quality still lives in the last ranking step.
- **How Cortex adapted it:** Cortex treats this as a future reference for reranking on top of the existing hybrid stack, not a reason to overcomplicate the first shipping version.
- **Status:** Deferred

### Active Context Compression (2026)
- **Link:** https://arxiv.org/abs/2601.07190
- **Key insight:** Compression should be active and task-aware, not a one-time static trim.
- **What we thought was cool:** It treats compression as a live systems behavior rather than a preprocessing hack.
- **How Cortex adapted it:** Cortex sees this as future work for adaptive boot compression and post-retrieval shaping once the simpler compression path has enough instrumentation behind it.
- **Status:** Deferred

### HyDE: Precise Zero-Shot Dense Retrieval without Relevance Labels (2022)
- **Link:** https://arxiv.org/abs/2212.10496
- **Key insight:** Generating a hypothetical document before retrieval can improve recall for short or underspecified queries.
- **What we thought was cool:** It is a clever way to make weak queries less weak without requiring labels.
- **How Cortex adapted it:** Cortex keeps HyDE in the backlog for vague or low-information recall requests, but it is not yet worth the extra generation step in the local-first critical path.
- **Status:** Deferred

## What Cortex Has Not Implemented Yet

- Learned admission, retention, and crystallization policy are still future work. Cortex currently favors explainable heuristics over opaque learned control.
- Full reranking over the local hybrid stack is not shipped yet. Phase 1 improved retrieval a lot, but ranking quality is still an active frontier.
- Query routing across separate memory views is not implemented yet. Current recall is stronger than before, but still less differentiated than the best research systems.
- Hierarchical summary trees and multi-graph recall remain backlog work, not hidden product behavior.
- Provenance-aware team memory governance is planned, not complete.

## What Cortex Delayed or Rejected for Now

- Hosted-service assumptions were stripped out. Many papers assume cloud services or large orchestration layers that do not fit Cortex's local-first product goal.
- Full autonomy was delayed in favor of operator visibility. Cortex would rather show why memory was admitted, fused, or disputed than hide it inside an invisible policy.
- Copying reference implementations was rejected. Even when external work was influential, Cortex documents inspiration and then builds its own architecture around local process boundaries and coding workflows.
- Overfitting to benchmarks was avoided. Some research ideas look strong in evaluation settings but do not yet justify the operational complexity they would add to Cortex.

## Why This Page Exists

Cortex should be inspectable at the product-design level, not just at the source-code level.

If a feature is research-informed, contributors should be able to see the paper. If an open-source project influenced the design, contributors should be able to see that too. And if Cortex changed an idea substantially, the change should be stated plainly instead of hidden behind vague "inspired by research" language.

That is the standard this page is trying to hold.

## How to Keep This Page Honest

- When a new research-backed feature ships, update the corresponding entry from `Planned` or `Deferred` to `Implemented`.
- When a new paper or repo influences Cortex design, add the link, the interesting idea, and the adaptation note in the same change.
- Prefer `Inspired by` wording for open-source repositories, talks, blog posts, and non-paper references.
- If Cortex intentionally rejects an appealing idea, note that in `What Cortex Delayed or Rejected for Now` instead of leaving the omission ambiguous.

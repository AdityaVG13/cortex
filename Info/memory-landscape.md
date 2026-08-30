<p align="center"><a href="../README.md">← Back to README</a></p>

# Memory landscape

How Cortex sits next to other agent-memory systems. This is a product map, not a benchmark claim.

**Last reviewed:** 2026-08-30

Almost every system in this space answers four questions differently:

| Question | Typical answer | Cortex today |
|----------|----------------|--------------|
| What do I store? | LLM-extracted facts, or a summary tree | The original trace, plus derived anchors and entities |
| Who may change memory? | An extractor (add/update/delete) or the agent rewriting a block | The caller writes; Jaccard conflict; clocks admit on read |
| When do I retrieve? | At chat turn, from the user utterance | At `/recall` / `/boot`, from query + path/symbol/session |
| What is a success? | LoCoMo / LongMemEval “the model remembered the preference” | Honest miss, as-of windows, deterministic `why` |

If you optimize for the last row of the first three columns, you reinstall embeddings. Cortex does not.

---

## Systems

### Mem0

[mem0ai/mem0](https://github.com/mem0ai/mem0) · [arXiv:2504.19413](https://arxiv.org/abs/2504.19413)

Production personalization layer. Write path is an LLM: extract facts, then ADD (current algorithm is add-only; older builds used ADD/UPDATE/DELETE). Retrieval fuses semantic + BM25 + entity linking, with time-aware ranking. Strong published LoCoMo / LongMemEval numbers on the managed platform.

**Better than Cortex at:** unconstrained paraphrase, user-preference memory, SDKs, published scores.

**Weaker at:** abstention, as-of as stored windows, inspectable admit, local empty-home, not inventing a fact that was never stored.

### Supermemory

[supermemoryai/supermemory](https://github.com/supermemoryai/supermemory)

Mem0’s product cousin: temporal graph, learning model that extracts / expires / infers, user profiles in front of search, connectors (Drive, Gmail, Notion, GitHub).

**Better at:** ingesting the user’s life, always-on profile, consumer app.

**Weaker at:** auditing why a derived inference exists; overwrite vs two valid-at intervals.

### Letta (MemGPT)

[letta-ai/letta](https://github.com/letta-ai/letta) · [arXiv:2310.08560](https://arxiv.org/abs/2310.08560)

Not a memory API. A **stateful agent runtime**. Core blocks (`persona`, `human`, objectives) stay in the window. Archival is paged in by tools. The agent self-edits its blocks. Git-backed context repos in newer work.

**Better at:** identity that survives a session without a recall call; the OS metaphor.

**Weaker at:** you live inside Letta; archival search is ordinary retrieval; no honest-miss contract; not a shared brain across Cursor / Claude / Codex unless Letta *is* the loop.

### Eidetic Engine

[Dicklesworthstone/eidetic_engine_cli](https://github.com/Dicklesworthstone/eidetic_engine_cli)

Closest cousin: durable, local, explainable memory for coding agents. Typed kinds (rule, decision, failure, anti-pattern) with decay. `ee pack` with profiles and lenses. Journal → distill → curate (no silent rewrite). Helpful/harmful outcomes change confidence. `ee why` / `ee why-not`. CASS import from Claude / Codex / Cursor sessions. Hybrid BM25 + local Model2Vec, with lexical fallback.

**Ahead of Cortex on:** the product loop (pack, distill, typed kinds, outcome-weighted confidence).

**Behind Cortex on:** four-clock quorum, hard abstention, as-of windows, a daemon several tools share without `ee` becoming the loop, no required embedder even as fallback.

### OptMem

[VictorTaelin/OptMem](https://github.com/VictorTaelin/OptMem)

Radical opposite of Mem0. Tiny prompt, append-only lines, binary summary tree rebuilt from the log, `wake` / `note` / `nap` / `zoom`. Position is identity. Retrieval is regex plus tree navigation. The agent writes summaries at nap time.

**Better at:** always-on identity, compression without a vector DB, zero infrastructure.

**Weaker at:** ACL, as-of, path/symbol, abstention. Summaries are whatever the model typed.

**Idea to steal:** the log is truth; the tree is a derived cache.

### Recuris (paper, not a product)

[arXiv:2608.24876](https://arxiv.org/abs/2608.24876) — *Recursive Experiential–Working Memory Evolution*. There is no shipping product named Recursis; Recuris is the closest match.

Two loops:

1. **Inside a task:** working memory is a verified board (`pending` / `done` / `blocked`). Skills fire at execution events. A checker commits `done` only on a tool receipt.
2. **Across tasks:** a Meta-Agent attributes failure to one component and a gate refuses the patch if held-out tasks regress.

Ablation: adding a skill library does almost nothing. Adding verified working state does a lot. Dumping the whole library into context hurts.

Recuris is a **memory-control layer**, not a store. Cortex has the store. It does not have the control layer.

---

## What Cortex has that they do not combine

- The fact is the stored trace, not an extracted sentence
- Time is a window, not an overwrite
- Empty is a valid answer
- Clocks, anchors, and hops are the admit reason
- HTTP + MCP + Control Center, shared across tools
- Model-free hot path; empty home does not create `~/.cortex/models`

## What Cortex does not have yet

These are open product decisions, not scheduled commits. See [roadmap.md](roadmap.md).

1. **Working memory that is true.** Boot is a snapshot. Recuris / Letta keep a live board. Cortex does not know “return still pending, exchange done” unless an agent stuffed that into a trace.
2. **Metabolism of traces into types.** Everything is a memory/decision row. `ee` has rule / failure / anti-pattern with decay.
3. **Event-shaped recall.** Recuris’s result is that *when* you retrieve beats *what* you stored. Cortex retrieves from English.
4. **Pack as the default loop.** `/recall` is a search API. `ee pack` and OptMem `wake` are “here is what you are.” Cortex boot is halfway there.
5. **Outcomes gating admission.** `used_with` exists. Harmful-weighted confidence that can strip `strong_lexical` does not.
6. **Hierarchical derived view.** Aging compresses text in place. There is no OptMem-style summary tree as a rebuildable index.
7. **LoCoMo / LongMemEval scores.** Deferred on purpose. CQR will lose unconstrained-paraphrase evals against Mem0.

---

## Invention space (not implementation)

Keep CQR as the only *admit* engine. Change what it admits over, and when it is called.

| Direction | Steal from | Cortex-shaped version |
|-----------|------------|------------------------|
| Board-first | Recuris | A `board` target; `done` only from a receipt |
| Pack-first | `ee` / OptMem | Promote boot capsules into `/pack` as the session default |
| Distill-first | Mem0 metabolism, `ee journal` | Offline steward proposes typed heads; CQR admits promoted rules |
| Event-grounded | Recuris invocation policy | `event=edit\|test\|boot` + live board, not only `q=` |
| Outcome-gated rank | `ee` helpful/harmful | Harm cannot admit alone; helpful cannot resurrect superseded |

Those compose. They do **not** require cosine or an LLM on the hot path.

A longer architecture walkthrough of the current engine: [ARCHITECTURE.md](../ARCHITECTURE.md). Papers: [research.md](research.md).

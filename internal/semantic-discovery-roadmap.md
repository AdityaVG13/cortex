# Semantic Discovery Roadmap — recall without models or embeddings

Goal: beat the field at recall while keeping the core bet — **zero LLM
calls, zero embeddings, deterministic admission**. We do not walk the
river (BM25+WordNet ensembles — everyone else's map). We cut the brush:
an invented path from substrate no competitor has.

Law of the program: **route is not admit.** Every technique below widens
the candidate funnel. The admission law still decides, and every widened
candidate carries provenance so `why` can say how it got in.

## The reframe (read this first)

The wall as stated: *paraphrase queries share no surface form with stored
content; bridging requires distributional similarity (embeddings) or
understanding (LLMs).* That wall is shadow. It assumes memory is search —
"what's similar to this query". But memory is **re-finding what mattered**,
and re-finding routes through *purpose, situation, and past success* —
things Cortex already records and no embedding knows:

- **Activity skeleton**: threads, obligations, attempts, checkpoints,
  episodes. A query arrives inside an activity context. Recall can route
  through *what was alive when things like this were alive* — situational
  recall, the way human memory actually works (cue- and context-dependent,
  not similarity-ranked).
- **Outcome ledger**: `outcome_feedback` already separates exposure / use /
  success / credit per scope+task-family, with a held-out-benefit gate.
  Every successful recall is a *paraphrase proof*: query Q found D and the
  outcome was good → Q's terms and D's terms are now linked **for this
  brain**, deterministically, with provenance.
- **Route edges**: `assembly_route_edges(cue, assembly, positive,
  negative)` + `learning_events` with rewards, retractions, and erasure
  handling is a working use-grown association engine with explanations
  (`RouteExplanation`: utility, mass, cues, training units). The machinery
  that learns already exists — it has never been aimed at the paraphrase
  bridge itself.
- **Query frames**: clockwork parses queries into structured frames with
  anchors and signatures. Past queries + their outcomes can become
  first-class memory: route new questions through old successful questions
  (deterministic frame similarity — structured, not vector).

## The invented path (the actual bet)

**Recall as reactivation over a use-grown deterministic association
substrate** — four fused layers, all inspectable, all governed:

1. **Purpose routing** (activity skeleton): query → obligations/attempts/
   threads it bears on → evidence they cite. Relevance through purpose,
   not similarity. Embeddings can't do this — they don't know what the
   agent is trying to do. We do.
2. **Use-grown bridges** (route edges + outcome ledger): paraphrase
   associations learned from *this brain's* successful recalls, with
   positive/negative mass, retractions, and scope isolation. The recall
   system gets smarter the more it's used — zero models. The moat compounds
   into the machinery, not just the content.
3. **Structural graph** (anchors, entities, co-occurrence, conflict links,
   multi-head relations, bitemporal windows): content topology, including
   contradiction-as-signal (competing heads = "about the same thing",
   deterministically).
4. **Query memory** (frames + signatures + outcomes): the history of asking
   as navigable structure.

The click-shaped center: **outcome-attested association as a new CQR
witness class** — "this bridge has worked N times with good outcomes" as
evidence, alongside anchors and lexical witnesses. Learned, but every edge
attributable, retractable, and erasable.

## Growth without ballooning (governing picture)

Everyone else's memory gets bigger; ours gets wiser. Two curves, not one:

- **Content** grows roughly linearly with experience — but cold,
  compressed (deflate), sealed, and tiered (fresh → recent → old →
  archived). Verbatim bytes are never summarized away.
- **Recall power** grows faster than content: every successful recall
  strengthens edges that make *all old bytes* more reachable. A bridge
  learned in month six surfaces a memory from month one.

"Hold more for less" = more recall power per byte, fewer hot bytes per
memory. Memories don't shrink; their *retrieval value density* grows.
Anti-ballooning properties (all already law or substrate):

- **Counters, not copies**: reuse increments edge mass — ~zero bytes per
  recall. Aggregation (assemblies, threads, crystals) links members by
  reference; it never rewrites them.
- **Ephemera expires**: retention classes + GC score thresholds burn chaff;
  durable identity is immune by law.
- **Learning prunes itself**: retractions, negative mass, erasure
  propagation — the substrate forgets deliberately, the way content never
  may. Erasure is unresurrectable (ledger survives restore).
- **Derived summaries are allowed — as labeled derivatives**: an
  LLM-written rollup may exist as a new revision *citing its sources*,
  versioned and retractable. It may never replace them.
- **Multi-harness safe**: principals/scopes isolate writers; content-hash
  (BLAKE3) + RefZero interning dedup identical bytes across harnesses —
  same transcript absorbed once, referenced N times.

Bulk-ingest design (thousands of sessions, LLM-assisted): see
`bulk-ingest-design.md`.

## Phase 0 — Measure first (the optimization target)

- [x] **Recall-positive eval suite** (`tests/contracts/recall_positive.rs`):
  fixed corpus + fixed queries with expected admittances — the mirror of
  `falsifiers.rs`. Seed with paraphrase pairs, temporal questions, update
  chains, multi-session aggregations, abstention controls.
- [ ] **Falsifier regression gate**: every new recall feature must keep all
  must-be-empty cases empty.
- [ ] Report admit rate + expected-doc hit rate per category on every run.

## Phase 1 — The invented program (first, in this order)

- [ ] **Substrate map**: read `assembly/runtime/{learning,compile}`,
  `outcome_feedback` writers, query-frame shapes, thread/obligation
  writers. Document exactly what association substrate exists today and
  what each table's cue/target vocabulary is. (Grounds everything below.)
- [ ] **Query memory**: persist query frames + signatures with their recall
  outcomes; new query → deterministic frame similarity → past successful
  queries → their recalled evidence as candidates. Provenance:
  `expanded:query-memory`.
- [ ] **Use-grown term bridges**: on positive outcome, record query-term ↔
  recalled-doc-term edges (counts, scope-isolated, retractable); expansion
  walks edges above a mass threshold with quorum. Provenance:
  `expanded:bridge(N successes)`. This is the paraphrase learner.
- [ ] **Activity-routed arm**: thread/episode/obligation-scoped candidate
  collection as a new CQR arm — "what was alive with what this query
  touches". Purpose routing, not similarity.
- [ ] **Outcome-attested witness**: the admission-law extension — learned
  bridges corroborated (two independent-origin bridges, or bridge + weak
  lexical). Never a single bridge alone. True unknowns still empty.

## Phase 2 — Table stakes, in parallel (cheap harvest, not the path)

Classical IR the field already mapped. Do it for the free gains; it is not
where we win.

- [ ] BM25 audit, FTS5 trigram option, RRF across arms, 1-round
  pseudo-relevance feedback. Each independently gated by falsifiers.

## Phase 3 — Structure reasoning (funds multi-session + temporal)

- [ ] Personalized PageRank over the deterministic memory graph
  (HippoRAG's best idea minus its LLM dependency — linear algebra over our
  anchors/entities/relations/threads).
- [ ] Deterministic coreference sieves (Stanford multi-sieve style) for
  conversational chains across sessions.
- [ ] Temporal query algebra: rule-based temporal-expression parsing +
  typed ops over the bitemporal schema.
- [ ] Multi-session aggregation recipes over threads/attempts.

## Phase 4 — Answer-side (already ahead, keep pushing)

- [ ] Evidence bundles with traveling qualifiers, contradiction
  attachment, coverage statements — judge-visible quality.

## Admission integration (honesty preserved)

- [ ] Provenance tags on every widened candidate, surfaced in `why`.
- [ ] Calibrated expanded-evidence path (corroboration rules above).
- [ ] Document the updated admission law in `ARCHITECTURE.md` + kernel
  `CONTRACT.md` when it lands.

## Ordering rationale

Measure → invent (use-grown bridges + query memory + activity routing) →
harvest classical gains in parallel → reason over structure → polish
answers. Each step shippable alone and falsifier-gated. The bet: a memory
system that learns *your* paraphrases from *your* outcomes, deterministically
and inspectably, beats generic internet-meaning vectors at the only game
that matters — re-finding what mattered to you.

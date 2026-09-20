# Substrate Map — what can connect to what, and what is still blank

Roadmap Phase 1, item 1. Read from the code at `699d880`, not from memory.
Each substrate names its cue vocabulary, its target vocabulary, its live
producer, its live consumer — and whether the loop is open or closed.

## The one-paragraph map

Cortex has **six** association substrates with working math and **zero**
closed learning loops. Every read path is wired; almost every write path is
a hand-operated CLI, an empty table, or a stub returning `1.0`. The recall
program is therefore not "build learning machinery" — it is **close the
loops**, deterministically, with provenance. The invention surface is wiring,
not math.

## A. Assembly route edges (use-grown, hand-fed)

- Tables: `learning_events` → compiled `assembly_route_edges`.
- Cue vocabulary: `alnum_underscore_lower` token sets (`tokenize_cues`),
  ≤64 cues/event, taken from the raw query text on read.
- Target vocabulary: `assembly_id` (exact member lists over revisions).
- Math: one vote per (training_unit, cue, target); conflicting labels in a
  unit are discarded, not voted; utility = (pos−neg)/(2+mass); deterministic
  order. Only `explicit`/`verified_use` kinds count as reward.
- Producer: `record_learning_event` — called in production **only** by the
  manual CLI (`capture learn-event`). No automatic writer exists.
- Consumer: **live** — every orient/query compiles bundles for the query's
  cues (`attach_assembly_evidence` → `compile_assemblies_for_paths`).
- Gate: `assembly_route_state.enabled` per (principal, scope), default OFF.
- Status: **open loop**. Read wired, write starved, usually disabled.

## B. Observation associations (co-occurrence, opt-in)

- Tables: `observation_association_incidence` + `observation_association_feedback`.
- Cue vocabulary: lowercase alnum tokens, 3–64 chars, 10 stopwords, ≤32 per
  source, ≤512 sources, over `document`/`user_statement` observations only.
- Target vocabulary: `source_id` reached via a cue→alias bridge: alias must
  co-occur with the cue in ≥2 independent lineages (digest/lineage
  dedup — a copied revision is never a second witness).
- Math: score = min(support,8)/8 ± clamp(feedback,−2,+2)/8. Navigation
  hints only — explicitly never witnesses.
- Producer: incidence auto-derived on rebuild/maintain (opt-in per scope);
  feedback by hand CLI (`capture assess`) only.
- Consumer: **live** as the `learned` channel in observation queries.
- Status: **half-open**. Incidence flows; usefulness feedback is manual.

## C. Clock anchors + links (auto-projected topology)

- Tables: `clock_anchors` + `clock_anchor_evidence` + `clock_links`.
- Cue vocabulary: 15 anchor kinds (citation, path, symbol, entity, ticket,
  error_code, command, flag, url_host, quoted_phrase, term, acronym, goal,
  session, source) with specificity 0–3; ≤64/trace, ≤32/query.
- Target vocabulary: (target_type, target_id) rows; link relations:
  updates, extends, same_goal, same_path, same_symbol, observed_with,
  used_with, caused_by, supports.
- Producer: `project_target` on every deposit (auto); shared strong anchors
  auto-link. `record_used_with` (**Feedback** origin) has **zero callers**.
- Consumer: **live** — anchor/truth/task/history/hop arms + quorum.
- Status: **open loop on the feedback edge only**. Structure flows;
  use never feeds back into links.

## D. Query frames (parsed, never remembered)

- Shape: `QueryFrame` — terms, quoted phrases, typed anchors, entity ids
  (+expanded), temporal mode + as-of, owner/session/goal, paths, symbols,
  head. `query_signature` = BLAKE3 of the canonical payload.
- Producer: `parse_query_frame` on every query (in memory only).
- Consumer: none persist. No query-memory table exists.
  `recall_feedback.query_signature` column exists and **nothing writes it**.
- Status: **blank region**. The richest cue structure in the system
  evaporates after each query.

## E. outcome_feedback (paraphrase proofs, unmined)

- Table: `outcome_feedback` — scope, task_family, task, prior_view_receipt,
  selected_action, outcome, exposed[], used[], harmful_reuse, wrong_scope.
- Producer: **live** — the `feedback` op writes it atomically with
  `agent_feedback`; exposure resolves from the prior View's receipt aliases.
- Consumer: **none in production**. `family_stats` is read only by tests.
  The held-out-benefit bandit gate (`adaptive_policy`, n≥30) never fires
  because nothing runs the comparison.
- Status: **open loop**. Every successful recall's paraphrase proof
  (query Q found D, outcome good) is recorded and never read.

## F. agent_feedback (task outcomes, stats-only)

- Table: `agent_feedback` — agent, task_class, outcome/score, latency,
  retries, tokens, memory_sources[], notes.
- Producer: **live** via the `feedback` op.
- Consumer: stats endpoints only. Never touches recall.
- Status: **open loop** (by design so far — no recall consumer exists).

## G. recall_feedback (read live, written never)

- Table: `recall_feedback` — query_text, query_embedding (embedding-era
  vestige, unfilled), query_signature (unwritten), result_source,
  result_type, result_id, signal, agent.
- Producer: **none in production**. Only the compaction re-aggregator
  re-inserts; no live path records a signal.
- Consumers: **all live** — ranker (`score.rs`: pos − 2·neg per source),
  `compute_boosts` (±clamp, 30-day half-life), aging immunity (≥5 positive
  in 14 days), GC thresholds.
- `query_similarity_weight` is a stub returning `1.0`: boosts are not
  query-conditioned at all.
- Status: **open loop, inverted**. Live ranking math over a table that
  production never fills.

## H. Activity skeleton (threads, unlinked to content)

- Tables: `threads`, `thread_members` (roles: obligation/attempt/
  checkpoint), `obligations` (8-state machine, checker-verified completion,
  artifact-bound reopen), attempt/checkpoint revisions.
- Producer: **live** via the `checkpoint` op family.
- The gap: **deposits never join `thread_members`**. A deposit's thread
  label becomes a scope *anchor* (task-clock evidence) only. No recall arm
  reads `thread_members` for candidates (`thread_summary` and `reflex`
  are the only readers).
- Status: **two planes, one missing edge**. Purpose routing ("what was
  alive with what this query touches") has no membership edge to walk.

## Admission (the law all loops must respect)

- Six collector arms: lexical/write (FTS5 porter+unicode61), anchor,
  truth (+entity graph), task, history, hop.
- Six witness domains: lexical, anchor, entity, task, history, hop.
  Independent support = distinct origins; derived witnesses inherit the
  seed's origin (one traversal = one vote, however many counters).
- Hard anchor or quorum admits; `use_score` in tie-breaks draws from the
  empty `recall_feedback` table.
- Route is not admit: every substrate above widens candidates; the law
  decides. Any new witness class (outcome-attested bridges included)
  lands here, corroborated, never alone.

## Blank regions (all six closed; the wires as built)

1. [x] **Query memory table** — `query_memory` persists (principal,
   signature, terms, strong anchors, asks, successes, evidence).
   Similar successful queries expand recall via `bridge.rs`.
2. [x] **Outcome → learning_events wire** — success records `verified_use`
   events (cues = query tokens, target = learned singleton assembly,
   training unit = receipt); harmful records −1. Routes refresh inline.
3. [x] **Outcome → recall_feedback wire** — success +1, partial +0.5,
   harmful −1, failure none; query text resolves from the receipt (views
   now store the bounded need), signature recomputed deterministically.
   `query_similarity_weight` still stubbed — next turn of this loop.
4. [x] **Term-bridge edges** — `term_bridges` counts query × doc token
   pairs; mass ≥ 2 unvetoed pairs expand queries. The paraphrase
   learner; principal-scoped, maintenance-trimmed.
5. [x] **Activity-routed arm** — threaded deposits join `thread_members`
   (role `deposit`); the `activity` arm routes thread siblings as
   task-vote candidates. Runs last so hop traversal still reaches them.
6. [x] **Outcome-attested witness** — co-used sources link `used_with`
   from feedback; the hop arm's existing feedback grant (Task direct,
   "feedback" key) pairs with the route for quorum. Harmful rejects;
   `hop_relation` now honors rejection. No new domain needed — the
   codebase's own grain (feedback = Task usefulness, never truth) held.

## Reframe (read after PUSH-PROMPT)

The wall as the field states it — *paraphrase needs distributional
similarity* — is shadow here twice over. First, the system already records
purpose (threads), success (outcomes), and use (exposure vs. use vs.
credit) — things no embedding knows. Second, the learning math is already
built three times over (routes, associations, clock links) with
deterministic tie-breaks, retractions, erasure propagation, and
lineage-counted independence. What is missing was never machinery. It is
*causation*: nothing that happens during successful recall causes anything
to be learned. Six open loops, six wires to close — each shippable alone,
each falsifier-gated, each making all old bytes more reachable. The moat
compounds into the machinery. Cut there.

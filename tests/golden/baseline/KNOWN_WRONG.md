# Known-wrong outputs frozen in `six_arm_recall.golden`

The golden freezes *current* behavior so later changes are deliberate. The
following lines are known defects, not truth. Each has a failing contract in
`tests/contracts/baseline_fixtures.rs` and a fixing bead.

| Query | Wrong line | Why it is wrong | Fixing bead |
|---|---|---|---|
| `PAY-77` | (FIXED) `src/pay/retry.rs handles idempotent replay …` was admitted through a single derived route: the hop collector minted write=1/truth=1 and the anchor/entity arms marked expansion-reached rows as hard anchors. Lineage-aware admission (`admit_with_lineage`, provenance witnesses, qualified anchor matching, expansion never hard) now leaves it out of the supported set; it can appear only as a lead. | done — witness lineage |

Deliberate golden change recorded with that fix: rows reached through query
expansion (sibling anchor → entity) are admitted by `clock_quorum` when two
direct channels support them, never labelled `hard_anchor`.

Boot delta cursor (FIXED): boot now carries a self-contained `## Constraints`
capsule of durable decisions on every boot with an explicit omission count, and
the operations surface decides suppression only on host-attested presence
(`cortex_logic::presence`). Contract
`counterexample_cursor_must_not_withhold_unchanged_decision` passes.
`dedup_and_mark_served` in recall/engine_support.rs is the same shape but is
currently dead code (no callers).

Regenerate: `UPDATE_GOLDENS=1 cargo test -p cortex-tests --test baseline_fixtures`
then review `git diff tests/golden/baseline` and update this table.

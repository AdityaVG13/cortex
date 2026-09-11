<p align="center"><a href="../README.md">← Back to README</a></p>

# MCP Tool Reference

> `tools/list` advertises the **eight semantic operations** below. Source of truth:
> `crates/daemon/src/handlers/operations/`. A small set of legacy names remains
> callable by exact name and routes onto those operations or a specialised
> dispatcher. Removed historical names return `UNKNOWN_TOOL` with a replacement
> hint — they are never half-advertised.

There is **no HTTP listener** in this runtime. MCP is local stdio
(`cortex mcp --agent <name>`).

## The eight operations

| Tool | Required | Optional | What it does |
|---|---|---|---|
| `cortex_capabilities` | — | — | Operations, Lens profiles, statuses, brain epochs, aliases, removed-tool map |
| `cortex_orient` | — | `task`, `thread`, `budget`, `evidence`, `observation_scope`, `observations` | Situation brief + optional attributed V5 observations |
| `cortex_query` | `need` | `profile`, `needs[]`, `thread`, `time`, `budget`, `evidence`, `paths[]`, `symbols[]`, `observation_scope`, `observations` | A View of Cards plus a separate `observations` section for V5 capture hits |
| `cortex_expand` | `alias`+`receipt` or `reference` | — | Exact source for a Card alias, `decision::N`, or V5 `obs:<source_id>` |
| `cortex_commit` | `entries[]` or `decision` | `idempotency_key`, `return_view`, `retention_class` | Atomic deposit; Receipt with durability vector; same key + same payload replays |
| `cortex_checkpoint` | `thread` | `goal`, `state`, `note`, `action` | Durable Thread checkpoint / obligations / attempts |
| `cortex_resolve` | `record`+`rationale` (or legacy `keepId`+`action`) | `considered[]`, `body` | Resolution revision over competing heads; rejected evidence kept |
| `cortex_feedback` | `outcome` | `taskClass`, `memorySources[]`, `qualityScore`, `notes` | Outcome telemetry; usefulness stays separate from truth |

Every response carries a protocol `status`: `ok`, `partial`, `no_match`,
`ambiguous`, `needs_more_budget` (+`required_plan_bytes`), `projection_pending`,
`resnapshot_required`, `unavailable`, `denied`, `outcome_unknown`,
`invalid_request`.

## Working legacy aliases (callable, not advertised)

| Legacy name | Routes to | Notes |
|---|---|---|
| `cortex_boot` | orient | Situation brief (not a separate boot compiler tool) |
| `cortex_recall` / `cortex_peek` / `cortex_semantic_recall` | query (legacy shapes) | CQR result envelopes preserved |
| `cortex_store` | commit | |
| `cortex_unfold` | expand | |
| `cortex_conflicts_resolve` | resolve | |
| `cortex_focus_start` / `cortex_focus_end` | checkpoint | |
| `cortex_agent_feedback_record` | feedback (legacy shape) | |
| `cortex_health` | specialised health payload | Not the capabilities envelope |

## Specialised legacy surfaces that still dispatch

| Tool | What it does |
|---|---|
| `cortex_health` | DB stats / memory counts |
| `cortex_digest` | Daily activity digest |
| `cortex_lastCall` | Latest memory/decision/event |
| `cortex_agent_feedback_stats` | Reliability trends |
| `cortex_permissions_list` / `_grant` / `_revoke` | Client permission ACL |

## Removed names (UNKNOWN_TOOL)

These no longer have a dispatcher. `cortex_capabilities.removed_tools` maps
each to a replacement:

`cortex_boot_audit`, `cortex_diary`, `cortex_forget`, `cortex_reconnect`,
`cortex_recall_policy_explain`, `cortex_focus_status`, `cortex_conflicts_list`,
`cortex_conflicts_get`, `cortex_consensus_promote`, `cortex_memory_decay_run`,
`cortex_eval_run`.

Calling a removed name never returns a fabricated success.

## Observation and assembly bridges

`cortex_query` / `cortex_orient` attach a separate `observations` object when
registered capture sources exist. Hits are **attributed observations**, never
CQR Cards:

```json
"observations": {
  "status": "ready",
  "scope": "project",
  "count": 1,
  "projection_pending": 0,
  "items": [{
    "source_id": "…",
    "source_key": "worklog",
    "role": "tool_report",
    "preview": "…",
    "expand": "obs:…",
    "trust": {"kind": "attributed_observation", "instruction": false, "privilege": "none", "provenance": "worklog"}
  }],
  "note": "Attributed observations, not CQR facts. Expand obs:<source_id> for exact text."
}
```

- Default scope is `project` (`observation_scope` overrides).
- Opt out with `observations: false`.
- `cortex_expand` `{"reference":"obs:<source_id>"}` returns the exact retained text with the same trust envelope.
- Capture never changes CQR admission, status, or Card epistemic state.

After `routes-rebuild`, the same calls may attach an `assemblies` section:
evidence-closed bundles with member roles, expand handles (`asm:<id>`,
`obs:` / `rev:`), and an extractive `brief`. Missing required exceptions
are `qualification_unavailable` with no prefix claim. Opt out with
`assemblies: false`. Routes off omits the section. Cards stay CQR.

## Quick reference

- **Recall budget** defaults to `200` tokens on legacy recall surfaces.
- **Conflict classes**: `AGREES`, `CONTRADICTS`, `REFINES`, `UNRELATED`.
- **Feedback outcomes**: `success`, `partial`, `failure`.
- **Permission levels**: `read`, `write`, `admin`. Default scope: `*`.
- **Progressive disclosure**: `cortex_query` (View + aliases) → `cortex_expand` (exact source).

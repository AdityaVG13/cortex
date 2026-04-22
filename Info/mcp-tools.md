<p align="center"><a href="../README.md">← Back to README</a></p>

# MCP Tool Reference

> All 28 tools exposed by the Cortex MCP server. Source of truth: `daemon-rs/src/handlers/mcp.rs`.

---

## Core

| Tool | Required | Optional | What it does |
|---|---|---|---|
| `cortex_boot` | — | `agent`, `budget`, `profile` | Session boot prompt from identity + delta capsules |
| `cortex_peek` | `query` | `limit` | Headline-only relevance check (~80% cheaper than recall) |
| `cortex_recall` | `query` | `budget`, `policyMode`, `k`, `agent`, `taskClass`, `adaptive` | Hybrid keyword + semantic memory search |
| `cortex_semantic_recall` | `query` | `budget`, `k`, `agent` | Semantic-only recall (skips keyword fusion) |
| `cortex_recall_policy_explain` | `query` | `budget`, `policyMode`, `k`, `pool_k`, `agent` | Explain ranking: why these results, in this order |
| `cortex_store` | `decision` | `context`, `type`, `source_agent`, `confidence`, `reasoning_depth` | Persist a decision with conflict detection |
| `cortex_unfold` | `sources` | — | Expand memory sources to full text (use after peek) |
| `cortex_health` | — | — | System health, DB stats, memory counts |
| `cortex_digest` | — | — | Daily summary: activity, savings, top recalls |

## Memory management

| Tool | Required | Optional | What it does |
|---|---|---|---|
| `cortex_forget` | `source` | — | Decay matching entries (score × 0.3) |
| `cortex_resolve` | `keepId`, `action` | `supersededId` | Resolve a disputed decision pair (keep or merge) |
| `cortex_memory_decay_run` | — | `includeAging`, `cleanupExpired` | Run maintenance: decay, aging, expired cleanup |
| `cortex_lastCall` | — | `kind`, `agent` | Fetch most recent memory, decision, or event |

## Conflicts

| Tool | Required | Optional | What it does |
|---|---|---|---|
| `cortex_conflicts_list` | — | `status`, `classification`, `conflictId`, `limit` | List conflicts with filters |
| `cortex_conflicts_get` | `conflictId` | — | Fetch one conflict by ID |
| `cortex_conflicts_resolve` | `action` | `winnerId`, `keepId`, `supersededId`, `loserId`, `conflictId`, `classification`, `similarity`, `notes`, `resolvedBy` | Resolve with winner + metadata |
| `cortex_consensus_promote` | — | `limit`, `minMargin`, `dryRun` | Auto-resolve when trust margin is clear |

## Focus sessions

| Tool | Required | Optional | What it does |
|---|---|---|---|
| `cortex_focus_start` | `label` | `agent` | Start focus session (context checkpoint) |
| `cortex_focus_end` | `label` | `agent` | End session, consolidate into summary |
| `cortex_focus_status` | — | `agent` | Active/recent sessions and savings |

## Agent telemetry

| Tool | Required | Optional | What it does |
|---|---|---|---|
| `cortex_agent_feedback_record` | `outcome` | `agent`, `taskClass`, `outcomeScore`, `qualityScore`, `latencyMs`, `retries`, `tokensUsed`, `memorySources`, `notes` | Record task outcome with metrics |
| `cortex_agent_feedback_stats` | — | `horizonDays`, `limit`, `taskClass`, `agent` | Reliability trends from outcomes |
| `cortex_eval_run` | — | `horizonDays` | Conflict pressure + resolution snapshot |

## Permissions

| Tool | Required | Optional | What it does |
|---|---|---|---|
| `cortex_permissions_list` | — | — | List permission grants |
| `cortex_permissions_grant` | `client`, `permission` | `scope` | Grant read / write / admin |
| `cortex_permissions_revoke` | `client`, `permission` | `scope` | Revoke a grant |

## Session

| Tool | Required | Optional | What it does |
|---|---|---|---|
| `cortex_diary` | — | `accomplished`, `nextSteps`, `decisions`, `pending`, `knownIssues` | Write session state for cross-session continuity |
| `cortex_reconnect` | — | `agent`, `model` | Re-register after daemon restart or disconnect |

---

## Quick reference

- **Recall budget** defaults to `200` tokens. Policy modes: `fast`, `balanced`, `deep`.
- **Conflict classes**: `AGREES`, `CONTRADICTS`, `REFINES`, `UNRELATED`.
- **Feedback outcomes**: `success`, `partial`, `failure`.
- **Permission levels**: `read`, `write`, `admin`. Default scope: `*`.
- **Progressive disclosure**: `peek` (headlines) → `unfold` (full text of selected items) → `recall` (full search).

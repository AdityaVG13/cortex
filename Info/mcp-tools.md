# Cortex MCP Tool Reference

Source of truth: `daemon-rs/src/handlers/mcp.rs` (`mcp_tools()`).

| Tool | Required args | Optional args | Purpose |
|---|---|---|---|
| `cortex_boot` | — | `agent`, `budget`, `profile` | Compile session boot prompt from identity + delta capsules |
| `cortex_peek` | `query` | `limit` | Headline-only relevance check before full recall |
| `cortex_recall` | `query` | `budget`, `agent` | Hybrid memory/decision search |
| `cortex_store` | `decision` | `context`, `type`, `source_agent`, `confidence` | Persist a decision or insight |
| `cortex_health` | — | — | Health/status stats |
| `cortex_digest` | — | — | Daily summary and savings stats |
| `cortex_forget` | `source` | — | Decay matching memories/decisions |
| `cortex_resolve` | `keepId`, `action` | `supersededId` | Resolve a decision conflict |
| `cortex_unfold` | `sources` | — | Expand selected memory/decision sources to full text |
| `cortex_focus_start` | `label` | `agent` | Start a focus session |
| `cortex_focus_end` | `label` | `agent` | End a focus session and summarize |
| `cortex_focus_status` | — | `agent` | Show active/recent focus sessions |
| `cortex_diary` | — | `accomplished`, `nextSteps`, `decisions`, `pending`, `knownIssues` | Write cross-session state to `state.md` |

Notes:
- `cortex_recall` defaults to budget `200` when omitted.
- `cortex_health` and `cortex_digest` include liveness metadata in MCP responses.

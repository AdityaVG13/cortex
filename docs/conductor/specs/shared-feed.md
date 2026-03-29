# Shared Feed — Inter-Agent Communication

## Purpose

Both AIs see each other's prompts, work summaries, and task completions without Aditya relaying. Token-efficient: ~3,200 tokens/session vs ~30,000 for full transcript sharing.

## Feed Entry Schema

```json
{
  "id": "uuid",
  "agent": "claude-code",
  "kind": "prompt | completion | task_complete | system",
  "priority": "normal | high",
  "content": "User asked: implement the task board",
  "summary": "Implemented Task Board: 6 endpoints, 12 tests",
  "files": ["/src/daemon.js"],
  "taskId": "uuid-or-null",
  "traceId": "uuid-or-null",
  "timestamp": "ISO-8601",
  "tokens": 42
}
```

- `kind`: what type of feed entry
- `summary`: short version always present (1-2 lines, used in boot injection)
- `content`: full version (only returned by GET /feed/:id, not in list)
- `tokens`: estimated token count of content (for budget tracking)
- `traceId`: optional — links prompt → completion as a pair

## API Endpoints

```
POST /feed
Headers: Authorization: Bearer <token>
Body: {
  "agent": "claude-code",
  "kind": "completion",
  "summary": "Implemented Task Board with 6 endpoints",
  "content": "Full detailed output...",    // optional, stored for detail-on-demand
  "files": ["/src/daemon.js"],             // optional
  "taskId": "uuid",                        // optional
  "traceId": "uuid",                       // optional
  "priority": "normal"                     // optional, default normal
}
Response: 201 {
  "feedId": "uuid",
  "recorded": true
}
```

```
GET /feed
Query params:
  - since: duration string ("5m", "1h", "1d"), default "1h"
  - agent: filter by agent name (optional)
  - kind: filter by kind (optional)
  - unread: "true" — only entries this agent hasn't acked (requires agent param)
Response: 200 {
  "entries": [
    {
      "id": "uuid",
      "agent": "claude-code",
      "kind": "completion",
      "summary": "Implemented Task Board with 6 endpoints",
      "files": ["/src/daemon.js"],
      "taskId": null,
      "traceId": null,
      "priority": "normal",
      "timestamp": "ISO-8601",
      "tokens": 42
    }
  ]
}
```

Note: GET /feed returns `summary` only, NOT `content`. This keeps list queries cheap.

```
GET /feed/:id
Response: 200 {
  "id": "uuid",
  "agent": "claude-code",
  "kind": "completion",
  "summary": "...",
  "content": "Full detailed output...",
  "files": [...],
  ...full entry
}
```

Detail-on-demand: only fetch full content when the AI actually needs it.

```
POST /feed/ack
Headers: Authorization: Bearer <token>
Body: {
  "agent": "factory-droid",
  "lastSeenId": "uuid"
}
Response: 200 {
  "acked": true
}
```

Marks all entries up to `lastSeenId` as read for this agent.

## Implementation

- In-memory array (same pattern as activities/messages)
- Max 200 entries (FIFO eviction of oldest)
- Retention TTL: 4 hours (entries older than this auto-pruned on access)
- Ack tracking: `Map<agent, lastSeenId>` — used by `?unread=true` filter
- Secret redaction: strip patterns matching Bearer tokens, API keys, hex strings >32 chars before storing

## Auto-Post on Task Complete

When `POST /tasks/complete` succeeds, the daemon automatically posts a feed entry:

```json
{
  "agent": "<from task.claimedBy>",
  "kind": "task_complete",
  "summary": "Completed: <task.title>",
  "content": "<task.summary if provided>",
  "taskId": "<task.taskId>",
  "priority": "normal"
}
```

Dedupe: check if a feed entry with same `taskId` and `kind=task_complete` already exists. Skip if so.

## Boot Injection

Extend `conductorState` to include unread feed entries for the booting agent.

In delta capsule:
```
## Feed
- [prompt] Aditya → factory-droid: "build the dashboard" (12m ago)
- [completion] factory-droid: Built Streamlit dashboard with 6 tabs (8m ago)
- [task_complete] factory-droid: Completed "Dashboard MVP" (5m ago)
```

Cap at 10 most recent unread entries. Use `summary` field only.
Auto-ack after boot injection (so next boot only shows new entries).

## Secret Redaction

Before storing any feed entry, run content and summary through:
```javascript
function redactSecrets(text) {
  return text
    .replace(/Bearer\s+[a-f0-9]{32,}/gi, 'Bearer [REDACTED]')
    .replace(/[a-f0-9]{40,}/gi, '[HASH_REDACTED]')
    .replace(/(?:token|key|secret|password)\s*[:=]\s*\S+/gi, '[CREDENTIAL_REDACTED]');
}
```

## Tests

- `POST /feed creates feed entry`
- `GET /feed returns recent entries with summary only`
- `GET /feed filters by kind`
- `GET /feed?unread=true returns only unacked entries`
- `GET /feed/:id returns full content`
- `POST /feed/ack marks entries as read`
- `Feed entries auto-expire after TTL`
- `Task completion auto-posts to feed`
- `Feed entries have secrets redacted`
- `Boot prompt injects unread feed entries`

# Task Board — Agent Work Queue

## Purpose

Agents pull tasks from a shared queue instead of Aditya relaying between terminals. User posts tasks with descriptions and file scopes. Agents claim and complete them. Unclaimed tasks surface in boot prompts.

## Task Lifecycle

```
pending → claimed → completed
              ↓
           abandoned (if agent session expires while holding task)
```

## API Endpoints

```
POST /tasks
Headers: Authorization: Bearer <token>
Body: {
  "title": "Add rate limiting to /store endpoint",
  "description": "Prevent abuse by limiting to 60 calls/min",
  "project": "cortex",
  "files": ["/src/daemon.js"],
  "priority": "high",           // low | medium | high | critical
  "requiredCapability": "node"  // optional — node | python | any
}
Response: 201 {
  "taskId": "uuid",
  "status": "pending"
}
```

```
GET /tasks
Query params:
  - status: pending | claimed | completed | all (default: pending)
  - project: filter by project name
Response: 200 {
  "tasks": [
    {
      "taskId": "uuid",
      "title": "Add rate limiting",
      "description": "...",
      "project": "cortex",
      "files": ["/src/daemon.js"],
      "priority": "high",
      "requiredCapability": "node",
      "status": "pending",
      "claimedBy": null,
      "createdAt": "ISO-8601",
      "claimedAt": null,
      "completedAt": null
    }
  ]
}
```

```
POST /tasks/claim
Headers: Authorization: Bearer <token>
Body: {
  "taskId": "uuid",
  "agent": "claude-code"
}
Response: 200 {
  "claimed": true,
  "taskId": "uuid"
}
Response: 409 {
  "error": "task_already_claimed",
  "claimedBy": "factory-droid"
}
Response: 404 {
  "error": "task_not_found"
}
```

```
POST /tasks/complete
Headers: Authorization: Bearer <token>
Body: {
  "taskId": "uuid",
  "agent": "claude-code",
  "summary": "Added rate limiter middleware, 60 req/min per token"
}
Response: 200 {
  "completed": true,
  "taskId": "uuid"
}
Response: 403 {
  "error": "not_task_holder"
}
```

```
POST /tasks/abandon
Headers: Authorization: Bearer <token>
Body: {
  "taskId": "uuid",
  "agent": "claude-code"
}
Response: 200 {
  "abandoned": true,
  "taskId": "uuid",
  "status": "pending"
}
```

```
GET /tasks/next
Query params:
  - agent: agent name (required)
  - capability: node | python | any (default: any)
Response: 200 {
  "task": { ... } or null
}
```

`GET /tasks/next` returns the highest-priority unclaimed task matching the agent's capability. Priority order: critical > high > medium > low. Within same priority, oldest first (FIFO).

## Implementation

- In-memory `Map<string, TaskObject>` keyed by taskId
- No database table for MVP (same pattern as locks/sessions)
- Tasks capped at 500 (FIFO eviction of completed tasks)
- Priority ranking: critical=4, high=3, medium=2, low=1

## Boot Injection

Extend `conductorState` to include pending/claimed tasks. In delta capsule:

```
## Pending Tasks
- [high] Add rate limiting to /store endpoint (cortex, files: /src/daemon.js)
- [medium] Write backup/restore endpoints (cortex)

## Your Active Tasks
- [high] Implement Session Bus (claimed 5m ago)
```

Show pending tasks the agent hasn't claimed (so they know what's available).
Show their own claimed tasks (so they remember what they're working on).

## Auto-Abandonment

When a session expires (heartbeat timeout), any tasks claimed by that agent return to `pending`. This prevents deadlocked tasks when an AI session crashes.

## Tests

- `POST /tasks creates a new task`
- `GET /tasks lists pending tasks`
- `GET /tasks filters by status`
- `POST /tasks/:id/claim claims a task`
- `POST /tasks/:id/claim returns 409 when already claimed`
- `POST /tasks/:id/complete marks task done`
- `POST /tasks/:id/complete only allowed by holder`
- `POST /tasks/:id/abandon returns task to pending`
- `GET /tasks/next returns highest priority unclaimed task`
- `GET /tasks/next filters by capability`
- `GET /tasks/next returns null when no tasks available`
- `Boot prompt injects pending and claimed tasks`

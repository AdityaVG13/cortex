# Phase 0: File Locking & Inter-Agent Communication

## Scope

Phase 0 implements the foundation for multi-agent coordination — file locking, activity tracking, and boot-injected awareness. This prevents the file collision incidents (like the mcp.json collision on 2026-03-28) that have already occurred between Claude Code and Factory Droid.

## Components

### 0a: File Lock Ledger

**Purpose:** Prevent concurrent writes to the same file by different AI agents.

**API Endpoints:**

```
POST /lock
Headers: Authorization: Bearer <token>
Body: {
  "path": "/path/to/file",
  "agent": "factory-droid",
  "ttl": 300  // seconds, default 300 (5 minutes)
}
Response: 200 {
  "locked": true,
  "lockId": "uuid",
  "expiresAt": "ISO-8601-timestamp"
}
Response: 409 {
  "error": "file_already_locked",
  "holder": "claude-code",
  "expiresAt": "ISO-8601-timestamp"
}
```

```
POST /unlock
Headers: Authorization: Bearer <token>
Body: {
  "path": "/path/to/file",
  "agent": "factory-droid"
}
Response: 200 {
  "unlocked": true
}
Response: 404 {
  "error": "no_lock_found"
}
```

```
GET /locks
Response: 200 {
  "locks": [
    {
      "path": "/path/to/file",
      "agent": "factory-droid",
      "lockedAt": "ISO-8601-timestamp",
      "expiresAt": "ISO-8601-timestamp"
    }
  ]
}
```

**Implementation:**
- In-memory map (Map<string, LockObject>) for MVP
- No new database table needed
- Auto-expire on access (check `expiresAt < now` and delete)
- Lock structure: `{ path, agent, lockedAt, expiresAt, id }`

**Behavior:**
- Default TTL: 300 seconds (5 minutes)
- Same agent CAN renew lock by posting again
- Different agent gets 409 with holder info
- Unlock only allowed by lock holder

### 0b: Activity Channel

**Purpose:** Real-time visibility into what all agents are doing.

**API Endpoints:**

```
POST /activity
Headers: Authorization: Bearer <token>
Body: {
  "agent": "factory-droid",
  "description": "Writing Phase 0 spec",
  "files": ["/cortex/docs/conductor/specs/phase-0.md"]
}
Response: 200 {
  "recorded": true,
  "activityId": "uuid"
}
```

```
GET /activity?since=5m
Query params:
  - since: duration string (e.g., "5m", "1h", "1d"), default "1h"
Response: 200 {
  "activities": [
    {
      "id": "uuid",
      "agent": "factory-droid",
      "description": "Writing Phase 0 spec",
      "files": ["/cortex/docs/conductor/specs/phase-0.md"],
      "timestamp": "ISO-8601-timestamp"
    }
  ]
}
```

```
POST /message
Headers: Authorization: Bearer <token>
Body: {
  "from": "factory-droid",
  "to": "claude-code",
  "message": "Don't touch auth.js, I'm fixing CORS"
}
Response: 200 {
  "sent": true,
  "messageId": "uuid"
}
```

```
GET /messages?agent=factory-droid
Query params:
  - agent: recipient agent name
Response: 200 {
  "messages": [
    {
      "id": "uuid",
      "from": "claude-code",
      "to": "factory-droid",
      "message": "Don't touch auth.js, I'm fixing CORS",
      "timestamp": "ISO-8601-timestamp"
    }
  ]
}
```

**Implementation:**
- In-memory arrays for MVP (activities[], messages[])
- Activities capped at 1000 entries (FIFO)
- Messages capped at 100 entries per agent
- Store to database table optional for Phase 1

**Behavior:**
- GET /activity returns only activities since duration
- GET /messages returns only messages for recipient
- Messages are NOT consumed (persist until read)

### 0c: Boot-Injected Awareness

**Purpose:** Inject active locks and pending messages into agent boot prompt.

**Capsule Compiler Enhancement:**

Extend `delta capsule` to include:

```javascript
// In src/compiler.js, add to delta capsule:

if (locks.size > 0) {
  delta += `\n## Active Locks\n`;
  for (const [path, lock] of locks) {
    const minutesLeft = Math.ceil((new Date(lock.expiresAt) - new Date()) / 60000);
    delta += `- ${path} locked by ${lock.agent} (${minutesLeft}m remaining)\n`;
  }
}

const myMessages = messages.filter(m => m.to === agentName);
if (myMessages.length > 0) {
  delta += `\n## Pending Messages\n`;
  for (const msg of myMessages) {
    delta += `- From ${msg.from}: "${msg.message}"\n`;
  }
}
```

**Boot Prompt Example:**

```
## Delta
Claude sent you a message: "Don't touch auth.js, I'm fixing CORS"

## Active Locks
- /cortex/src/daemon.js locked by claude-code (3m remaining)
```

## Implementation Order

1. **Write Phase 0 spec** (this file) ✓
2. **Write tests** (test/conductor.test.js)
3. **Implement in-memory lock ledger**
4. **Implement in-memory activity channel**
5. **Implement message system**
6. **Extend capsule compiler for boot injection**
7. **Run test suite**
8. **Manual testing with cross-agent coordination**

## Tests (TDD)

Write tests BEFORE implementation:

**File: test/conductor.test.js**

- `test("POST /lock acquires lock successfully")`
- `test("POST /lock returns 409 when already locked")`
- `test("POST /lock renews lock for same agent")`
- `test("POST /unlock releases lock")`
- `test("POST /unlock only allowed by lock holder")`
- `test("GET /locks lists all active locks")`
- `test("Locks auto-expire after TTL")`
- `test("POST /activity records activity")`
- `test("GET /activity returns only recent activities")`
- `test("POST /message sends message")`
- `test("GET /messages returns messages for agent")`
- `test("Capsule compiler injects locks into delta")`
- `test("Capsule compiler injects messages into delta")`

## Coordination Workflow

**Before Phase 0 (manual relay):**
1. Aditya tells Claude what to work on
2. Aditya tells Droid what to work on  
3. AI edits shared file → Aditya: "check with other AI first"

**After Phase 0 (automatic):**
1. Claude boots → sees "Droid is editing src/routes.js — avoid this file"
2. Claude checks GET /locks before writing files
3. Claude POST /lock when starting to edit a file
4. Claude POST /unlock when done editing
5. Claude POST /activity to show current work

## Success Criteria

- ✅ Two AI agents can claim locks on different files without collision
- ✅ Agent gets 409 when trying to lock a file already held by another agent
- ✅ Agent can see all active locks via GET /locks
- ✅ Agent gets informed of pending messages on boot
- ✅ Agent can see recent activity from all agents
- ✅ No database dependency (in-memory for MVP)
- ✅ Tests pass at 100%

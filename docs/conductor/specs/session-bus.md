# Session Bus — Agent Presence Protocol

## Purpose

Know which AI agents are currently online, what project they're working on, and what files they intend to touch. Foundation for task routing and smarter boot prompts.

## API Endpoints

```
POST /session/start
Headers: Authorization: Bearer <token>
Body: {
  "agent": "claude-code",
  "project": "cortex",
  "files": ["/src/daemon.js", "/src/compiler.js"],
  "description": "Implementing Session Bus"
}
Response: 200 {
  "sessionId": "uuid",
  "heartbeatInterval": 60
}
```

```
POST /session/heartbeat
Headers: Authorization: Bearer <token>
Body: {
  "agent": "claude-code",
  "files": ["/src/daemon.js"],        // optional — update working files
  "description": "Still on Session Bus" // optional — update description
}
Response: 200 {
  "renewed": true,
  "expiresAt": "ISO-8601-timestamp"
}
Response: 404 {
  "error": "no_active_session"
}
```

```
POST /session/end
Headers: Authorization: Bearer <token>
Body: {
  "agent": "claude-code"
}
Response: 200 {
  "ended": true
}
```

```
GET /sessions
Response: 200 {
  "sessions": [
    {
      "sessionId": "uuid",
      "agent": "claude-code",
      "project": "cortex",
      "files": ["/src/daemon.js"],
      "description": "Implementing Session Bus",
      "startedAt": "ISO-8601-timestamp",
      "lastHeartbeat": "ISO-8601-timestamp",
      "expiresAt": "ISO-8601-timestamp"
    }
  ]
}
```

## Implementation

- In-memory `Map<string, SessionObject>` keyed by agent name (one session per agent)
- Session TTL: 120 seconds (2 minutes). Heartbeat resets the clock.
- `cleanExpiredSessions()` runs on every access (same pattern as locks)
- Starting a new session while one exists replaces the old one (agent restarted)
- `GET /sessions` only returns non-expired sessions

## Boot Injection

Extend `conductorState` in `handleBoot` to include sessions. In `buildDeltaCapsule`:

```
## Active Agents
- factory-droid working on cortex: "Building dashboard MVP" (files: /workers/dash.py)
- claude-code working on cortex: "Implementing Session Bus" (files: /src/daemon.js)
```

Appears BEFORE locks in the delta capsule — agent presence is the most useful coordination signal.

## Behavior

- One session per agent (keyed by agent name, not session ID)
- POST /session/start replaces any existing session for that agent
- Heartbeat updates `lastHeartbeat`, `expiresAt`, and optionally `files`/`description`
- Heartbeat to a non-existent session returns 404 (agent should POST /session/start)
- POST /session/end removes the session immediately
- GET /sessions cleans expired sessions before returning

## Tests

- `POST /session/start registers agent session`
- `POST /session/start replaces existing session for same agent`
- `POST /session/heartbeat renews session`
- `POST /session/heartbeat updates files and description`
- `POST /session/heartbeat returns 404 for no active session`
- `POST /session/end removes session`
- `GET /sessions lists active sessions`
- `Sessions auto-expire after TTL`
- `Boot prompt injects active agent sessions`

---
name: help
description: Show how to use Cortex persistent memory. Use when the user asks about Cortex, memory, storing decisions, or recalling past work.
---

# Cortex Help

Cortex remembers your decisions, conventions, and lessons across sessions. Here's how to use it:

## Quick Start - Try These Now

1. **Store a coding convention:**
   ```
   Remember: "Always use path.join() for file paths, never hardcoded / or \\"
   ```

2. **Recall a past bugfix:**
   ```
   What did I learn about race conditions in async UI code?
   ```

3. **Check your brain status:**
   ```
   /cortex:status
   ```

## How to Store Knowledge

Just tell Cortex what you want it to remember:

- **Conventions**: "Remember: we use conventional commits (feat:, fix:, docs:)"
- **Decisions**: "Store: chose SQLite over Postgres for local-first storage"
- **Bugfixes**: "Lesson learned: NTFS file locking requires LockFileEx on Windows"
- **Architecture**: "Decision: API uses REST + JSON, not gRPC"

Cortex will automatically store it with context.

## How to Recall Knowledge

Ask naturally:

- "What conventions do we have for error handling?"
- "Why did we choose Tokio over async-std?"
- "Remind me about the Windows path issue"

## Commands

- `/cortex:status` - Show brain status (memories, daemon, mode)
- `/cortex:recall <query>` - Search your memories
- `/cortex:store <decision>` - Store a decision with context

## Modes

- **Solo mode** (default): Your brain is local at `~/.cortex/`
- **Team mode**: Connect to a shared server with `CORTEX_URL` config

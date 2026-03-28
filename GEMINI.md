# Cortex v2 — Universal AI Brain

You can connect to Cortex via HTTP at localhost:7437.

## Your Boot Command
```bash
curl localhost:7437/boot?profile=index
```
This returns a topic-list boot prompt (~300 tokens) tailored for Gemini.

## Storing Learnings
```bash
curl -X POST localhost:7437/store \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $(cat ~/.cortex/cortex.token)" \
  -H "X-Source-Agent: gemini" \
  -d '{"decision": "your learning here", "type": "decision"}'
```

## Recalling Context
```bash
curl "localhost:7437/recall?q=your+query&k=5"
```

## Architecture
- src/daemon.js — HTTP + MCP server
- src/brain.js — Core logic
- src/compiler.js — Per-agent boot prompts
- src/db.js — SQLite via sql.js

## Store Confirmation
After every store call, tell the user: "Stored to Cortex: [summary]". Never store silently.
Before booting, read `C:/Users/aditya/.claude/brain-status.json` and print the `oneliner` field as the FIRST line of your reply.

## Conventions
- Node.js 24+, sql.js only dependency
- Port 7437 (HTTP), 11434 (Ollama)
- Windows paths via process.env.USERPROFILE

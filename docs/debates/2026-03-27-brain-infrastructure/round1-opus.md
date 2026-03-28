# Round 1 — Claude Opus (Architect)

## Architecture

**1. Merge or separate?** MERGE. Three MCP servers is three failure modes. Build a single "Cortex" server in Node.js that subsumes brain-server's indexing + recall AND OMEGA's store/query/coordination. One process, one health check, one registration. The Python OMEGA package has Windows compat issues that keep recurring — rewriting the coordination layer in Node eliminates that class of bug entirely. Keep claude-mem as a read-only external archive (it's a third-party plugin, don't couple to it).

**2. OMEGA on every prompt?** NO. Too expensive and too slow. Instead: SessionStart hook loads full context briefing (one call). UserPromptSubmit hook does a lightweight intent check ONLY when the prompt mentions coordination keywords ("dispatch", "parallel", "check with"). PostToolUse hook on Agent calls handles subagent inbox/checkout. This gives you coordination without per-prompt tax.

**3. Startup sequence?** Sequential with circuit breaker:
1. brain-check.js pings Cortex HTTP → if down, attempt `node cortex.js serve &` auto-start
2. Cortex MCP responds to `initialize` → tools registered
3. SessionStart hook calls `cortex_briefing()` → compressed state loaded
4. If ANY step fails: hook outputs `BRAIN OFFLINE — [reason]` and Claude falls back to reading state.md + MEMORY.md directly. Never silent failure.

**4. Who owns what?** Cortex (merged brain+OMEGA): owns persistent memory, coordination, semantic recall, state compression. Auto-Memory: owns lightweight per-conversation notes (it's built-in, don't fight it). Claude-Mem: optional archive for cross-project search. State.md: DEPRECATED — replaced by Cortex's auto-generated state snapshot.

## MemStack Integration

**5. Private ultra repo:** Echo's anti-rationalization table pattern (not the whole skill — the PATTERN), Work's Silent Context Compilation (Step 0), Diary's structured Session Handoff format. Port the concepts, not the code.

**6. Public superpowers fork:** Governor (tier governance is universally useful), Familiar (parallel dispatch complements our existing dispatching-parallel-agents skill). Both are clean, self-contained.

**7. Redundant:** State skill (we have state.md + Cortex will auto-generate), Grimoire (we already manage CLAUDE.md via claude-md-creator), Compress (headroom proxy is a different approach than our token-efficiency skill), Forge (we have writing-skills), Humanize (we have humanizer-aditya).

**8. Anti-rationalization + Silent Context as standard?** YES. Anti-rationalization tables are the single best idea in memstack. They catch the exact failure mode we hit today — Claude rationalizing "I already know this" when it doesn't. Every skill that involves search, recall, or verification should have one. Silent Context Compilation should be the first step of every process skill.

**9. Diary Handoff vs state.md?** Diary Handoff is better in structure, but state.md is better in simplicity. The answer: Cortex auto-generates a Diary-structured handoff INTO state.md on every session end. Best of both.

## Self-Improvement

**10. Self-healing:** Three layers: (a) SessionStart hook verifies all MCPs and auto-restarts dead ones, (b) Cortex self-pings every 60s via setInterval, (c) Claude's first tool call in any session MUST be `cortex_briefing()` — if it fails, the error message includes auto-recovery instructions.

**11. Vector embeddings?** YES but lightweight. Use nomic-embed-text (already installed in ollama) for local embeddings. No external API dependency. Index lessons, diary entries, and memory files. Brain-server's current keyword matching misses semantic connections.

**12. Path to JARVIS?** Three phases: (a) Cortex v1 — merge brain+OMEGA, add embeddings, self-healing startup. (b) Cortex v2 — auto-diary on session end, auto-state on session start, learning loop where each session's lessons improve next session's briefing. (c) Cortex v3 — cross-project knowledge transfer, predictive context loading (anticipate what user needs before they ask), and the ultimate goal: Claude starts sessions already knowing what to do next without being told.

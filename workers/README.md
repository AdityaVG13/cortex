# Cortex Workers (Python)

Python processes that add intelligence on top of the Node.js daemon.
All workers talk to Cortex via HTTP at `localhost:7437`.

## Workers

| Worker | Purpose | Status |
|--------|---------|--------|
| `cortex-dream` | Nightly compaction, dedup, synthesis via local LLM | Planned |
| `cortex-dash` | Streamlit dashboard / JARVIS visualizer | Planned |
| `cortex-embed` | Batch re-embedding pipeline | Planned |
| `cortex-capture` | Ambient knowledge capture from hooks | Planned |

## Setup

```bash
cd workers
uv init
uv add httpx ollama streamlit
```

## Architecture

Workers are independent. They read/write through the same HTTP API any AI uses.
No shared state with the daemon. No imports from `src/`.

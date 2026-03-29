# Cortex Dashboard

A real-time web dashboard for monitoring and controlling the Cortex AI brain.

## Features

- **📊 Health Monitoring** — Real-time stats on memories, decisions, embeddings, and token savings
- **👥 Agent Presence** — See which AI agents are online, what projects they're working on, and when they'll expire
- **🔒 Active Locks** — Monitor file locks held by agents, see who owns what and when locks expire
- **📜 Activity Feed** — Recent actions across all agents (file changes, decisions, completions)
- **🧠 Memory Explorer** — Semantic search across all Cortex memories and decisions
- **⚡ Quick Actions** — One-click operations for common tasks

## Installation

```bash
# Install dependencies
pip install streamlit httpx
```

## Usage

```bash
# Start the dashboard on port 3333
streamlit run workers/cortex_dash.py --server.port 3333
```

Then open: `http://localhost:3333`

## Tabs

### 📊 Dashboard
- Cortex health metrics
- Ollama connectivity status
- Token savings summary
- Task board (pending until Task Board endpoints exist)

### 👥 Agents & Locks
- Active agent sessions with project/context
- File locks with ownership and expiration
- Time remaining until locks/sessions expire

### 📜 Activity
- Recent activity from all agents (last hour)
- Agent, description, files touched, timestamp
- Searchable feed for tracking work progress

### 🧠 Memory Explorer
- Semantic search across memories and decisions
- Search by keyword (e.g., "authentication", "python")
- Shows relevance score, source, and excerpt

### ⚡ Actions
- Quick stats refresh
- Placeholder for future automation (clean entries, run dream)

## Data Sources

All data comes from the Cortex daemon at `http://localhost:7437`:

| Endpoint | Used For | Auth |
|----------|----------|------|
| `/health` | Stats, Ollama status | No |
| `/sessions` | Active agent sessions | Yes |
| `/locks` | Active file locks | Yes |
| `/activity` | Recent activity feed | Yes |
| `/recall` | Memory search | No |
| `/digest` | Token savings, daily stats | No |

## Requirements

- Cortex daemon running on `localhost:7437`
- Auth token at `~/.cortex/cortex.token` (for protected endpoints)
- Python 3.10+
- `cortex_client.py` in same directory (or parent/workers)

## Customization

### Change refresh interval

Edit `workers/cortex_dash.py`:
```python
refresh = st.number_input("Auto-refresh (seconds)", min_value=0, max_value=60, value=30)
```

### Add new tab

Add to `main()`:
```python
tab6 = st.tabs([...], ["New Tab"])

with tab6:
    st.subheader("Your Content")
    # Your code here
```

### Add new endpoint to cortex_client.py

```python
def your_new_endpoint() -> dict:
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    req = Request(f"{BASE_URL}/your-endpoint")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())
```

## Troubleshooting

**Dashboard won't load:**
- Check: `curl http://localhost:7437/health`
- Ensure Cortex daemon is running

**"Failed to fetch stats":**
- Verify daemon is responding: `curl http://localhost:7437/health`
- Check the logs at `~/.cortex/cortex.log`

**Agent/session data missing:**
- Auth token might be missing: `cat ~/.cortex/cortex.token`
- Token might have expired - restart daemon to regenerate

## Future Enhancements

- [ ] Real-time WebSocket updates (SSE)
- [ ] Task Board integration
- [ ] Interactive lock release
- [ ] Memory editing/deletion from dashboard
- [ ] Decision resolution UI
- [ ] Export stats to CSV
- [ ] Configurable time ranges for activity feed
- [ ] Dark theme optimization

## Support

For issues or questions:
1. Check daemon logs: `~/.cortex/cortex.log`
2. Verify endpoints: curl each endpoint individually
3. Check Cortex TODO and ROADMAP for known issues

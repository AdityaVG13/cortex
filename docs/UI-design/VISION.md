# Cortex Control Center — UI/UX Vision

**Goal**: When someone opens Cortex, the first reaction should be "wait, this is free?"
This is not a developer tool — it's a **control center for intelligence**.

## Design Philosophy

- **Apple-level polish** — every pixel intentional, every animation purposeful
- **Information density done beautifully** — show everything, overwhelm nothing
- **Living system** — the UI should feel alive, breathing, thinking
- **Dark mode first** — this is a brain, it lives in the dark
- **Real-time everything** — memories forming, agents connecting, knowledge flowing

## Core Visual Experiences

### 1. Brain Visualizer (Hero Feature)
Inspired by Jarvis/Friday from Marvel's Iron Man/Avengers.
- Real-time node graph showing memories forming and connecting
- Nodes glow when accessed, pulse when created, fade with decay
- Agent-colored edges show who stored what (Claude = blue, Droid = amber, etc.)
- Zoom from galaxy view (all knowledge) → cluster view (topics) → node view (single memory)
- Connections animate when recall finds related memories
- Conflict nodes show tension lines between contradicting memories

### 2. Agent Presence Dashboard
- Live status rings for each connected AI (heartbeat animation)
- Activity streams flowing in real-time (SSE-powered)
- Session timeline showing who's working on what
- Lock visualization — which agent holds which resource

### 3. Memory Explorer
- Card-based memory browser with preview on hover
- Semantic clustering visualization (memories grouped by topic)
- Decay timeline — watch memories age, pin important ones
- Search with highlighted semantic matches

### 4. Command Center
- Task board with Kanban-style columns
- Inter-agent message feed (chat-like)
- System health gauges (daemon, Ollama, embeddings)
- One-click actions (store, recall, forget, resolve conflicts)

## Design Inspirations

| Reference | What to Study | Apply How |
|-----------|---------------|-----------|
| **Arc Browser** | Blur effects, smooth animations, information density | Overall app shell, sidebar navigation |
| **Linear** | Typography, subtle gradients, keyboard-first | Task board, memory cards, transitions |
| **Raycast** | Command palette, dense info display | Quick search, command bar |
| **Windows 11 Mica/Acrylic** | Native translucency, depth layers | Window chrome, panel backgrounds |
| **Apple Activity Monitor** | Clean data visualization | Health gauges, system metrics |
| **Jarvis/Friday (Marvel)** | Holographic UI, node networks, real-time data flow | Brain visualizer, the hero experience |

## Technical Approach

- Tauri native window (WebView2 on Windows)
- Canvas/WebGL for brain visualizer (three.js or d3-force-3d)
- CSS animations for micro-interactions
- SSE for real-time updates from daemon
- Dark theme with accent colors per agent identity

## Design Phases

1. **Foundation** — App shell, navigation, dark theme, typography
2. **Dashboard** — Agent presence, health, activity feed
3. **Memory Explorer** — Card browser, search, semantic clusters
4. **Brain Visualizer** — The hero feature, node graph, real-time memory formation
5. **Polish** — Animations, transitions, micro-interactions, icons, empty states

## Non-Negotiables
- Must look premium, not like a hackathon project
- Must work at 60fps even with 1000+ memory nodes
- Must be responsive (works on laptop and external monitor)
- No loading spinners — progressive rendering, skeleton states
- Every interaction should have satisfying visual feedback

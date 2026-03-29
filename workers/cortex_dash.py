"""Cortex Dashboard — Streamlit web UI for Cortex monitoring and control.

Run: streamlit run workers/cortex_dash.py --server.port 3333
"""

import asyncio
import json
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

try:
    import streamlit as st
    import httpx
except ImportError as e:
    print("Missing dependencies: install with: pip install streamlit httpx")
    sys.exit(1)

# Allow importing cortex_client from same directory
sys.path.insert(0, str(Path(__file__).parent.parent / "workers"))
import cortex_client

BASE_URL = "http://localhost:7437"


def init_page_config():
    """Configure Streamlit page settings."""
    st.set_page_config(
        page_title="Cortex Dashboard",
        page_icon="🧠",
        layout="wide",
        initial_sidebar_state="expanded",
    )


def format_timestamp(iso_str: str) -> str:
    """Format ISO timestamp to readable format."""
    if not iso_str:
        return "Never"
    try:
        dt = datetime.fromisoformat(iso_str.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        now = datetime.now(timezone.utc)
        delta = now - dt

        if delta.days > 0:
            return f"{dt.strftime('%Y-%m-%d %H:%M')} ({delta.days}d ago)"
        elif delta.seconds > 3600:
            hours = delta.seconds // 3600
            return f"{dt.strftime('%Y-%m-%d %H:%M')} ({hours}h ago)"
        elif delta.seconds > 60:
            mins = delta.seconds // 60
            return f"{dt.strftime('%Y-%m-%d %H:%M')} ({mins}m ago)"
        else:
            return f"{dt.strftime('%Y-%m-%d %H:%M')} (just now)"
    except Exception:
        return iso_str[:16]


def format_duration(secs: float) -> str:
    """Format seconds to readable duration."""
    if secs < 60:
        return f"{int(secs)}s"
    elif secs < 3600:
        return f"{int(secs // 60)}m"
    elif secs < 86400:
        return f"{int(secs // 3600)}h"
    else:
        return f"{int(secs // 86400)}d"


@dataclass
class CortexStats:
    """Statistics from Cortex health endpoint."""
    status: str
    memories: int
    decisions: int
    embeddings: int
    events: int
    ollama: str


def get_cortex_stats() -> CortexStats | None:
    """Fetch and parse Cortex stats."""
    try:
        data = cortex_client.health()
        s = data.get("stats", {})
        return CortexStats(
            status=data.get("status", "unknown"),
            memories=s.get("memories", 0),
            decisions=s.get("decisions", 0),
            embeddings=s.get("embeddings", 0),
            events=s.get("events", 0),
            ollama=s.get("ollama", "unknown"),
        )
    except Exception as e:
        st.error(f"Failed to fetch stats: {e}")
        return None


def ollama_status_badge(ollama: str) -> str:
    """Return emoji badge for Ollama status."""
    if ollama == "connected":
        return "🟢"
    elif ollama.startswith("error_"):
        return "🟡"
    else:
        return "🔴"


def render_health_card(stats: CortexStats):
    """Render the Cortex health card."""
    with st.expander("📊 Cortex Health", expanded=True):
        col1, col2, col3, col4 = st.columns(4)
        
        with col1:
            st.metric("Memories", f"{stats.memories:,}")
        
        with col2:
            st.metric("Decisions", f"{stats.decisions:,}")
        
        with col3:
            st.metric("Embeddings", f"{stats.embeddings:,}")
        
        with col4:
            st.metric("Ollama", f"{ollama_status_badge(stats.ollama)} {stats.ollama}")
        
        # Show token savings if available
        try:
            digest = cortex_client.digest()
            ts = digest.get("tokenSavings", {}).get("allTime", {})
            saved = ts.get("saved", 0)
            if saved > 0:
                savings_percent = ts.get("percent", 0)
                st.markdown(f"💰 **Token Savings:** {saved:,} tokens ({savings_percent}%) across {ts.get('boots', 0)} boots")
            
            # Show today's activity
            today = digest.get("today", {})
            if today.get("newMemories", 0) > 0 or today.get("newDecisions", 0) > 0:
                st.markdown(f"📈 **Today:** +{today['newMemories']} memories, +{today['newDecisions']} decisions")
        except Exception:
            pass


def render_agent_presence(sessions_data: dict | None):
    """Render the agent presence display."""
    st.subheader("👥 Agent Presence")
    
    if not sessions_data:
        st.info("No active sessions")
        return
    
    sessions = sessions_data.get("sessions", [])
    
    if not sessions:
        st.info("No active agents currently")
        return
    
    for session in sessions:
        agent = session.get("agent", "unknown")
        project = session.get("project") or "unknown"
        desc = session.get("description") or "Working"
        files = session.get("files", []) or []
        expires_at = session.get("expiresAt", "")
        last_heartbeat = session.get("lastHeartbeart", session.get("lastHeartbeat", ""))
        
        time_left = ""
        if expires_at:
            try:
                dt = datetime.fromisoformat(expires_at.replace("Z", "+00:00"))
                if dt.tzinfo is None:
                    dt = dt.replace(tzinfo=timezone.utc)
                now = datetime.now(timezone.utc)
                delta = dt - now
                if delta.total_seconds() > 0:
                    time_left = f" (expires in {format_duration(delta.total_seconds())})"
            except Exception:
                pass
        
        with st.container():
            cols = st.columns([3, 2, 1])
            with cols[0]:
                st.markdown(f"**{agent}** on `{project}`")
                st.caption(desc)
            
            with cols[1]:
                if files:
                    st.text(", ".join(files[:3]) + ("..." if len(files) > 3 else ""))
            
            with cols[2]:
                if time_left:
                    st.caption(time_left)
            st.divider()


def render_active_locks(locks_data: dict | None):
    """Render the active locks display."""
    st.subheader("🔒 Active Locks")
    
    if not locks_data:
        st.info("No lock data available")
        return
    
    locks = locks_data.get("locks", [])
    
    if not locks:
        st.success("🎉 No active locks — all resources free!")
        return
    
    for lock in locks:
        path = lock.get("path", "unknown")
        agent = lock.get("agent", "unknown")
        expires_at = lock.get("expiresAt", "")
        locked_at = lock.get("lockedAt", "")
        
        time_left = ""
        if expires_at:
            try:
                dt = datetime.fromisoformat(expires_at.replace("Z", "+00:00"))
                if dt.tzinfo is None:
                    dt = dt.replace(tzinfo=timezone.utc)
                now = datetime.now(timezone.utc)
                delta = dt - now
                if delta.total_seconds() > 0:
                    mins_left = int(delta.total_seconds() / 60)
                    time_left = f"📅 Expires in {mins_left} min"
                else:
                    time_left = "⚠️ Expired"
            except Exception:
                time_left = "⚠️ Unknown"
        
        with st.container():
            col1, col2, col3 = st.columns([4, 2, 2])
            with col1:
                st.code(path, language="text")
            
            with col2:
                st.markdown(f"👤 `{agent}`")
                if locked_at:
                    st.caption(f"Locked {format_timestamp(locked_at)}")
            
            with col3:
                if time_left:
                    st.warning(time_left)
            st.divider()


def render_activity_feed(activity_data: dict | None):
    """Render the recent activity feed."""
    st.subheader("📜 Recent Activity")
    
    if not activity_data:
        st.info("No activity data available")
        return
    
    activities = activity_data.get("activities", [])
    
    if not activities:
        st.info("No recent activity")
        return
    
    for activity in reversed(activities[-10:]):  # Show last 10, newest first
        agent = activity.get("agent", "unknown")
        description = activity.get("description", "No description")
        files = activity.get("files", []) or []
        timestamp = activity.get("timestamp", "")
        
        with st.container():
            col1, col2 = st.columns([1, 5])
            with col1:
                st.markdown(f"**{agent}**")
                st.caption(format_timestamp(timestamp))
            
            with col2:
                st.text(description)
                if files:
                    st.caption("📁 " + ", ".join(files[:5]) + ("..." if len(files) > 5 else ""))
            st.divider()


def render_task_board():
    """Render the task board with real data from the daemon."""
    st.subheader("📋 Task Board")
    
    try:
        all_tasks = cortex_client.get_tasks(status="all")
        tasks = all_tasks.get("tasks", [])
    except Exception as e:
        st.error(f"Failed to fetch tasks: {e}")
        return
    
    pending = [t for t in tasks if t.get("status") == "pending"]
    claimed = [t for t in tasks if t.get("status") == "claimed"]
    completed = [t for t in tasks if t.get("status") == "completed"]
    
    t_col1, t_col2, t_col3 = st.columns(3)
    
    def priority_badge(p: str) -> str:
        return {"critical": "🔴", "high": "🟠", "medium": "🟡", "low": "🟢"}.get(p, "⚪")
    
    with t_col1:
        st.markdown("### 📅 Pending")
        if not pending:
            st.caption("No pending tasks")
        else:
            for task in pending:
                p = task.get("priority", "medium")
                title = task.get("title", "Untitled")
                project = task.get("project", "")
                capability = task.get("requiredCapability", "any")
                with st.container():
                    st.markdown(f"{priority_badge(p)} **{title}**")
                    st.caption(f"Project: {project} | Capability: {capability}")
    
    with t_col2:
        st.markdown("### ⚡ In Progress")
        if not claimed:
            st.caption("No tasks in progress")
        else:
            for task in claimed:
                p = task.get("priority", "medium")
                title = task.get("title", "Untitled")
                agent = task.get("claimedBy", "unknown")
                claimed_at = task.get("claimedAt", "")
                with st.container():
                    st.markdown(f"{priority_badge(p)} **{title}**")
                    st.caption(f"👤 {agent} | Claimed {format_timestamp(claimed_at)}")
    
    with t_col3:
        st.markdown("### ✅ Completed")
        if not completed:
            st.caption("No completed tasks")
        else:
            for task in completed[-5:]:  # Show last 5
                title = task.get("title", "Untitled")
                agent = task.get("completedBy", task.get("claimedBy", "unknown"))
                summary = task.get("summary", "")
                with st.container():
                    st.markdown(f"✅ **{title}**")
                    st.caption(f"by {agent}")
                    if summary:
                        st.text(summary[:80] + ("..." if len(summary) > 80 else ""))


def render_memory_explorer():
    """Render the memory explorer section."""
    st.subheader("🧠 Memory Explorer")
    
    query = st.text_input("Search memories and decisions", placeholder="e.g., authentication, python, windows...")
    
    if st.button("🔍 Search") and query:
        try:
            results = cortex_client.recall(query, k=10)
            
            if not results:
                st.info("No results found")
                return
            
            for i, result in enumerate(results[:10], 1):
                source = result.get("source", "unknown")
                relevance = result.get("relevance", 0)
                excerpt = result.get("excerpt", "")
                method = result.get("method", "unknown")
                
                with st.expander(f"[{method}] {source} ({relevance:.2%})", expanded=(i <= 2)):
                    st.text(excerpt)
                    st.caption(f"Relevance: {relevance:.2%} | Method: {method}")
        except Exception as e:
            st.error(f"Search failed: {e}")


def render_quick_actions():
    """Render quick action buttons."""
    st.subheader("⚡ Quick Actions")
    
    c1, c2, c3 = st.columns(3)
    
    with c1:
        if st.button("📊 Refresh Stats"):
            st.rerun()
    
    with c2:
        if st.button("🗑️ Clean Old Entries"):
            st.info("Feature coming soon!")
    
    with c3:
        if st.button("🔄 Run Cortex Dream"):
            st.info("Feature coming soon!")


def render_messages_tab():
    """Render inter-agent messaging interface."""
    st.subheader("💬 Agent Messages")
    
    # Get all messages for display - we need to check messages for known agents
    st.markdown("### Inbox")
    
    # Agent selector to view messages
    agent_to_check = st.text_input("Check messages for agent:", value="droid", key="msg_agent")
    
    if st.button("📬 Check Inbox", key="check_inbox"):
        try:
            result = cortex_client.get_messages(agent_to_check)
            messages = result.get("messages", [])
            
            if not messages:
                st.info(f"No messages for {agent_to_check}")
            else:
                for msg in messages:
                    from_agent = msg.get("from", "unknown")
                    message = msg.get("message", "")
                    timestamp = msg.get("timestamp", "")
                    
                    with st.container():
                        col1, col2 = st.columns([1, 4])
                        with col1:
                            st.markdown(f"**From: {from_agent}**")
                            st.caption(format_timestamp(timestamp))
                        with col2:
                            st.info(message)
                        st.divider()
        except Exception as e:
            st.error(f"Failed to fetch messages: {e}")
    
    st.markdown("---")
    st.markdown("### Send Message")
    
    col_from, col_to = st.columns(2)
    with col_from:
        from_agent = st.text_input("From (your agent name):", value="droid", key="send_from")
    with col_to:
        to_agent = st.text_input("To (recipient):", value="claude", key="send_to")
    
    message_text = st.text_area("Message:", placeholder="e.g., Don't touch auth.js, I'm fixing CORS", key="msg_text")
    
    if st.button("📨 Send Message", key="send_msg"):
        if message_text.strip():
            try:
                result = cortex_client.send_message(from_agent, to_agent, message_text)
                if result.get("sent"):
                    st.success(f"Message sent to {to_agent}!")
                    st.rerun()
                else:
                    st.error("Failed to send message")
            except Exception as e:
                st.error(f"Failed to send: {e}")
        else:
            st.warning("Please enter a message")


def render_feed_tab():
    """Render shared inter-agent feed."""
    st.subheader("📰 Shared Feed")

    c1, c2, c3, c4 = st.columns([1, 1, 2, 1])
    with c1:
        since = st.selectbox(
            "Since",
            options=["15m", "1h", "4h", "1d"],
            index=1,
            key="feed_since",
        )
    with c2:
        kind = st.selectbox(
            "Kind",
            options=["all", "prompt", "completion", "task_complete", "system"],
            index=0,
            key="feed_kind",
        )
    with c3:
        agent = st.text_input(
            "Agent (optional)",
            placeholder="factory-droid",
            key="feed_agent",
        ).strip()
    with c4:
        unread_only = st.checkbox("Unread only", value=False, key="feed_unread")

    if unread_only and not agent:
        st.warning("Unread filter requires an agent. Showing all entries.")
    unread_filter = unread_only if agent else None

    try:
        result = cortex_client.get_feed(
            since=since,
            agent=agent or None,
            kind=kind,
            unread=unread_filter,
        )
    except Exception as e:
        if "404" in str(e):
            st.info("Feed endpoint not available yet.")
            return
        st.error(f"Failed to fetch feed: {e}")
        return

    entries = result.get("entries", [])
    if not entries:
        st.info("No feed entries found")
        return

    kind_icon = {
        "prompt": "📝",
        "completion": "✅",
        "task_complete": "🎯",
        "system": "⚙️",
    }

    for entry in reversed(entries[-30:]):
        entry_kind = entry.get("kind", "system")
        icon = kind_icon.get(entry_kind, "•")
        entry_agent = entry.get("agent", "unknown")
        timestamp = format_timestamp(entry.get("timestamp", ""))
        summary = entry.get("summary", "(no summary)")
        priority = entry.get("priority", "normal")
        files = entry.get("files", []) or []
        task_id = entry.get("taskId")
        trace_id = entry.get("traceId")
        tokens = entry.get("tokens")

        st.markdown(f"{icon} **[{entry_kind}]** `{entry_agent}` — {summary}")
        st.caption(f"{timestamp} | priority: {priority}" + (f" | tokens: {tokens}" if tokens is not None else ""))
        if files:
            st.caption("📁 " + ", ".join(files[:6]) + ("..." if len(files) > 6 else ""))
        if task_id:
            st.caption(f"taskId: `{task_id}`")
        if trace_id:
            st.caption(f"traceId: `{trace_id}`")
        st.divider()


def main():
    """Main dashboard application."""
    init_page_config()
    
    st.title("🧠 Cortex Dashboard")
    st.caption("Real-time monitoring and control for the multi-AI brain")
    
    # Auto-refresh control
    with st.sidebar:
        st.header("⚙️ Settings")
        refresh = st.number_input("Auto-refresh (seconds)", min_value=0, max_value=60, value=5)
        
        if refresh > 0:
            st.caption(f"🙅 Auto-refresh disabled until Streamlit allows")
            # Note: Streamlit doesn't support auto-refresh from within the app yet
            # Users need to use browser refresh or an external auto-reload extension
    
    # Main content - use tabs for organization
    tab1, tab2, tab3, tab4, tab5, tab6, tab7 = st.tabs(
        ["📊 Dashboard", "👥 Agents & Locks", "📜 Activity", "📋 Task Board", "💬 Messages", "📰 Feed", "⚡ Actions"]
    )
    
    with tab1:
        # Core dashboard tab
        stats = get_cortex_stats()
        if stats:
            render_health_card(stats)
    
    with tab2:
        # Agents and locks tab
        st.info("Fetching agent data...")
        try:
            sessions = cortex_client.get_sessions()
            locks = cortex_client.get_locks()
            render_agent_presence(sessions)
            render_active_locks(locks)
        except Exception as e:
            st.error(f"Failed to fetch agent data: {e}")
    
    with tab3:
        # Activity feed tab
        st.info("Fetching activity data...")
        try:
            activity = cortex_client.get_activity(since="1h")
            render_activity_feed(activity)
        except Exception as e:
            st.error(f"Failed to fetch activity data: {e}")
    
    with tab4:
        # Task Board tab
        render_task_board()
    
    with tab5:
        # Inter-agent messaging
        render_messages_tab()
    
    with tab6:
        # Shared Feed tab
        render_feed_tab()

    with tab7:
        # Quick actions
        render_quick_actions()
        st.markdown("---")
        st.subheader("📖 Cortex Documentation")
        st.markdown("""
        - [README](../../README.md) — Overview and quick start
        - [CONNECTING](../../CONNECTING.md) — API documentation
        - [ROADMAP](../../ROADMAP.md) — Planned features
        - [TODO](../../TODO.md) — Current work items
        """)
    
    # Footer
    st.markdown("---")
    st.caption("Cortex Dashboard v1.0 | Powered by Streamlit")


if __name__ == "__main__":
    main()

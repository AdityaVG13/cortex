"""ChatGPT Conversation Ingestion Adapter for Cortex.

Parses a ChatGPT data export (conversations.json), filters by user identity,
extracts meaningful memories/decisions, deduplicates against existing Cortex
entries, and stores via the Cortex HTTP API.

Usage:
    uv run python tools/ingest_chatgpt.py <conversations.json> [--dry-run] [--user-filter KEYWORD]

The ChatGPT export format is an array of conversation objects, each with a
'mapping' dict of message nodes forming a tree. We walk the tree to extract
user messages and assistant responses in order.

Filtering: If --user-filter is provided, only conversations where the user
messages contain the keyword are included. This helps separate your
conversations from shared-account usage.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from collections import Counter

import urllib.request
import urllib.error


# ─── Configuration ───────────────────────────────────────────────────────────

CORTEX_URL = "http://127.0.0.1:7437"
CORTEX_TOKEN_PATH = Path.home() / ".cortex" / "cortex.token"

# Minimum message length to consider for extraction
MIN_MESSAGE_LEN = 30

# Signal words that indicate a message contains a decision or preference
DECISION_SIGNALS = [
    "always", "never", "prefer", "instead", "switch", "use", "avoid",
    "decided", "going with", "chose", "better to", "from now on",
    "don't use", "stop using", "migrate", "replace", "upgrade",
]

PREFERENCE_SIGNALS = [
    "i like", "i prefer", "i want", "i need", "my style", "my approach",
    "i usually", "i tend to", "my workflow", "i always", "for me",
]

FACT_SIGNALS = [
    "i work", "my job", "i'm a", "my team", "our project", "we use",
    "my stack", "i specialize", "my background", "i studied",
    "my company", "our codebase",
]

# Topics that indicate technical/development conversations (likely Aditya)
TECH_FINGERPRINT = [
    "python", "rust", "javascript", "typescript", "react", "api",
    "database", "git", "docker", "deploy", "server", "code",
    "function", "class", "module", "import", "install", "npm",
    "pip", "uv", "cortex", "claude", "ai", "model", "llm",
    "embedding", "vector", "neural", "training", "prompt",
    "algorithm", "data structure", "architecture", "backend",
    "frontend", "css", "html", "sql", "query", "debug",
]


# ─── Types ───────────────────────────────────────────────────────────────────

@dataclass
class ExtractedMemory:
    text: str
    memory_type: str  # decision, preference, fact, context
    confidence: float
    source_conversation: str
    timestamp: float
    tags: list[str] = field(default_factory=list)


@dataclass
class ConversationStats:
    total_conversations: int = 0
    filtered_in: int = 0
    filtered_out: int = 0
    messages_processed: int = 0
    memories_extracted: int = 0
    duplicates_skipped: int = 0
    stored_to_cortex: int = 0


# ─── Cortex API ──────────────────────────────────────────────────────────────

def get_cortex_token() -> str | None:
    try:
        return CORTEX_TOKEN_PATH.read_text().strip()
    except FileNotFoundError:
        return None


def cortex_store(token: str, decision: str, context: str | None = None,
                 entry_type: str = "memory", confidence: float = 0.7,
                 source_agent: str = "chatgpt-import") -> dict | None:
    payload = json.dumps({
        "decision": decision,
        "context": context,
        "type": entry_type,
        "source_agent": source_agent,
        "confidence": confidence,
    }).encode()

    req = urllib.request.Request(
        f"{CORTEX_URL}/store",
        data=payload,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        method="POST",
    )

    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            return json.loads(resp.read())
    except urllib.error.URLError as e:
        print(f"  [ERROR] Store failed: {e}", file=sys.stderr)
        return None


def cortex_peek(token: str, query: str, limit: int = 3) -> list[dict]:
    """Check if a similar memory already exists in Cortex."""
    url = f"{CORTEX_URL}/peek?q={urllib.parse.quote(query)}&k={limit}"
    req = urllib.request.Request(
        url,
        headers={"Authorization": f"Bearer {token}"},
    )
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            data = json.loads(resp.read())
            return data.get("matches", [])
    except Exception:
        return []


# ─── Parsing ─────────────────────────────────────────────────────────────────

def parse_conversations(path: Path) -> list[dict]:
    """Parse the ChatGPT export JSON file."""
    with open(path, "r", encoding="utf-8") as f:
        data = json.load(f)

    if not isinstance(data, list):
        print(f"[ERROR] Expected array at root, got {type(data).__name__}", file=sys.stderr)
        sys.exit(1)

    return data


def extract_messages(conversation: dict) -> list[tuple[str, str, float]]:
    """Extract ordered (role, text, timestamp) tuples from a conversation."""
    mapping = conversation.get("mapping", {})
    messages = []

    for node in mapping.values():
        msg = node.get("message")
        if not msg:
            continue

        role = msg.get("author", {}).get("role", "unknown")
        content = msg.get("content", {})
        parts = content.get("parts", [])

        text = ""
        for part in parts:
            if isinstance(part, str):
                text += part
            elif isinstance(part, dict) and "text" in part:
                text += part["text"]

        text = text.strip()
        if not text or len(text) < MIN_MESSAGE_LEN:
            continue

        ts = msg.get("create_time") or conversation.get("create_time") or 0
        messages.append((role, text, ts))

    # Sort by timestamp
    messages.sort(key=lambda m: m[2])
    return messages


# ─── Filtering ───────────────────────────────────────────────────────────────

def compute_tech_score(messages: list[tuple[str, str, float]]) -> float:
    """Score how 'technical' a conversation is (0.0 - 1.0)."""
    user_text = " ".join(text.lower() for role, text, _ in messages if role == "user")
    words = user_text.split()
    if not words:
        return 0.0

    tech_hits = sum(1 for w in words if w in TECH_FINGERPRINT)
    return min(1.0, tech_hits / max(len(words), 1) * 10)


def should_include(conversation: dict, messages: list[tuple[str, str, float]],
                   user_filter: str | None) -> bool:
    """Decide if this conversation belongs to the target user."""
    if user_filter:
        user_text = " ".join(text.lower() for role, text, _ in messages if role == "user")
        if user_filter.lower() not in user_text:
            # Also check conversation title
            title = conversation.get("title", "").lower()
            if user_filter.lower() not in title:
                return False

    # If no filter, include conversations with tech fingerprint > 0.1
    if not user_filter:
        return compute_tech_score(messages) > 0.1

    return True


# ─── Extraction ──────────────────────────────────────────────────────────────

def classify_message(text: str) -> tuple[str, float]:
    """Classify a user message into a memory type with confidence."""
    lower = text.lower()

    # Check for decisions
    decision_hits = sum(1 for kw in DECISION_SIGNALS if kw in lower)
    if decision_hits >= 2:
        return "decision", min(0.9, 0.5 + decision_hits * 0.1)

    # Check for preferences
    pref_hits = sum(1 for kw in PREFERENCE_SIGNALS if kw in lower)
    if pref_hits >= 1:
        return "preference", min(0.85, 0.5 + pref_hits * 0.15)

    # Check for facts about the user
    fact_hits = sum(1 for kw in FACT_SIGNALS if kw in lower)
    if fact_hits >= 1:
        return "fact", min(0.8, 0.5 + fact_hits * 0.15)

    return "context", 0.4


def extract_memories(messages: list[tuple[str, str, float]],
                     conversation_title: str) -> list[ExtractedMemory]:
    """Extract meaningful memories from a conversation's messages."""
    memories = []

    for role, text, ts in messages:
        if role != "user":
            continue

        # Skip very short or very long messages (likely code dumps)
        if len(text) < MIN_MESSAGE_LEN or len(text) > 2000:
            continue

        # Skip messages that are just questions with no assertion
        if text.strip().endswith("?") and len(text) < 100:
            continue

        mem_type, confidence = classify_message(text)

        # Only extract high-confidence memories
        if confidence < 0.5:
            continue

        # Truncate to a reasonable length
        extracted_text = text[:500].strip()
        if len(text) > 500:
            extracted_text += "..."

        memories.append(ExtractedMemory(
            text=extracted_text,
            memory_type=mem_type,
            confidence=confidence,
            source_conversation=conversation_title,
            timestamp=ts,
            tags=["chatgpt-import"],
        ))

    return memories


# ─── Deduplication ───────────────────────────────────────────────────────────

def is_duplicate(token: str, memory: ExtractedMemory) -> bool:
    """Check if a similar memory already exists in Cortex."""
    # Use first 80 chars as the search query
    query = memory.text[:80]
    matches = cortex_peek(token, query, limit=3)

    for match in matches:
        if match.get("relevance", 0) > 0.85:
            return True

    return False


# ─── Main Pipeline ───────────────────────────────────────────────────────────

def run_ingestion(
    conversations_path: Path,
    dry_run: bool = False,
    user_filter: str | None = None,
    max_store: int | None = None,
) -> ConversationStats:
    stats = ConversationStats()

    # Load and parse
    print(f"Loading {conversations_path}...")
    conversations = parse_conversations(conversations_path)
    stats.total_conversations = len(conversations)
    print(f"  Found {len(conversations)} conversations")

    # Get Cortex token
    token = get_cortex_token()
    if not token and not dry_run:
        print("[ERROR] No Cortex token found. Is the daemon running?", file=sys.stderr)
        sys.exit(1)

    # Process each conversation
    all_memories: list[ExtractedMemory] = []

    for conv in conversations:
        title = conv.get("title", "Untitled")
        messages = extract_messages(conv)

        if not messages:
            stats.filtered_out += 1
            continue

        if not should_include(conv, messages, user_filter):
            stats.filtered_out += 1
            continue

        stats.filtered_in += 1
        stats.messages_processed += len(messages)

        memories = extract_memories(messages, title)
        all_memories.extend(memories)

    stats.memories_extracted = len(all_memories)
    print(f"\n  Conversations: {stats.filtered_in} included, {stats.filtered_out} filtered out")
    print(f"  Messages processed: {stats.messages_processed}")
    print(f"  Memories extracted: {stats.memories_extracted}")

    if dry_run:
        print("\n[DRY RUN] Would store these memories:")
        for i, mem in enumerate(all_memories[:20]):
            print(f"  {i+1}. [{mem.memory_type}] (conf={mem.confidence:.2f}) {mem.text[:100]}")
        if len(all_memories) > 20:
            print(f"  ... and {len(all_memories) - 20} more")

        # Show type breakdown
        type_counts = Counter(m.memory_type for m in all_memories)
        print(f"\n  Type breakdown: {dict(type_counts)}")
        return stats

    # Deduplicate and store
    print(f"\n  Deduplicating against Cortex ({CORTEX_URL})...")
    stored = 0
    for i, mem in enumerate(all_memories):
        if max_store and stored >= max_store:
            print(f"  Reached max_store limit ({max_store})")
            break

        if is_duplicate(token, mem):
            stats.duplicates_skipped += 1
            continue

        context = f"Source: ChatGPT conversation '{mem.source_conversation}' " \
                  f"({time.strftime('%Y-%m-%d', time.gmtime(mem.timestamp))})"

        result = cortex_store(
            token=token,
            decision=mem.text,
            context=context,
            entry_type=mem.memory_type,
            confidence=mem.confidence,
            source_agent="chatgpt-import",
        )

        if result and result.get("stored"):
            stored += 1
            stats.stored_to_cortex += 1
            if stored % 10 == 0:
                print(f"  Stored {stored} memories...")
        elif result and not result.get("stored"):
            stats.duplicates_skipped += 1

        # Rate limit: don't overwhelm the daemon
        if stored % 50 == 0 and stored > 0:
            time.sleep(1)

    print(f"\n  === Ingestion Complete ===")
    print(f"  Stored: {stats.stored_to_cortex}")
    print(f"  Duplicates skipped: {stats.duplicates_skipped}")
    print(f"  Total in Cortex: check {CORTEX_URL}/health")

    return stats


# ─── CLI ─────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Ingest ChatGPT conversations into Cortex",
    )
    parser.add_argument("file", type=Path, help="Path to conversations.json")
    parser.add_argument("--dry-run", action="store_true",
                        help="Parse and extract without storing to Cortex")
    parser.add_argument("--user-filter", type=str, default=None,
                        help="Keyword to filter conversations by (e.g. your name, a project)")
    parser.add_argument("--max-store", type=int, default=None,
                        help="Maximum number of memories to store (for testing)")

    args = parser.parse_args()

    if not args.file.exists():
        print(f"[ERROR] File not found: {args.file}", file=sys.stderr)
        sys.exit(1)

    stats = run_ingestion(
        conversations_path=args.file,
        dry_run=args.dry_run,
        user_filter=args.user_filter,
        max_store=args.max_store,
    )

    # Exit with non-zero if nothing was processed
    if stats.total_conversations == 0:
        sys.exit(1)


if __name__ == "__main__":
    import urllib.parse
    main()

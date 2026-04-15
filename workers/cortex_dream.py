"""cortex-dream — Memory compaction worker for Cortex.

Reads all active memories and decisions, clusters by similarity,
and archives duplicates when requested.

Usage:
  python workers/cortex_dream.py              # dry run (default)
  python workers/cortex_dream.py --execute    # actually archive duplicates
  python workers/cortex_dream.py --threshold 0.5  # lower similarity bar
"""

import argparse
import json
import sys
from pathlib import Path
from urllib.error import URLError

# Make repo-local imports work when the worker is run directly.
sys.path.insert(0, str(Path(__file__).parent))
import cortex_client

CLIENT_ERRORS = (URLError, TimeoutError, json.JSONDecodeError, ValueError)


def tokenize(text: str) -> set[str]:
    """Split text into lowercase word tokens, drop short ones."""
    return {w for w in text.lower().split() if len(w) > 2}


def jaccard(a: str, b: str) -> float:
    """Word-level Jaccard similarity between two strings."""
    sa, sb = tokenize(a), tokenize(b)
    if not sa and not sb:
        return 1.0
    if not sa or not sb:
        return 0.0
    intersection = len(sa & sb)
    union = len(sa | sb)
    return intersection / union if union else 0.0


def cluster_entries(entries: list[dict], text_key: str, threshold: float = 0.6) -> list[list[dict]]:
    """Group entries by Jaccard similarity above threshold.

    Simple single-pass greedy clustering: each entry joins the first
    cluster it's similar enough to, or starts a new cluster.
    """
    clusters: list[list[dict]] = []

    for entry in entries:
        text = entry.get(text_key, "")
        placed = False

        for cluster in clusters:
            centroid_text = cluster[0].get(text_key, "")
            if jaccard(text, centroid_text) >= threshold:
                cluster.append(entry)
                placed = True
                break

        if not placed:
            clusters.append([entry])

    return clusters


def print_clusters(clusters: list[list[dict]], text_key: str, label: str):
    """Print clusters that have 2+ entries (duplicates)."""
    dupes = [c for c in clusters if len(c) >= 2]
    if not dupes:
        print(f"  {label}: no duplicates found")
        return

    print(f"  {label}: {len(dupes)} clusters with overlapping entries")
    for i, cluster in enumerate(dupes, 1):
        print(f"\n  Cluster {i} ({len(cluster)} entries):")
        for entry in cluster:
            entry_id = entry.get("id", "?")
            text = entry.get(text_key, "")[:100]
            score = entry.get("score", "?")
            agent = entry.get("source_agent", "?")
            print(f"    #{entry_id} [{agent}] (score: {score}) {text}")


def run_dream(threshold: float = 0.6, execute: bool = False):
    """Main dreaming pipeline."""
    print("Cortex Dream — Memory Compaction")
    print("=" * 40)

    try:
        h = cortex_client.health()
        stats = h.get("stats", {})
        print(f"Connected: {stats.get('memories', '?')} memories, {stats.get('decisions', '?')} decisions")
    except CLIENT_ERRORS as e:
        print(f"Cannot connect to Cortex: {e}")
        return 1

    print(f"\nFetching all active entries...")
    try:
        data = cortex_client.dump()
    except CLIENT_ERRORS as e:
        print(f"Dump failed: {e}")
        return 1

    memories = data.get("memories", [])
    decisions = data.get("decisions", [])
    print(f"  Loaded {len(memories)} memories, {len(decisions)} decisions")

    print(f"\nClustering (threshold: {threshold})...")
    mem_clusters = cluster_entries(memories, "text", threshold)
    dec_clusters = cluster_entries(decisions, "decision", threshold)

    print_clusters(mem_clusters, "text", "Memories")
    print_clusters(dec_clusters, "decision", "Decisions")

    mem_dupes = [c for c in mem_clusters if len(c) >= 2]
    dec_dupes = [c for c in dec_clusters if len(c) >= 2]
    total_archivable = sum(len(c) - 1 for c in mem_dupes) + sum(len(c) - 1 for c in dec_dupes)

    if total_archivable == 0:
        print("\nNo duplicates to compact. Brain is clean.")
        return 0

    print(f"\nTotal archivable: {total_archivable} entries (keeping 1 per cluster)")

    if not execute:
        print("\n[DRY RUN] No changes made. Run with --execute to archive duplicates.")
        return 0

    print("\nArchiving duplicates...")
    archived = 0

    for cluster in mem_dupes:
        cluster.sort(key=lambda e: e.get("score", 0), reverse=True)
        ids_to_archive = [e["id"] for e in cluster[1:]]
        try:
            result = cortex_client.archive("memories", ids_to_archive)
            archived += result.get("archived", 0)
        except CLIENT_ERRORS as e:
            print(f"  Failed to archive memory cluster: {e}")

    for cluster in dec_dupes:
        cluster.sort(key=lambda e: e.get("score", 0), reverse=True)
        ids_to_archive = [e["id"] for e in cluster[1:]]
        try:
            result = cortex_client.archive("decisions", ids_to_archive)
            archived += result.get("archived", 0)
        except CLIENT_ERRORS as e:
            print(f"  Failed to archive decision cluster: {e}")

    print(f"\nDone. Archived {archived} duplicate entries.")
    return 0


def main():
    parser = argparse.ArgumentParser(description="Cortex Dream — Memory compaction worker")
    parser.add_argument("--execute", action="store_true", help="Actually archive duplicates (default: dry run)")
    parser.add_argument("--threshold", type=float, default=0.6, help="Jaccard similarity threshold (default: 0.6)")
    args = parser.parse_args()

    sys.exit(run_dream(threshold=args.threshold, execute=args.execute))


if __name__ == "__main__":
    main()

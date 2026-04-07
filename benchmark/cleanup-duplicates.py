#!/usr/bin/env python3
"""
One-time Cortex duplicate purge.

Default mode is a dry run. Use --apply to delete duplicate active memories and
decisions plus their embeddings in one transaction. Every applied deletion is
logged to events.data with the deleted row and embedding encoded for reversal.
"""

from __future__ import annotations

import argparse
import base64
import json
import sqlite3
import struct
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable


DB_PATH = Path.home() / "cortex" / "cortex.db"
COSINE_DELETE_THRESHOLD = 0.92
COSINE_REVIEW_THRESHOLD = 0.90
JACCARD_DELETE_THRESHOLD = 0.70
PREVIEW_CHARS = 80


@dataclass(frozen=True)
class Entry:
    kind: str
    table: str
    id: int
    text: str
    source: str | None
    score: float
    retrievals: int
    created_at: str | None
    row: dict
    embedding: dict
    vector: tuple[float, ...]
    token_set: frozenset[str]


@dataclass(frozen=True)
class DuplicateEdge:
    kind: str
    left_id: int
    right_id: int
    similarity: float
    reason: str
    jaccard: float | None = None


def die(message: str) -> None:
    print(f"ERROR: {message}", file=sys.stderr)
    sys.exit(1)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Dry-run or apply Cortex duplicate memory/decision purge."
    )
    parser.add_argument("--apply", action="store_true", help="execute the deletion plan")
    parser.add_argument(
        "--db",
        type=Path,
        default=DB_PATH,
        help=f"SQLite DB path (default: {DB_PATH})",
    )
    return parser.parse_args()


def blob_to_vector(blob: bytes) -> tuple[float, ...]:
    if len(blob) % 4 != 0:
        die(f"Embedding BLOB has invalid byte length {len(blob)}; expected float32 multiples.")
    return struct.unpack(f"{len(blob) // 4}f", blob)


def cosine_similarity(a: tuple[float, ...], b: tuple[float, ...]) -> float:
    if len(a) != len(b):
        raise ValueError(f"Embedding dimension mismatch: {len(a)} vs {len(b)}.")

    dot = 0.0
    norm_a = 0.0
    norm_b = 0.0
    for x, y in zip(a, b):
        dot += x * y
        norm_a += x * x
        norm_b += y * y
    if norm_a == 0.0 or norm_b == 0.0:
        return 0.0
    return dot / ((norm_a**0.5) * (norm_b**0.5))


def whitespace_tokens(text: str) -> frozenset[str]:
    return frozenset(text.lower().split())


def jaccard(a: frozenset[str], b: frozenset[str]) -> float:
    if not a and not b:
        return 1.0
    union = a | b
    if not union:
        return 0.0
    return len(a & b) / len(union)


def has_contradiction_marker(left: Entry, right: Entry) -> bool:
    left_text = left.text.lower()
    right_text = right.text.lower()
    marker_pairs = [
        ("[pattern]", "[anti-pattern]"),
        ("often", "rarely"),
    ]
    for marker_a, marker_b in marker_pairs:
        if marker_a in left_text and marker_b in right_text:
            return True
        if marker_b in left_text and marker_a in right_text:
            return True
    return False


def keeper_key(entry: Entry) -> tuple[float, int, int]:
    # Higher score wins, then higher retrievals, then older/lower id.
    return (entry.score, entry.retrievals, -entry.id)


def preview(text: str) -> str:
    compact = " ".join(text.split())
    if len(compact) <= PREVIEW_CHARS:
        return compact
    return compact[: PREVIEW_CHARS - 3] + "..."


def dict_from_row(row: sqlite3.Row, skip: Iterable[str] = ()) -> dict:
    skipped = set(skip)
    return {key: row[key] for key in row.keys() if key not in skipped}


def load_entries(conn: sqlite3.Connection, kind: str) -> list[Entry]:
    if kind == "memory":
        table = "memories"
        text_expr = "m.text"
        query = """
            SELECT
                m.*,
                e.id AS embedding_id,
                e.vector AS embedding_vector,
                e.model AS embedding_model,
                e.created_at AS embedding_created_at
            FROM memories m
            JOIN embeddings e
              ON e.target_type = 'memory'
             AND e.target_id = m.id
            WHERE m.status = 'active'
            ORDER BY m.id, e.id DESC
        """
    elif kind == "decision":
        table = "decisions"
        text_expr = "COALESCE(d.decision, '') || ' ' || COALESCE(d.context, '')"
        query = """
            SELECT
                d.*,
                e.id AS embedding_id,
                e.vector AS embedding_vector,
                e.model AS embedding_model,
                e.created_at AS embedding_created_at
            FROM decisions d
            JOIN embeddings e
              ON e.target_type = 'decision'
             AND e.target_id = d.id
            WHERE d.status = 'active'
            ORDER BY d.id, e.id DESC
        """
    else:
        raise ValueError(f"unsupported kind: {kind}")

    seen: set[int] = set()
    entries: list[Entry] = []
    duplicate_embedding_targets: list[int] = []
    for row in conn.execute(query):
        entry_id = int(row["id"])
        if entry_id in seen:
            duplicate_embedding_targets.append(entry_id)
            continue
        seen.add(entry_id)

        if kind == "memory":
            text = row["text"] or ""
            source = row["source"]
        else:
            decision = row["decision"] or ""
            context = row["context"] or ""
            text = f"{decision} {context}".strip()
            source = row["source_agent"]

        row_data = dict_from_row(
            row,
            skip={
                "embedding_id",
                "embedding_vector",
                "embedding_model",
                "embedding_created_at",
            },
        )
        embedding_blob = row["embedding_vector"]
        embedding_data = {
            "id": row["embedding_id"],
            "target_type": kind,
            "target_id": entry_id,
            "model": row["embedding_model"],
            "created_at": row["embedding_created_at"],
            "vector_b64": base64.b64encode(embedding_blob).decode("ascii"),
        }
        entries.append(
            Entry(
                kind=kind,
                table=table,
                id=entry_id,
                text=text,
                source=source,
                score=float(row["score"] if row["score"] is not None else 0.0),
                retrievals=int(row["retrievals"] if row["retrievals"] is not None else 0),
                created_at=row["created_at"],
                row=row_data,
                embedding=embedding_data,
                vector=blob_to_vector(embedding_blob),
                token_set=whitespace_tokens(text),
            )
        )

    if duplicate_embedding_targets:
        print(
            f"WARN: {kind} targets with multiple embeddings were found; "
            f"using newest embedding per target. ids={duplicate_embedding_targets}",
            file=sys.stderr,
        )

    # text_expr is kept explicit above to make the query intent easy to audit.
    _ = text_expr
    return entries


def scan_duplicates(entries: list[Entry]) -> tuple[list[DuplicateEdge], dict[str, int]]:
    edges: list[DuplicateEdge] = []
    counters = {
        "cosine_gt_092": 0,
        "cosine_090_092": 0,
        "jaccard_accepted": 0,
        "jaccard_rejected": 0,
        "safety_rejected": 0,
        "dimension_mismatch": 0,
    }

    for i, left in enumerate(entries):
        for right in entries[i + 1 :]:
            if len(left.vector) != len(right.vector):
                counters["dimension_mismatch"] += 1
                continue
            sim = cosine_similarity(left.vector, right.vector)
            if sim > COSINE_DELETE_THRESHOLD:
                counters["cosine_gt_092"] += 1
                jac = jaccard(left.token_set, right.token_set)
                if jac > JACCARD_DELETE_THRESHOLD and not has_contradiction_marker(
                    left, right
                ):
                    edges.append(
                        DuplicateEdge(
                            left.kind,
                            left.id,
                            right.id,
                            sim,
                            "cosine>0.92+jaccard>0.70",
                            jac,
                        )
                    )
                else:
                    counters["safety_rejected"] += 1
            elif COSINE_REVIEW_THRESHOLD <= sim <= COSINE_DELETE_THRESHOLD:
                counters["cosine_090_092"] += 1
                jac = jaccard(left.token_set, right.token_set)
                if jac > JACCARD_DELETE_THRESHOLD:
                    counters["jaccard_accepted"] += 1
                    edges.append(
                        DuplicateEdge(
                            left.kind,
                            left.id,
                            right.id,
                            sim,
                            "cosine0.90-0.92+jaccard>0.70",
                            jac,
                        )
                    )
                else:
                    counters["jaccard_rejected"] += 1

    return edges, counters


def connected_components(edges: list[DuplicateEdge]) -> list[set[int]]:
    parent: dict[int, int] = {}

    def find(x: int) -> int:
        parent.setdefault(x, x)
        if parent[x] != x:
            parent[x] = find(parent[x])
        return parent[x]

    def union(a: int, b: int) -> None:
        root_a = find(a)
        root_b = find(b)
        if root_a != root_b:
            parent[root_b] = root_a

    for edge in edges:
        union(edge.left_id, edge.right_id)

    components: dict[int, set[int]] = {}
    for node in list(parent):
        components.setdefault(find(node), set()).add(node)
    return [nodes for nodes in components.values() if len(nodes) > 1]


def build_plan(
    entries: list[Entry], edges: list[DuplicateEdge]
) -> tuple[dict[int, dict], list[dict]]:
    by_id = {entry.id: entry for entry in entries}
    edge_lookup: dict[frozenset[int], DuplicateEdge] = {
        frozenset((edge.left_id, edge.right_id)): edge for edge in edges
    }
    deletions: dict[int, dict] = {}
    components_log: list[dict] = []

    for component in connected_components(edges):
        survivor = max((by_id[node_id] for node_id in component), key=keeper_key)
        components_log.append(
            {
                "survivor_id": survivor.id,
                "deleted_ids": sorted(node_id for node_id in component if node_id != survivor.id),
                "member_ids": sorted(component),
            }
        )
        for node_id in component:
            if node_id == survivor.id:
                continue

            deleted = by_id[node_id]
            direct_edge = edge_lookup.get(frozenset((deleted.id, survivor.id)))
            if direct_edge is None:
                direct_edges = [
                    edge
                    for edge in edges
                    if deleted.id in (edge.left_id, edge.right_id)
                    and edge.left_id in component
                    and edge.right_id in component
                ]
                direct_edge = max(direct_edges, key=lambda edge: edge.similarity)
                twin_id = (
                    direct_edge.right_id
                    if direct_edge.left_id == deleted.id
                    else direct_edge.left_id
                )
            else:
                twin_id = survivor.id

            deletions[deleted.id] = {
                "kind": deleted.kind,
                "table": deleted.table,
                "id": deleted.id,
                "survivor_id": survivor.id,
                "twin_id": twin_id,
                "similarity": direct_edge.similarity,
                "reason": direct_edge.reason,
                "jaccard": direct_edge.jaccard,
                "deleted": deleted,
                "survivor": survivor,
            }

    return deletions, components_log


def count_rows(conn: sqlite3.Connection) -> dict[str, int]:
    return {
        "active_memories": conn.execute(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'"
        ).fetchone()[0],
        "active_decisions": conn.execute(
            "SELECT COUNT(*) FROM decisions WHERE status = 'active'"
        ).fetchone()[0],
        "embeddings": conn.execute("SELECT COUNT(*) FROM embeddings").fetchone()[0],
    }


def deletion_payload(plan_entry: dict) -> dict:
    deleted: Entry = plan_entry["deleted"]
    survivor: Entry = plan_entry["survivor"]
    return {
        "kind": deleted.kind,
        "table": deleted.table,
        "deleted_id": deleted.id,
        "survivor_id": survivor.id,
        "twin_id": plan_entry["twin_id"],
        "similarity": round(plan_entry["similarity"], 6),
        "reason": plan_entry["reason"],
        "jaccard": None
        if plan_entry["jaccard"] is None
        else round(plan_entry["jaccard"], 6),
        "deleted_row": deleted.row,
        "deleted_embedding": deleted.embedding,
        "survivor_preview": preview(survivor.text),
    }


def insert_event_log(
    conn: sqlite3.Connection,
    before_counts: dict[str, int],
    after_counts: dict[str, int],
    memory_deletions: list[dict],
    decision_deletions: list[dict],
    components: dict[str, list[dict]],
) -> None:
    payload = {
        "event_type": "duplicate_purge",
        "created_by": "codex",
        "created_at": datetime.now(timezone.utc).isoformat(),
        "reversible": True,
        "thresholds": {
            "cosine_delete": COSINE_DELETE_THRESHOLD,
            "cosine_review": COSINE_REVIEW_THRESHOLD,
            "jaccard_delete": JACCARD_DELETE_THRESHOLD,
        },
        "before_counts": before_counts,
        "after_counts": after_counts,
        "components": components,
        "deletions": memory_deletions + decision_deletions,
    }

    columns = {
        row["name"] for row in conn.execute("PRAGMA table_info(events)").fetchall()
    }
    event_column = "event_type" if "event_type" in columns else "type"
    if "source_agent" in columns:
        conn.execute(
            f"INSERT INTO events ({event_column}, data, source_agent, created_at) "
            "VALUES (?1, ?2, ?3, datetime('now'))",
            ("duplicate_purge", json.dumps(payload, sort_keys=True), "codex"),
        )
    else:
        conn.execute(
            f"INSERT INTO events ({event_column}, data, created_at) "
            "VALUES (?1, ?2, datetime('now'))",
            ("duplicate_purge", json.dumps(payload, sort_keys=True)),
        )


def delete_ids(conn: sqlite3.Connection, table: str, kind: str, ids: list[int]) -> None:
    if not ids:
        return
    placeholders = ", ".join("?" for _ in ids)
    conn.execute(f"DELETE FROM {table} WHERE id IN ({placeholders})", ids)
    conn.execute(
        f"DELETE FROM embeddings WHERE target_type = ? AND target_id IN ({placeholders})",
        [kind, *ids],
    )


def apply_plan(
    conn: sqlite3.Connection,
    before_counts: dict[str, int],
    memory_deletions: list[dict],
    decision_deletions: list[dict],
    components: dict[str, list[dict]],
) -> dict[str, int]:
    memory_ids = sorted(item["deleted_id"] for item in memory_deletions)
    decision_ids = sorted(item["deleted_id"] for item in decision_deletions)

    try:
        conn.execute("BEGIN IMMEDIATE")
        delete_ids(conn, "memories", "memory", memory_ids)
        delete_ids(conn, "decisions", "decision", decision_ids)
        after_counts = count_rows(conn)
        insert_event_log(
            conn,
            before_counts,
            after_counts,
            memory_deletions,
            decision_deletions,
            components,
        )
        conn.execute("COMMIT")
        return after_counts
    except Exception:
        conn.execute("ROLLBACK")
        raise


def print_plan(
    kind: str,
    entries: list[Entry],
    counters: dict[str, int],
    deletions: dict[int, dict],
) -> None:
    print(f"\n{kind.upper()} duplicate scan")
    print(f"  Active {kind}s with embeddings: {len(entries)}")
    print(f"  Pairs cosine > 0.92: {counters['cosine_gt_092']}")
    print(f"  Pairs cosine 0.90-0.92: {counters['cosine_090_092']}")
    print(f"  Jaccard accepted (>0.70): {counters['jaccard_accepted']}")
    print(f"  Jaccard rejected (<=0.70): {counters['jaccard_rejected']}")
    print(f"  Safety rejected high-cosine pairs: {counters['safety_rejected']}")
    print(f"  Skipped cross-dimension pairs: {counters['dimension_mismatch']}")

    if not deletions:
        print(f"  No {kind} deletions planned.")
        return

    print(f"  Planned {kind} deletions:")
    for plan_entry in sorted(
        deletions.values(),
        key=lambda item: (-item["similarity"], item["deleted"].id),
    ):
        deleted: Entry = plan_entry["deleted"]
        print(
            f"    id={deleted.id} source={deleted.source or ''!r} "
            f"score={deleted.score:.3f} retrievals={deleted.retrievals} "
            f"twin={plan_entry['twin_id']} survivor={plan_entry['survivor_id']} "
            f"cosine={plan_entry['similarity']:.6f} reason={plan_entry['reason']}"
        )
        print(f"      preview: {preview(deleted.text)}")


def main() -> None:
    args = parse_args()
    if not args.db.exists():
        die(f"Database not found at {args.db} -- verify ~/cortex/cortex.db exists.")

    conn = sqlite3.connect(args.db)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys = ON")
    conn.execute("PRAGMA busy_timeout = 5000")

    before_counts = count_rows(conn)
    memory_entries = load_entries(conn, "memory")
    decision_entries = load_entries(conn, "decision")

    memory_edges, memory_counters = scan_duplicates(memory_entries)
    decision_edges, decision_counters = scan_duplicates(decision_entries)
    memory_deletion_plan, memory_components = build_plan(memory_entries, memory_edges)
    decision_deletion_plan, decision_components = build_plan(decision_entries, decision_edges)

    print(f"Mode: {'APPLY' if args.apply else 'DRY RUN'}")
    print(f"Database: {args.db}")
    print(
        "Before counts: "
        f"{before_counts['active_memories']} active memories, "
        f"{before_counts['active_decisions']} active decisions, "
        f"{before_counts['embeddings']} embeddings"
    )

    print_plan("memory", memory_entries, memory_counters, memory_deletion_plan)
    print_plan("decision", decision_entries, decision_counters, decision_deletion_plan)

    memory_deletions = [
        deletion_payload(item) for item in memory_deletion_plan.values()
    ]
    decision_deletions = [
        deletion_payload(item) for item in decision_deletion_plan.values()
    ]
    memory_count = len(memory_deletions)
    decision_count = len(decision_deletions)
    before_noise_base = max(before_counts["active_memories"] + before_counts["active_decisions"], 1)
    estimated_noise_reduction = ((memory_count + decision_count) / before_noise_base) * 100

    print(
        f"\nSummary: Would delete {memory_count} memories, {decision_count} decisions. "
        f"Estimated noise reduction: {estimated_noise_reduction:.1f}%"
    )

    if not args.apply:
        print("Dry run only. Re-run with --apply to execute this plan in one transaction.")
        return

    after_counts = apply_plan(
        conn,
        before_counts,
        memory_deletions,
        decision_deletions,
        {"memory": memory_components, "decision": decision_components},
    )
    print("\nApplied duplicate purge in one transaction.")
    print(
        "After counts: "
        f"{after_counts['active_memories']} active memories "
        f"(was {before_counts['active_memories']}), "
        f"{after_counts['active_decisions']} active decisions "
        f"(was {before_counts['active_decisions']}), "
        f"{after_counts['embeddings']} embeddings "
        f"(was {before_counts['embeddings']})"
    )
    print(
        f"Deleted {memory_count} memories and {decision_count} decisions; "
        "reversal data was written to events.data where type='duplicate_purge'."
    )


if __name__ == "__main__":
    main()

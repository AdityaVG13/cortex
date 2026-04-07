"""
Measure TRUE recall: for each query, find ALL relevant items in the DB,
then check how many Cortex actually returned.

recall = returned_relevant / total_relevant_in_db
"""

import json
import sqlite3
import sys
import time
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

CORTEX_URL = "http://127.0.0.1:7437"
TOKEN_FILE = Path.home() / ".cortex" / "cortex.token"
DB_PATH = Path.home() / "cortex" / "cortex.db"

AUTH_TOKEN = TOKEN_FILE.read_text().strip() if TOKEN_FILE.exists() else ""
AUTH_HEADERS = {
    "Authorization": f"Bearer {AUTH_TOKEN}",
    "X-Cortex-Request": "true",
}


def die(message: str) -> None:
    print(f"ERROR: {message}", file=sys.stderr)
    sys.exit(1)


def request_json(url: str, timeout: int = 5) -> dict:
    if not AUTH_TOKEN:
        die(f"Missing Cortex token at {TOKEN_FILE} -- start Cortex once to create it, then re-run the benchmark.")

    req = Request(url, headers=AUTH_HEADERS)
    try:
        resp = urlopen(req, timeout=timeout)
        return json.loads(resp.read())
    except HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        die(
            f"Cortex API returned HTTP {e.code} for {url}: {body}. "
            "Verify the daemon is on port 7437 and the token in ~/.cortex/cortex.token is current."
        )
    except URLError as e:
        die(
            "Daemon not responding on port 7437 -- run `cargo run --release` in daemon-rs/ "
            f"or check CORTEX_PORT. Details: {e}"
        )
    except TimeoutError as e:
        die(
            "Daemon timed out on port 7437 -- check daemon logs for a stuck request "
            f"and re-run the benchmark. Details: {e}"
        )
    except json.JSONDecodeError as e:
        die(f"Cortex returned invalid JSON for {url} -- check daemon logs. Details: {e}")


def check_daemon_health() -> None:
    data = request_json(f"{CORTEX_URL}/health", timeout=3)
    if data.get("status") != "ok":
        die(f"Daemon health check failed at {CORTEX_URL}/health -- response was: {data!r}")

    stats = data.get("stats", {})
    print(
        f"  Daemon health: ok | memories={stats.get('memories', '?')} "
        f"decisions={stats.get('decisions', '?')} embeddings={stats.get('embeddings', '?')}"
    )

# Same queries + ground truth patterns from v2
QUERIES = [
    {"q": "token optimization settings", "gt": ["token optimization", "ENABLE_TOOL_SEARCH", "thinking_tokens", "output_tokens", "compression"]},
    {"q": "cache expiry guard hook", "gt": ["cache expiry", "cache-expiry-guard", "UserPromptSubmit", "idle", "TTL"]},
    {"q": "RTK path fix bashrc", "gt": ["rtk", "bashrc", "PATH", "rtk-real", "local/bin"]},
    {"q": "CCMeter analytics dashboard", "gt": ["ccmeter", "analytics", "dashboard", "heatmap", "session"]},
    {"q": "browser cleanup playwright", "gt": ["browser", "playwright", "chrome", "dev-browser", "cleanup"]},
    {"q": "uv python package management", "gt": ["uv", "python", "pip", "package", "pytest"]},
    {"q": "never use em-dashes", "gt": ["em-dash", "em dash", "double hyphen", "--", "emdash"]},
    {"q": "cortex recall before investigation", "gt": ["cortex_recall", "recall", "before", "investigation", "debug"]},
    {"q": "codex agent contributions", "gt": ["codex", "agent", "scout", "batch", "build"]},
    {"q": "gemini agent decisions", "gt": ["gemini", "agent", "decision", "model"]},
    {"q": "factory droid builds", "gt": ["factory", "droid", "build", "automation"]},
    {"q": "multi-agent shared state", "gt": ["multi-agent", "shared", "conductor", "session", "team"]},
    {"q": "boot compiler capsule system", "gt": ["boot", "capsule", "compiler", "identity", "delta"]},
    {"q": "conflict detection jaccard cosine", "gt": ["conflict", "jaccard", "cosine", "similarity", "supersed"]},
    {"q": "embedding engine MiniLM", "gt": ["embedding", "MiniLM", "ONNX", "vector", "cosine"]},
    {"q": "crystal cluster formation", "gt": ["crystal", "cluster", "leiden", "community", "pattern"]},
    {"q": "user writing voice style", "gt": ["writing", "voice", "style", "confident", "earnest"]},
    {"q": "self improvement engine goals", "gt": ["self-improvement", "improvement", "compound", "autoresearch", "lesson"]},
    {"q": "tauri dashboard control center", "gt": ["tauri", "dashboard", "control", "desktop", "metrics"]},
    {"q": "job applicator skill", "gt": ["job", "applicat", "indeed", "skill", "tracker"]},
]


def matches_ground_truth(text: str, gt_patterns: list[str]) -> bool:
    lower = text.lower()
    return any(p.lower() in lower for p in gt_patterns)


def find_all_relevant_in_db(gt_patterns: list[str]) -> list[dict]:
    """Scan entire DB for items matching ground truth patterns."""
    conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    relevant = []

    # Search decisions
    for row in conn.execute(
        "SELECT id, decision, context, source_agent FROM decisions WHERE status='active'"
    ):
        text = f"{row[1]} {row[2] or ''}"
        if matches_ground_truth(text, gt_patterns):
            relevant.append({
                "type": "decision",
                "id": row[0],
                "preview": row[1][:80],
                "agent": row[3],
            })

    # Search memories
    for row in conn.execute(
        "SELECT id, text, type FROM memories WHERE status='active'"
    ):
        if matches_ground_truth(row[1], gt_patterns):
            relevant.append({
                "type": f"memory:{row[2]}",
                "id": row[0],
                "preview": row[1][:80],
            })

    conn.close()
    return relevant


def cortex_recall(query: str, budget: int = 500) -> list[dict]:
    url = f"{CORTEX_URL}/recall?q={query.replace(' ', '+')}&budget={budget}"
    for attempt in range(3):
        data = request_json(url, timeout=5)
        results = data.get("results", [])
        if results or attempt == 2:
            if not results:
                print(
                    f"WARN: recall returned 0 results after 3 attempts for query: {query}",
                    file=sys.stderr,
                )
            return results
        time.sleep(0.2)
    return []


def check_returned(returned_results: list[dict], db_item: dict) -> bool:
    """Check if a DB item was in the returned results."""
    item_id = str(db_item["id"])
    item_preview = db_item["preview"].lower()[:40]

    for r in returned_results:
        source = r.get("source", "").lower()
        excerpt = r.get("excerpt", "").lower()
        # Match by ID in source or by content overlap
        if item_id in source:
            return True
        if item_preview[:30] in excerpt:
            return True
    return False


def run():
    print("=" * 70)
    print("RECALL METRIC: How much of what exists does Cortex find?")
    print("=" * 70)
    check_daemon_health()
    print()

    all_results = []

    for i, q in enumerate(QUERIES):
        query = q["q"]
        gt = q["gt"]

        # Find ALL relevant items in DB
        db_relevant = find_all_relevant_in_db(gt)
        # Get Cortex results
        returned = cortex_recall(query)
        # Check which DB items were found
        found = sum(1 for item in db_relevant if check_returned(returned, item))

        total = len(db_relevant)
        recall = found / total if total > 0 else 1.0  # 1.0 if nothing to find

        # Also compute precision from v2 data
        returned_relevant = sum(
            1 for r in returned
            if matches_ground_truth(
                f"{r.get('excerpt','')} {r.get('source','')}",
                gt
            )
        )
        precision = returned_relevant / len(returned) if returned else 0
        f1 = (2 * precision * recall / (precision + recall)) if (precision + recall) > 0 else 0

        all_results.append({
            "query": query,
            "db_relevant": total,
            "found": found,
            "returned": len(returned),
            "returned_relevant": returned_relevant,
            "recall": round(recall, 3),
            "precision": round(precision, 3),
            "f1": round(f1, 3),
        })

        print(f"  [{i+1:2d}/20] {query[:38]:<38} | "
              f"DB:{total:2d} rel | found:{found:2d} | "
              f"R={recall:.0%} P={precision:.0%} F1={f1:.2f}")

    # Aggregates
    print("\n" + "=" * 70)
    print("AGGREGATE")
    print("=" * 70)

    avg_recall = sum(r["recall"] for r in all_results) / len(all_results)
    avg_precision = sum(r["precision"] for r in all_results) / len(all_results)
    avg_f1 = sum(r["f1"] for r in all_results) / len(all_results)
    total_db = sum(r["db_relevant"] for r in all_results)
    total_found = sum(r["found"] for r in all_results)
    macro_recall = total_found / total_db if total_db else 0

    print(f"  Avg Recall:          {avg_recall:.0%}")
    print(f"  Avg Precision:       {avg_precision:.0%}")
    print(f"  Avg F1:              {avg_f1:.2f}")
    print(f"  Macro Recall:        {total_found}/{total_db} ({macro_recall:.0%})")
    print(f"  (items found / items that exist in DB)")

    # Worst recall queries
    print("\n  LOWEST RECALL (where Cortex misses the most):")
    sorted_by_recall = sorted(all_results, key=lambda r: r["recall"])
    for r in sorted_by_recall[:5]:
        if r["recall"] < 1.0:
            print(f"    {r['query'][:40]:<40} R={r['recall']:.0%} ({r['found']}/{r['db_relevant']})")

    # Save
    out_path = Path(__file__).parent / "recall-metric-results.json"
    output = {
        "aggregate": {
            "avg_recall": round(avg_recall, 3),
            "avg_precision": round(avg_precision, 3),
            "avg_f1": round(avg_f1, 3),
            "macro_recall": round(macro_recall, 3),
            "total_relevant_in_db": total_db,
            "total_found": total_found,
        },
        "results": all_results,
    }
    Path(out_path).write_text(json.dumps(output, indent=2))
    print(f"\n  Saved to: {out_path}")


if __name__ == "__main__":
    run()

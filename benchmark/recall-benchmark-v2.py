"""
Cortex Recall Benchmark v2
- Cosine similarity scoring via Cortex embeddings
- Manual relevance labels for ground truth
- Keyword + semantic + combined precision metrics
"""

import json
import sys
import time
import sqlite3
import struct
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

CORTEX_URL = "http://127.0.0.1:7437"
TOKEN_FILE = Path.home() / ".cortex" / "cortex.token"
DB_PATH = Path.home() / "cortex" / "cortex.db"
MEMORY_DIR = Path.home() / ".claude" / "projects" / "C--Users-aditya" / "memory"

AUTH_TOKEN = TOKEN_FILE.read_text().strip() if TOKEN_FILE.exists() else ""
AUTH_HEADERS = {
    "Authorization": f"Bearer {AUTH_TOKEN}",
    "X-Cortex-Request": "true",
}


def die(message: str) -> None:
    print(f"ERROR: {message}", file=sys.stderr)
    sys.exit(1)


def request_json(url: str, timeout: int = 5, fatal: bool = True) -> dict | None:
    if not AUTH_TOKEN:
        die(f"Missing Cortex token at {TOKEN_FILE} -- start Cortex once to create it, then re-run the benchmark.")

    req = Request(url, headers=AUTH_HEADERS)
    try:
        resp = urlopen(req, timeout=timeout)
        return json.loads(resp.read())
    except HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        if fatal:
            die(
                f"Cortex API returned HTTP {e.code} for {url}: {body}. "
                "Verify the daemon is on port 7437 and the token in ~/.cortex/cortex.token is current."
            )
        return None
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
    if not data or data.get("status") != "ok":
        die(f"Daemon health check failed at {CORTEX_URL}/health -- response was: {data!r}")

    stats = data.get("stats", {})
    print(
        f"  Daemon health: ok | memories={stats.get('memories', '?')} "
        f"decisions={stats.get('decisions', '?')} embeddings={stats.get('embeddings', '?')}"
    )

# 20 queries with manual ground truth labels
# relevant_sources: list of substrings that should appear in relevant results
QUERIES = [
    {
        "q": "token optimization settings",
        "category": "project_decisions",
        "ground_truth": ["token optimization", "ENABLE_TOOL_SEARCH", "thinking_tokens", "output_tokens", "compression"],
    },
    {
        "q": "cache expiry guard hook",
        "category": "project_decisions",
        "ground_truth": ["cache expiry", "cache-expiry-guard", "UserPromptSubmit", "idle", "TTL"],
    },
    {
        "q": "RTK path fix bashrc",
        "category": "project_decisions",
        "ground_truth": ["rtk", "bashrc", "PATH", "rtk-real", "local/bin"],
    },
    {
        "q": "CCMeter analytics dashboard",
        "category": "project_decisions",
        "ground_truth": ["ccmeter", "analytics", "dashboard", "heatmap", "session"],
    },
    {
        "q": "browser cleanup playwright",
        "category": "feedback_rules",
        "ground_truth": ["browser", "playwright", "chrome", "dev-browser", "cleanup"],
    },
    {
        "q": "uv python package management",
        "category": "feedback_rules",
        "ground_truth": ["uv", "python", "pip", "package", "pytest"],
    },
    {
        "q": "never use em-dashes",
        "category": "feedback_rules",
        "ground_truth": ["em-dash", "em dash", "double hyphen", "--", "emdash"],
    },
    {
        "q": "cortex recall before investigation",
        "category": "feedback_rules",
        "ground_truth": ["cortex_recall", "recall", "before", "investigation", "debug"],
    },
    {
        "q": "codex agent contributions",
        "category": "cross_agent",
        "ground_truth": ["codex", "agent", "scout", "batch", "build"],
    },
    {
        "q": "gemini agent decisions",
        "category": "cross_agent",
        "ground_truth": ["gemini", "agent", "decision", "model"],
    },
    {
        "q": "factory droid builds",
        "category": "cross_agent",
        "ground_truth": ["factory", "droid", "build", "automation"],
    },
    {
        "q": "multi-agent shared state",
        "category": "cross_agent",
        "ground_truth": ["multi-agent", "shared", "conductor", "session", "team"],
    },
    {
        "q": "boot compiler capsule system",
        "category": "architecture",
        "ground_truth": ["boot", "capsule", "compiler", "identity", "delta"],
    },
    {
        "q": "conflict detection jaccard cosine",
        "category": "architecture",
        "ground_truth": ["conflict", "jaccard", "cosine", "similarity", "supersed"],
    },
    {
        "q": "embedding engine MiniLM",
        "category": "architecture",
        "ground_truth": ["embedding", "MiniLM", "ONNX", "vector", "cosine"],
    },
    {
        "q": "crystal cluster formation",
        "category": "architecture",
        "ground_truth": ["crystal", "cluster", "leiden", "community", "pattern"],
    },
    {
        "q": "user writing voice style",
        "category": "user_context",
        "ground_truth": ["writing", "voice", "style", "confident", "earnest"],
    },
    {
        "q": "self improvement engine goals",
        "category": "user_context",
        "ground_truth": ["self-improvement", "improvement", "compound", "autoresearch", "lesson"],
    },
    {
        "q": "tauri dashboard control center",
        "category": "user_context",
        "ground_truth": ["tauri", "dashboard", "control", "desktop", "metrics"],
    },
    {
        "q": "job applicator skill",
        "category": "user_context",
        "ground_truth": ["job", "applicat", "indeed", "skill", "tracker"],
    },
]


def cortex_recall(query: str, budget: int = 500) -> dict:
    url = f"{CORTEX_URL}/recall?q={query.replace(' ', '+')}&budget={budget}"
    total_elapsed_ms = 0.0
    data: dict | None = None
    results = []
    for attempt in range(3):
        start = time.perf_counter()
        data = request_json(url, timeout=5)
        total_elapsed_ms += (time.perf_counter() - start) * 1000
        results = data.get("results", []) if data else []
        if results or attempt == 2:
            if not results:
                print(
                    f"WARN: recall returned 0 results after 3 attempts for query: {query}",
                    file=sys.stderr,
                )
            break
        time.sleep(0.2)
    return {
        "results": results,
        "elapsed_ms": round(total_elapsed_ms, 1),
        "token_estimate": sum(r.get("tokens", 0) for r in results),
    }


def blob_to_vector(blob: bytes) -> list[float]:
    """Convert SQLite BLOB to float vector."""
    n = len(blob) // 4
    return list(struct.unpack(f'{n}f', blob))


def cosine_sim(a: list[float], b: list[float]) -> float:
    dot = sum(x * y for x, y in zip(a, b))
    norm_a = sum(x * x for x in a) ** 0.5
    norm_b = sum(x * x for x in b) ** 0.5
    if norm_a == 0 or norm_b == 0:
        return 0.0
    return dot / (norm_a * norm_b)


def get_query_embedding(query: str) -> list[float] | None:
    """Get embedding for a query string via Cortex API."""
    import urllib.parse
    url = f"{CORTEX_URL}/embed?text={urllib.parse.quote(query)}"
    data = request_json(url, timeout=5, fatal=False)
    return data.get("vector", None) if data else None


def load_all_embeddings() -> dict:
    """Load all embeddings from DB for cosine scoring."""
    conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    cur = conn.cursor()
    embeddings = {}
    for row in cur.execute(
        "SELECT target_type, target_id, vector FROM embeddings"
    ):
        key = f"{row[0]}:{row[1]}"
        embeddings[key] = blob_to_vector(row[2])
    conn.close()
    return embeddings


def score_keyword(query: str, excerpt: str, source: str) -> float:
    """Keyword overlap score (0-1)."""
    keywords = set(query.lower().split())
    text = (excerpt + " " + source).lower()
    matched = sum(1 for kw in keywords if kw in text)
    return matched / len(keywords) if keywords else 0


def score_ground_truth(ground_truth: list[str], excerpt: str, source: str) -> bool:
    """Check if result matches any ground truth pattern."""
    text = (excerpt + " " + source).lower()
    matched = sum(1 for gt in ground_truth if gt.lower() in text)
    # At least 1 ground truth match = relevant
    return matched >= 1


def naive_baseline_tokens() -> int:
    """Total tokens if you dump all memories + decisions raw."""
    total_chars = 0
    if MEMORY_DIR.exists():
        for md in MEMORY_DIR.glob("*.md"):
            total_chars += md.stat().st_size

    try:
        conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
        for row in conn.execute("SELECT decision, context FROM decisions WHERE status='active'"):
            total_chars += len(row[0] or "")
            total_chars += len(row[1] or "")
        for row in conn.execute("SELECT text FROM memories WHERE status='active'"):
            total_chars += len(row[0] or "")
        conn.close()
    except Exception:
        pass

    return total_chars // 4  # ~4 chars per token


def run_benchmark():
    print("=" * 70)
    print("CORTEX RECALL BENCHMARK v2 (keyword + semantic + ground truth)")
    print("=" * 70)
    check_daemon_health()

    # Check if embeddings exist in the DB directly (no /embed endpoint needed)
    all_embeddings = load_all_embeddings()
    has_embed = len(all_embeddings) > 0
    if has_embed:
        print(f"  Embeddings: {len(all_embeddings)} loaded from DB (cosine scoring enabled)")
    else:
        print("  Embeddings: none found in DB (keyword + ground truth only)")

    baseline_tokens = naive_baseline_tokens()
    print(f"  Naive baseline: {baseline_tokens:,} tokens (full dump)")
    print()

    results = []

    for i, q in enumerate(QUERIES):
        query = q["q"]
        category = q["category"]
        ground_truth = q["ground_truth"]

        cortex = cortex_recall(query)

        # Score each result
        scored_results = []
        for r in cortex["results"]:
            excerpt = r.get("excerpt", "")
            source = r.get("source", "")

            kw_score = score_keyword(query, excerpt, source)
            gt_match = score_ground_truth(ground_truth, excerpt, source)

            scored_results.append({
                "source": source[:80],
                "excerpt_preview": excerpt[:100],
                "keyword_score": round(kw_score, 2),
                "ground_truth_match": gt_match,
                "cortex_relevance": r.get("relevance", 0),
                "method": r.get("method", "?"),
                "tokens": r.get("tokens", 0),
            })

        n_results = len(scored_results)
        gt_relevant = sum(1 for s in scored_results if s["ground_truth_match"])
        kw_relevant = sum(1 for s in scored_results if s["keyword_score"] >= 0.5)

        gt_precision = gt_relevant / n_results if n_results else 0
        kw_precision = kw_relevant / n_results if n_results else 0

        # MRR: position of first ground-truth match
        mrr = 0
        for j, s in enumerate(scored_results):
            if s["ground_truth_match"]:
                mrr = 1 / (j + 1)
                break

        result = {
            "query": query,
            "category": category,
            "result_count": n_results,
            "cortex_tokens": cortex.get("token_estimate", 0),
            "response_ms": cortex["elapsed_ms"],
            "gt_relevant": gt_relevant,
            "gt_precision": round(gt_precision, 3),
            "kw_precision": round(kw_precision, 3),
            "mrr": round(mrr, 3),
            "scored_results": scored_results,
        }
        results.append(result)

        gt_pct = f"{gt_precision:.0%}"
        print(f"  [{i+1:2d}/20] {query[:40]:<40} | "
              f"{n_results} res | "
              f"GT={gt_relevant}/{n_results} ({gt_pct}) | "
              f"MRR={mrr:.2f} | "
              f"{cortex['elapsed_ms']:.0f}ms | "
              f"{cortex.get('token_estimate',0)} tok")

    # Aggregates
    print("\n" + "=" * 70)
    print("AGGREGATE RESULTS")
    print("=" * 70)

    total_cortex = sum(r["cortex_tokens"] for r in results)
    avg_gt_p = sum(r["gt_precision"] for r in results) / len(results)
    avg_kw_p = sum(r["kw_precision"] for r in results) / len(results)
    avg_mrr = sum(r["mrr"] for r in results) / len(results)
    avg_ms = sum(r["response_ms"] for r in results) / len(results)
    avg_results = sum(r["result_count"] for r in results) / len(results)
    queries_with_hit = sum(1 for r in results if r["gt_relevant"] > 0)

    print(f"  Ground truth precision:  {avg_gt_p:.0%}")
    print(f"  Keyword precision:       {avg_kw_p:.0%}")
    print(f"  Mean Reciprocal Rank:    {avg_mrr:.2f}")
    print(f"  Hit rate (>=1 relevant): {queries_with_hit}/{len(results)} ({queries_with_hit/len(results):.0%})")
    print(f"  Avg response time:       {avg_ms:.1f}ms")
    print(f"  Avg results per query:   {avg_results:.1f}")
    print(f"  Total Cortex tokens:     {total_cortex:,}")
    print(f"  Naive baseline tokens:   {baseline_tokens:,}")
    print(f"  Token efficiency:        {total_cortex/baseline_tokens:.1%} of baseline")
    print(f"  Token savings:           {(1-total_cortex/baseline_tokens):.0%}")

    # By category
    print("\n  BY CATEGORY:")
    cats = {}
    for r in results:
        c = r["category"]
        if c not in cats:
            cats[c] = {"n": 0, "gt_p": 0, "mrr": 0, "ms": 0, "tok": 0}
        cats[c]["n"] += 1
        cats[c]["gt_p"] += r["gt_precision"]
        cats[c]["mrr"] += r["mrr"]
        cats[c]["ms"] += r["response_ms"]
        cats[c]["tok"] += r["cortex_tokens"]

    for cat, s in cats.items():
        n = s["n"]
        print(f"    {cat:<20} | GT_P={s['gt_p']/n:.0%} | MRR={s['mrr']/n:.2f} | "
              f"{s['ms']/n:.0f}ms | {s['tok']/n:.0f} tok")

    # Save
    out_path = Path(__file__).parent / "recall-benchmark-v2-results.json"
    output = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "version": 2,
        "queries": len(QUERIES),
        "has_embeddings": has_embed,
        "aggregate": {
            "ground_truth_precision": round(avg_gt_p, 3),
            "keyword_precision": round(avg_kw_p, 3),
            "mean_reciprocal_rank": round(avg_mrr, 3),
            "hit_rate": round(queries_with_hit / len(results), 3),
            "avg_response_ms": round(avg_ms, 1),
            "avg_results_per_query": round(avg_results, 1),
            "total_cortex_tokens": total_cortex,
            "naive_baseline_tokens": baseline_tokens,
            "token_savings_pct": round((1 - total_cortex / baseline_tokens) * 100, 1),
        },
        "category_stats": {
            cat: {
                "queries": s["n"],
                "gt_precision": round(s["gt_p"] / s["n"], 3),
                "mrr": round(s["mrr"] / s["n"], 3),
                "avg_ms": round(s["ms"] / s["n"], 1),
                "avg_tokens": round(s["tok"] / s["n"], 1),
            }
            for cat, s in cats.items()
        },
        "results": results,
    }
    out_path.write_text(json.dumps(output, indent=2))
    print(f"\n  Results saved to: {out_path}")

    # Boot benchmark
    print("\n" + "=" * 70)
    print("BOOT BENCHMARK")
    print("=" * 70)
    try:
        url = f"{CORTEX_URL}/boot?agent=claude-opus&budget=600"
        start = time.perf_counter()
        data = request_json(url, timeout=5)
        boot_ms = (time.perf_counter() - start) * 1000
        savings = data.get("savings", {})
        print(f"  Boot tokens:    {data.get('tokenEstimate', 0)}")
        print(f"  Raw baseline:   {savings.get('rawBaseline', 0):,}")
        print(f"  Compression:    {savings.get('percent', 0)}%")
        print(f"  Boot time:      {boot_ms:.0f}ms")
        print(f"  Capsules:       {len(data.get('capsules', []))}")
        for c in data.get("capsules", []):
            print(f"    - {c.get('name', '?')} ({c.get('tokens', 0)} tok)")
    except Exception as e:
        print(f"  Boot failed: {e}")


if __name__ == "__main__":
    run_benchmark()

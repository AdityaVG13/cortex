#!/usr/bin/env python3
"""Run a fixed-rate recall load probe against a Cortex daemon.

The probe is intentionally small and dependency-free so release evidence can be
reproduced on a clean machine with only Python installed.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import statistics
import time
import urllib.error
import urllib.request
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path


def percentile(values: list[float], pct: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int(round((pct / 100.0) * (len(ordered) - 1)))))
    return ordered[index]


def post_recall(base_url: str, token: str, index: int, timeout: float, recall_budget: int) -> dict:
    body = json.dumps(
        {
            "q": f"c3 budget load probe {index}",
            "k": 1,
            "budget": recall_budget,
            "agent": "c3-budget-load-probe",
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}/recall",
        data=body,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "X-Cortex-Request": "true",
            "User-Agent": "c3-budget-load-probe/1",
        },
        method="POST",
    )
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            payload = response.read(4096).decode("utf-8", errors="replace")
            status = response.getcode()
    except urllib.error.HTTPError as error:
        payload = error.read(4096).decode("utf-8", errors="replace")
        status = error.code
    except Exception as error:  # noqa: BLE001 - evidence needs the failure type.
        elapsed_ms = (time.perf_counter() - started) * 1000.0
        return {
            "index": index,
            "status": "transport_error",
            "latency_ms": elapsed_ms,
            "error": type(error).__name__,
            "message": str(error),
        }
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    return {
        "index": index,
        "status": status,
        "latency_ms": elapsed_ms,
        "body_prefix": payload[:300],
    }


def run_probe(args: argparse.Namespace) -> dict:
    interval = 1.0 / args.rate
    started_wall = datetime.now(timezone.utc)
    started = time.perf_counter()
    results: list[dict] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as executor:
        futures = []
        for index in range(args.requests):
            target = started + (index * interval)
            delay = target - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
            futures.append(
                executor.submit(
                    post_recall,
                    args.base_url,
                    args.token,
                    index,
                    args.timeout,
                    args.recall_budget,
                )
            )
        for future in concurrent.futures.as_completed(futures):
            results.append(future.result())
    ended = time.perf_counter()

    status_counts = Counter(str(item["status"]) for item in results)
    latencies = [float(item["latency_ms"]) for item in results]
    accepted = [float(item["latency_ms"]) for item in results if item["status"] == 200]
    denied = [float(item["latency_ms"]) for item in results if item["status"] == 429]
    failures = [
        {
            "index": item["index"],
            "status": item["status"],
            "latency_ms": round(float(item["latency_ms"]), 3),
            "body_prefix": item.get("body_prefix", item.get("message", ""))[:300],
        }
        for item in sorted(results, key=lambda row: row["index"])
        if item["status"] not in (200, 429)
    ][:10]

    summary = {
        "schema": "cortex.c3_budget_load_probe.v1",
        "started_at": started_wall.isoformat(),
        "base_url": args.base_url,
        "requests": args.requests,
        "rate_per_second": args.rate,
        "workers": args.workers,
        "recall_budget": args.recall_budget,
        "duration_seconds": round(ended - started, 3),
        "expected": {
            "status_200": args.expect_ok,
            "status_429": args.expect_denied,
            "max_p95_all_ms": args.max_p95_all_ms,
        },
        "status_counts": dict(sorted(status_counts.items())),
        "latency_ms": {
            "min": round(min(latencies), 3) if latencies else None,
            "median": round(statistics.median(latencies), 3) if latencies else None,
            "p95_all": round(percentile(latencies, 95) or 0.0, 3) if latencies else None,
            "p95_accepted": round(percentile(accepted, 95) or 0.0, 3) if accepted else None,
            "p95_denied": round(percentile(denied, 95) or 0.0, 3) if denied else None,
            "max": round(max(latencies), 3) if latencies else None,
        },
        "sample_unexpected": failures,
    }

    pass_checks = [
        status_counts.get("200", 0) == args.expect_ok,
        status_counts.get("429", 0) == args.expect_denied,
        status_counts.get("transport_error", 0) == 0,
    ]
    if args.max_p95_all_ms is not None and summary["latency_ms"]["p95_all"] is not None:
        pass_checks.append(summary["latency_ms"]["p95_all"] <= args.max_p95_all_ms)
    summary["passed"] = all(pass_checks)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--token", required=True)
    parser.add_argument("--requests", type=int, default=100)
    parser.add_argument("--rate", type=float, default=100.0)
    parser.add_argument("--workers", type=int, default=64)
    parser.add_argument("--timeout", type=float, default=10.0)
    parser.add_argument("--recall-budget", type=int, default=0)
    parser.add_argument("--expect-ok", type=int, default=60)
    parser.add_argument("--expect-denied", type=int, default=40)
    parser.add_argument("--max-p95-all-ms", type=float, default=None)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    summary = run_probe(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

# RQ1 Embedding Profile Gate

Generated: 2026-05-05T14:11:13.161313+00:00

Result: **PASS**

This local gate covers BGE backfill throughput and p50 recall latency versus the
legacy MiniLM profile. It does not replace scored Pure LongMemEval-S.

## Checks

- Backfill throughput: 102352.11 emb/hr (required >= 500.0)
- Backfill rows built: 64 / 64
- Recall p50 delta: 8.228 ms (allowed <= 10.0 ms)
- LongMemEval-S: blocked_no_provider_key

## Mode Summary

| Profile | p50 ms | p95 ms | top3 |
|---------|--------|--------|------|
| all-MiniLM-L12-v2 | 58.485 | 83.789 | 1.0 |
| bge-base-en-v1.5 | 66.713 | 82.007 | 1.0 |

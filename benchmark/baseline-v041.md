# Cortex v0.4.1 Recall Baseline
**Date:** 2026-04-06T18:39:53
**Nodes:** 544 (271 memories, 273 decisions)
**Embeddings:** MiniLM-L6 384-dim, has_embeddings=false (health embeddings=562, status=available)

## Aggregate Metrics
| Metric | Value | Source |
|--------|-------|--------|
| Ground Truth Precision | 0.552 | benchmark-v2 |
| Keyword Precision | 0.335 | benchmark-v2 |
| MRR | 0.692 | benchmark-v2 |
| Hit Rate | 0.900 | benchmark-v2 |
| Avg Latency (ms) | 97.5 | benchmark-v2 |
| Avg Recall | 0.101 | metric |
| Avg Precision | 0.597 | metric |
| Avg F1 | 0.153 | metric |
| Macro Recall | 0.083 | metric |
| Total Relevant in DB | 1842 | metric |

## By Category (benchmark-v2)
| Category | Queries | GT Precision | MRR | Avg ms | Avg Tokens |
|----------|---------|-------------|-----|--------|------------|
| project_decisions | 4 | 0.400 | 0.583 | 104.3 | 324.2 |
| feedback_rules | 4 | 0.595 | 0.750 | 87.2 | 249.8 |
| cross_agent | 4 | 0.475 | 0.750 | 98.5 | 136.0 |
| architecture | 4 | 0.291 | 0.375 | 99.9 | 134.5 |
| user_context | 4 | 1.000 | 1.000 | 97.8 | 177.8 |

## Worst Queries (GT precision < 0.40)
| Query | GT Precision | MRR | Category | Failure Mode |
|-------|-------------|-----|----------|-------------|
| cache expiry guard hook | 0.333 | 1.000 | project_decisions | GIGO |
| RTK path fix bashrc | 0.200 | 0.500 | project_decisions | RANKING |
| never use em-dashes | 0.143 | 0.500 | feedback_rules | RANKING |
| multi-agent shared state | 0.000 | 0.000 | cross_agent | SPARSE |
| conflict detection jaccard cosine | 0.333 | 0.500 | architecture | RANKING |
| embedding engine MiniLM | 0.000 | 0.000 | architecture | SPARSE |
| crystal cluster formation | 0.333 | 0.500 | architecture | RANKING |

## Delta from 2026-04-05 Run
| Metric | 2026-04-05 | Today | Delta |
|--------|-----------|-------|-------|
| GT Precision | 0.587 | 0.552 | -0.035 |
| MRR | 0.742 | 0.692 | -0.050 |
| Avg Latency | 105.5ms | 97.5ms | -8.0ms |

## Notes
- Health reported embedding_status=available and 562 embeddings, but benchmark-v2 recorded has_embeddings=false because the /embed probe did not return a vector. Recall still ran through the current /recall endpoint.
- GT precision dropped by 0.035 (-6.0% relative) and MRR dropped by 0.050 (-6.7% relative) from the 2026-04-05 run, while average latency improved by 8.0ms (-7.6%).
- The node count increased from the prompt context to 544 active memory/decision nodes at measurement time, which likely contributes to changed precision and recall totals.
- `cache expiry guard hook` is labeled GIGO even though the top two results were relevant; the precision loss comes from irrelevant tail results after the relevant hits.

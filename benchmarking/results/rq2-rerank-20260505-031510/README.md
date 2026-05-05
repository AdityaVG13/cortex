# RQ2 Rerank Gate

Generated: 2026-05-05T03:15:19

This is the local deterministic RQ2 gate. It proves model-load/runtime behavior,
off/shadow/primary telemetry, latency, and a Cortex-owned regression guard.

Release posture: **CAUTION**

Reason: Local model smoke and owned regression artifacts can support shadow/experimental posture, but scored LongMemEval-S is still required before a public primary rerank claim.

Important limitation: this run does **not** replace scored Pure LongMemEval-S.
Do not make a public primary rerank quality claim until LongMemEval-S is run
and passes the Phase 2 gate.

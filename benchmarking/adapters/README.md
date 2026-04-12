# Adapters

Tracked adapter code for running external benchmark harnesses against Cortex belongs here.

Planned first adapter:

- `cortex_amb_adapter.py`: AMB-compatible provider that maps benchmark ingest/retrieve calls onto Cortex HTTP endpoints.

Follow-on runners can live here too if LongMemEval or LoCoMo need thin wrapper scripts instead of a reusable provider.

# Adapters

Tracked adapter code for running external benchmark harnesses against Cortex belongs here.

Current adapters:

- `cortex_amb_provider.py`: AMB-compatible provider that maps benchmark ingest/retrieve calls onto Cortex HTTP endpoints.
- `cortex_function_adapter.py`: OpenAI function-call contract adapter that maps function payloads to `/health`, `/store`, and `/recall`.

Follow-on runners can live here too if LongMemEval or LoCoMo need thin wrapper scripts instead of a reusable provider.

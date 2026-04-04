# Local LLM (OpenAI-compatible) + Cortex

This example runs a minimal tool loop against any OpenAI-compatible local endpoint
(for example, LM Studio, vLLM, llama.cpp server, or LocalAI) and calls Cortex over HTTP.

## Setup

```bash
pip install openai httpx
pip install -e ../../sdks/python
```

Set environment variables:

```bash
export OPENAI_COMPAT_BASE_URL="http://127.0.0.1:1234/v1"
export OPENAI_COMPAT_MODEL="your-model-name"
export OPENAI_API_KEY="not-used-by-most-local-servers"
```

Start Cortex:

```bash
cortex serve
```

## Run

```bash
python ./tool_loop.py "What did we decide about release packaging?"
```

The script lets the model call two tools:

- `cortex_recall(query, budget, k)`
- `cortex_store(decision, context)`

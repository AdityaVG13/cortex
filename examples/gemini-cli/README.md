# Gemini CLI + Cortex

This example shows a lightweight flow for using Cortex with Gemini CLI through HTTP.

## Prerequisites

- Cortex daemon running locally: `cortex serve`
- A valid token at `~/.cortex/cortex.token`
- `curl` and `python` available on PATH

## 1) Boot context before each session

```bash
TOKEN="$(cat ~/.cortex/cortex.token)"
curl -sS \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "X-Cortex-Request: true" \
  "http://127.0.0.1:7437/boot?agent=gemini-cli&budget=600"
```

Use the returned `bootPrompt` as session context.

## 2) Inject prompt file with `cortex prompt-inject`

Keep your base Gemini system prompt in a file, then let Cortex append fresh context:

```bash
cortex prompt-inject --file ./gemini.system.md --agent gemini-cli --budget 600
```

This writes `./gemini.system.injected`.

## 3) Recall and store during a session

```bash
# Recall
curl -sS \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "X-Cortex-Request: true" \
  "http://127.0.0.1:7437/recall?q=release+checklist&k=8&budget=240"

# Store
curl -sS -X POST \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "X-Cortex-Request: true" \
  -H "Content-Type: application/json" \
  "http://127.0.0.1:7437/store" \
  -d '{"decision":"Ship v0.2.0 with team migration and OpenAPI spec","context":"release","source_agent":"gemini-cli"}'
```

## Optional hook

Use `session-start-hook.sh` in this folder as a bootstrap helper that fetches boot context and writes it to `./gemini.boot.json`.

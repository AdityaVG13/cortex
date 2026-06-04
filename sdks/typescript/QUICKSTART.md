# Cortex TypeScript SDK Quickstart

Private local memory first:

```ts
import { CortexClient } from "@cortex-memory/client";

const cortex = new CortexClient({ sourceAgent: "my-tool" });

await cortex.store("Use the status command before first attach", {
  context: "Onboarding smoke for a local Cortex install",
});

const memories = await cortex.recall("first attach status command", {
  k: 3,
  budget: 200,
});

console.log(cortex.formatRecallContext(memories, { maxItems: 3 }));
```

Before running the snippet:

```bash
cortex status --json
```

Success is `"status": "ready"`. If the status is `needs_action` or `error`, follow the returned `nextAction` / `repair` before using the SDK.

Notes:

- Local default: `http://127.0.0.1:7437`.
- Local auth token is read from `~/.cortex/cortex.token`.
- Remote or team URLs require an explicit `token`; the SDK will not silently reuse a local token for non-loopback targets.

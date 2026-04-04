import json
import os
import sys

from cortex_memory import CortexClient
from openai import OpenAI


TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "cortex_recall",
            "description": "Recall memories/decisions from Cortex.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "budget": {"type": "integer", "default": 200},
                    "k": {"type": "integer", "default": 8},
                },
                "required": ["query"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "cortex_store",
            "description": "Store a decision in Cortex.",
            "parameters": {
                "type": "object",
                "properties": {
                    "decision": {"type": "string"},
                    "context": {"type": "string"},
                },
                "required": ["decision"],
            },
        },
    },
]


def parse_args(raw: str) -> dict:
    if not raw:
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return {}


def tool_call(cortex: CortexClient, name: str, args: dict) -> dict:
    if name == "cortex_recall":
        return cortex.recall(
            query=args.get("query", ""),
            budget=int(args.get("budget", 200)),
            k=int(args.get("k", 8)),
            agent="local-llm",
        )
    if name == "cortex_store":
        return cortex.store(
            text=args.get("decision", ""),
            source="local-llm",
            source_agent="local-llm",
            context=args.get("context", "tool-loop"),
        )
    return {"error": f"unknown tool: {name}"}


def main() -> int:
    if len(sys.argv) < 2:
        print('usage: python tool_loop.py "<prompt>"')
        return 1

    prompt = sys.argv[1]
    client = OpenAI(
        base_url=os.getenv("OPENAI_COMPAT_BASE_URL", "http://127.0.0.1:1234/v1"),
        api_key=os.getenv("OPENAI_API_KEY", "local-key"),
    )
    model = os.getenv("OPENAI_COMPAT_MODEL", "local-model")
    cortex = CortexClient()

    messages = [
        {
            "role": "system",
            "content": (
                "You can call Cortex tools for memory recall and storage. "
                "Use tools when helpful, then provide a concise final answer."
            ),
        },
        {"role": "user", "content": prompt},
    ]

    for _ in range(6):
        resp = client.chat.completions.create(
            model=model,
            messages=messages,
            tools=TOOLS,
            tool_choice="auto",
        )
        msg = resp.choices[0].message
        tool_calls = msg.tool_calls or []

        assistant_message = {"role": "assistant", "content": msg.content or ""}
        if tool_calls:
            assistant_message["tool_calls"] = [
                {
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.function.name,
                        "arguments": call.function.arguments or "{}",
                    },
                }
                for call in tool_calls
            ]
        messages.append(assistant_message)

        if not tool_calls:
            print(msg.content or "")
            return 0

        for call in tool_calls:
            args = parse_args(call.function.arguments or "{}")
            result = tool_call(cortex, call.function.name, args)
            messages.append(
                {
                    "role": "tool",
                    "tool_call_id": call.id,
                    "name": call.function.name,
                    "content": json.dumps(result),
                }
            )

    print("No final response produced within max tool iterations.")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

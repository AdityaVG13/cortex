"""Cortex HTTP client — shared by all Python workers."""

import json
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import URLError
from urllib.parse import urlencode, quote

BASE_URL = "http://localhost:7437"
TOKEN_PATH = Path.home() / ".cortex" / "cortex.token"


def _read_token() -> str | None:
    try:
        return TOKEN_PATH.read_text().strip()
    except FileNotFoundError:
        return None


def _get(path: str, params: dict | None = None) -> dict:
    url = f"{BASE_URL}{path}"
    if params:
        url += "?" + urlencode(params)
    req = Request(url)
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def _post(path: str, body: dict) -> dict:
    token = _read_token()
    data = json.dumps(body).encode()
    req = Request(f"{BASE_URL}{path}", data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def boot(agent_id: str = "worker") -> dict:
    return _get("/boot", {"agent": agent_id})


def recall(query: str, k: int = 7) -> list:
    result = _get("/recall", {"q": query, "k": str(k)})
    return result.get("results", [])


def store(decision: str, context: str | None = None, agent: str = "worker") -> dict:
    body = {"decision": decision, "source_agent": agent}
    if context:
        body["context"] = context
    return _post("/store", body)


def dump() -> dict:
    token = _read_token()
    req = Request(f"{BASE_URL}/dump")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def archive(entry_type: str, ids: list[int]) -> dict:
    return _post("/archive", {"type": entry_type, "ids": ids})


def digest() -> dict:
    return _get("/digest")


def health() -> dict:
    return _get("/health")


if __name__ == "__main__":
    try:
        h = health()
        s = h.get("stats", {})
        print(f"Cortex: {h.get('status', 'unknown')}")
        print(f"  Memories:   {s.get('memories', '?')}")
        print(f"  Decisions:  {s.get('decisions', '?')}")
        print(f"  Embeddings: {s.get('embeddings', '?')}")
        print(f"  Ollama:     {s.get('ollama', '?')}")

        d = digest()
        ts = d.get("tokenSavings", {}).get("allTime", {})
        if ts.get("saved", 0) > 0:
            print(f"  Tokens saved: {ts['saved']:,} across {ts['boots']} boots")
    except URLError as e:
        print(f"Cannot connect to Cortex at {BASE_URL}: {e.reason}")

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
    token = _read_token()
    req = Request(url)
    if token:
        req.add_header("Authorization", f"Bearer {token}")
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
    return _post("/archive", {"table": entry_type, "ids": ids})


def digest() -> dict:
    return _get("/digest")


def health() -> dict:
    return _get("/health")


# ─── Conductor & Session endpoints ──────────────────────────────────────


def get_locks() -> dict:
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    req = Request(f"{BASE_URL}/locks")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def get_activity(since: str = "1h") -> dict:
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    req = Request(f"{BASE_URL}/activity?since={since}")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def get_sessions() -> dict:
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    req = Request(f"{BASE_URL}/sessions")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def post_activity(agent: str, description: str, files: list[str] | None = None) -> dict:
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    body = {"agent": agent, "description": description, "files": files or []}
    req = Request(f"{BASE_URL}/activity", data=json.dumps(body).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


# ─── Task Board endpoints ───────────────────────────────────────────────


def get_tasks(status: str = "pending") -> dict:
    """Get tasks from the task board. status: pending|claimed|completed|all"""
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    req = Request(f"{BASE_URL}/tasks?status={status}")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def get_next_task(agent: str, capability: str = "any") -> dict | None:
    """Get the highest priority task for this agent."""
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    req = Request(f"{BASE_URL}/tasks/next?agent={agent}&capability={capability}")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def create_task(
    title: str,
    description: str = "",
    project: str = "cortex",
    files: list[str] | None = None,
    priority: str = "medium",
    required_capability: str = "any",
) -> dict:
    """Create a new task on the board."""
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    body = {
        "title": title,
        "description": description,
        "project": project,
        "files": files or [],
        "priority": priority,
        "requiredCapability": required_capability,
    }
    req = Request(f"{BASE_URL}/tasks", data=json.dumps(body).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def claim_task(task_id: str, agent: str) -> dict:
    """Claim a task for the given agent."""
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    body = {"taskId": task_id, "agent": agent}
    req = Request(f"{BASE_URL}/tasks/claim", data=json.dumps(body).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def complete_task(task_id: str, agent: str, summary: str = "") -> dict:
    """Mark a task as completed."""
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    body = {"taskId": task_id, "agent": agent, "summary": summary}
    req = Request(f"{BASE_URL}/tasks/complete", data=json.dumps(body).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def abandon_task(task_id: str, agent: str) -> dict:
    """Return a claimed task to pending."""
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    body = {"taskId": task_id, "agent": agent}
    req = Request(f"{BASE_URL}/tasks/abandon", data=json.dumps(body).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


# ─── Inter-Agent Messaging ─────────────────────────────────────────────


def send_message(from_agent: str, to_agent: str, message: str) -> dict:
    """Send a message to another agent."""
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    body = {"from": from_agent, "to": to_agent, "message": message}
    req = Request(f"{BASE_URL}/message", data=json.dumps(body).encode(), method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def get_messages(agent: str) -> dict:
    """Get messages for a specific agent."""
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")
    req = Request(f"{BASE_URL}/messages?agent={agent}")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


def get_feed(
    since: str = "1h",
    agent: str | None = None,
    kind: str | None = None,
    unread: bool | None = None,
) -> dict:
    """Get shared feed entries."""
    token = _read_token()
    if not token:
        raise RuntimeError("No auth token found")

    params: dict[str, str] = {"since": since}
    if agent:
        params["agent"] = agent
    if kind and kind != "all":
        params["kind"] = kind
    if unread is not None:
        params["unread"] = "true" if unread else "false"

    req = Request(f"{BASE_URL}/feed?{urlencode(params)}")
    req.add_header("Authorization", f"Bearer {token}")
    with urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


if __name__ == "__main__":
    try:
        h = health()
        s = h.get("stats", {})
        print(f"Cortex: {h.get('status', 'unknown')}")
        print(f"  Memories:   {s.get('memories', '?')}")
        print(f"  Decisions:  {s.get('decisions', '?')}")
        print(f"  Embeddings: {s.get('embeddings', '?')}")

        d = digest()
        ts = d.get("tokenSavings", {}).get("allTime", {})
        if ts.get("saved", 0) > 0:
            print(f"  Tokens saved: {ts['saved']:,} across {ts['boots']} boots")
    except URLError as e:
        print(f"Cannot connect to Cortex at {BASE_URL}: {e.reason}")

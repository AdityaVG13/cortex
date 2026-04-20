import json
import os
import time
from pathlib import Path
from urllib.parse import urlencode, urlparse, urlunparse


def env(name, default=""):
    value = os.environ.get(name)
    return value if value is not None else default


LABEL = env("CORTEX_PROBE_LABEL", "runtime")
BASE_URL = env("CORTEX_PROBE_URL", "http://localhost:1420/")
CORTEX_BASE = env("CORTEX_PROBE_CORTEX_BASE", "http://127.0.0.1:7437")
AUTH_TOKEN = env("CORTEX_PROBE_AUTH_TOKEN", "")
OUT_DIR = Path(env("CORTEX_PROBE_OUT_DIR", "runtime-artifacts")).resolve()


def with_bootstrap(base_url, cortex_base, auth_token):
    from urllib.parse import urlencode, urlparse, urlunparse

    parsed = urlparse(base_url)
    query = {}
    query["panel"] = "overview"
    query["cortexBase"] = cortex_base
    if auth_token:
        query["authToken"] = auth_token
    return urlunparse(parsed._replace(query=urlencode(query)))


def detect_auth_token(max_wait_seconds=8.0):
    token_probe_js = r"""
(() => {
  const storages = [sessionStorage, localStorage];
  for (const storage of storages) {
    const direct = storage.getItem("cortex_auth_token");
    if (typeof direct === "string" && direct.trim()) return direct.trim();
  }
  for (const storage of storages) {
    for (const key of Object.keys(storage)) {
      const raw = storage.getItem(key);
      if (typeof raw !== "string") continue;
      let value = raw.trim();
      if (!value) continue;
      try {
        const parsed = JSON.parse(value);
        if (typeof parsed === "string" && parsed.trim()) {
          value = parsed.trim();
        }
      } catch {}
      if (/^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(value)) {
        return value;
      }
      if (/^[a-f0-9]{16,}$/i.test(value)) {
        return value;
      }
    }
  }
  return "";
})()
"""
    deadline = time.time() + max(0.0, float(max_wait_seconds))
    while time.time() <= deadline:
        token = js(token_probe_js)
        if isinstance(token, str) and token.strip():
            return token.strip()
        time.sleep(0.25)
    return ""


def close_existing_program_tabs(base_url):
    import urllib.parse as _urlparse

    def _signature(url):
        parsed = _urlparse.urlparse(url or "")
        return (
            (parsed.scheme or "").lower(),
            (parsed.netloc or "").lower(),
            parsed.path or "/",
        )

    base_signature = _signature(base_url)
    closed = 0
    for tab in list_tabs(include_chrome=False):
        tab_url = tab.get("url") or ""
        target_id = tab.get("targetId")
        if not target_id:
            continue
        if _signature(tab_url) != base_signature:
            continue
        try:
            cdp("Target.closeTarget", targetId=target_id)
            closed += 1
        except Exception:
            pass
    return closed


def install_perf_probe():
    js(
        """
(() => {
  if (window.__cortexPerfProbeInstalled) return true;
  const probe = { fcpMs: null, lcpMs: null, cls: 0, inpMs: null };
  window.__cortexPerfProbe = probe;
  window.__cortexPerfProbeInstalled = true;
  try {
    const paintObs = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (entry.name === "first-contentful-paint") {
          probe.fcpMs = entry.startTime;
        }
      }
    });
    paintObs.observe({ type: "paint", buffered: true });
  } catch {}
  try {
    const lcpObs = new PerformanceObserver((list) => {
      const entries = list.getEntries();
      const last = entries[entries.length - 1];
      if (last) probe.lcpMs = last.startTime;
    });
    lcpObs.observe({ type: "largest-contentful-paint", buffered: true });
  } catch {}
  try {
    const clsObs = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (!entry.hadRecentInput) probe.cls += entry.value;
      }
    });
    clsObs.observe({ type: "layout-shift", buffered: true });
  } catch {}
  try {
    const inpObs = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        const interactionId = Number(entry.interactionId || 0);
        if (interactionId > 0) {
          probe.inpMs = Math.max(probe.inpMs || 0, entry.duration || 0);
        }
      }
    });
    inpObs.observe({ type: "event", buffered: true, durationThreshold: 16 });
  } catch {}
  return true;
})()
"""
    )


def panel_focus():
    return js(
        """
(() => {
  const n = document.querySelector(".sidebar-utility-note strong");
  return n ? n.textContent.trim() : "";
})()
"""
    )


def click_panel(label, panel_focus_fn=panel_focus):
    import json as _json
    import time as _time

    clicked = js(
        f"""
(() => {{
  const target = [...document.querySelectorAll(".sidebar-nav .nav-item")]
    .find((node) => node.textContent && node.textContent.trim().includes({_json.dumps(label)}));
  if (!target) return false;
  target.click();
  return true;
}})()
"""
    )
    if not clicked:
        raise RuntimeError(f"panel button not found: {label}")

    start = _time.perf_counter()
    deadline = _time.time() + 20
    while _time.time() < deadline:
        if panel_focus_fn() == label:
            return round((_time.perf_counter() - start) * 1000, 2)
        _time.sleep(0.05)
    raise RuntimeError(f"panel did not become active: {label}")


def screenshot_memory_conflicts(path):
    found = js(
        """
(() => {
  const heading = [...document.querySelectorAll("h2")]
    .find((node) => node.textContent && node.textContent.trim().includes("Conflict Resolution"));
  if (!heading) return false;
  heading.scrollIntoView({ block: "start", behavior: "instant" });
  return true;
})()
"""
    )
    if not found:
        return False
    time.sleep(0.5)
    screenshot(path, full=True)
    return True


def web_vitals():
    return js(
        """
(() => {
  const nav = performance.getEntriesByType("navigation")[0];
  const probe = window.__cortexPerfProbe || {};
  const f = (value, digits) => (typeof value === "number" ? Number(value.toFixed(digits)) : null);
  return {
    ttfbMs: nav ? f(nav.responseStart, 2) : null,
    domContentLoadedMs: nav ? f(nav.domContentLoadedEventEnd, 2) : null,
    loadEventMs: nav ? f(nav.loadEventEnd, 2) : null,
    fcpMs: f(probe.fcpMs, 2),
    lcpMs: f(probe.lcpMs, 2),
    cls: f(probe.cls, 4),
    inpMs: f(probe.inpMs, 2),
  };
})()
"""
    )


def recall_smoke(cortex_base, auth_token, source_agent):
    import json as _json

    return js(
        f"""
(async () => {{
  const headers = {{
    "X-Cortex-Request": "true",
    "X-Source-Agent": {_json.dumps(source_agent)},
  }};
  const token = {_json.dumps(auth_token)};
  if (token) {{
    headers["Authorization"] = "Bearer " + token;
  }}
  async function timedJson(url) {{
    const started = performance.now();
    const response = await fetch(url, {{ headers }});
    const elapsedMs = Number((performance.now() - started).toFixed(2));
    let body = null;
    try {{ body = await response.json(); }} catch {{}}
    return {{ ok: response.ok, status: response.status, elapsedMs, body }};
  }}
  const peek = await timedJson(
    {_json.dumps(cortex_base)} + "/peek?q=" + encodeURIComponent("daemon startup lock lease") + "&k=8"
  );
  const recall = await timedJson(
    {_json.dumps(cortex_base)} + "/recall?q=" + encodeURIComponent("daemon startup lock lease") + "&k=6&budget=360"
  );
  const topSources = Array.isArray(recall.body?.results)
    ? recall.body.results.slice(0, 3).map((item) => String(item?.source || ""))
    : [];
  return {{
    peek: {{
      ok: peek.ok,
      status: peek.status,
      elapsedMs: peek.elapsedMs,
      count: Number(peek.body?.count || 0),
    }},
    recall: {{
      ok: recall.ok,
      status: recall.status,
      elapsedMs: recall.elapsedMs,
      count: Array.isArray(recall.body?.results) ? recall.body.results.length : 0,
      topSources,
    }},
  }};
}})()
"""
    )


run_id = f"{LABEL}-{time.strftime('%Y-%m-%dT%H-%M-%S')}"
run_dir = OUT_DIR / run_id
run_dir.mkdir(parents=True, exist_ok=True)

install_perf_probe()
boot_url = with_bootstrap(BASE_URL, CORTEX_BASE, AUTH_TOKEN)
closed_preexisting = close_existing_program_tabs(BASE_URL)
created_tab_id = None

try:
    created_tab_id = new_tab(boot_url)
    wait_for_load(20)
    time.sleep(1.2)
    runtime_auth_token = AUTH_TOKEN or detect_auth_token()

    switch_timings = {}
    tabs = ["Overview", "Analytics", "Agents", "Work", "Memory", "Brain", "About"]
    memory_conflicts_shot = run_dir / "memory-conflicts.png"
    memory_conflicts_captured = False
    for tab in tabs:
        switch_timings[tab.lower()] = click_panel(tab)
        time.sleep(3.5 if tab == "Brain" else 0.9)
        screenshot(str(run_dir / f"{tab.lower()}.png"), full=True)
        if tab == "Memory":
            memory_conflicts_captured = screenshot_memory_conflicts(str(memory_conflicts_shot))

    switch_timings["brainToOverview"] = click_panel("Overview")
    time.sleep(1.2)
    screenshot(str(run_dir / "overview-return.png"), full=True)

    report = {
        "label": LABEL,
        "capturedAt": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "url": BASE_URL,
        "cortexBase": CORTEX_BASE,
        "closedPreexistingProgramTabs": closed_preexisting,
        "authTokenDetected": bool(runtime_auth_token),
        "memoryConflictsCaptured": memory_conflicts_captured,
        "switchTimingsMs": switch_timings,
        "webVitals": web_vitals(),
        "recallSmoke": recall_smoke(CORTEX_BASE, runtime_auth_token, f"browser-harness-{run_id}"),
        "screenshots": [str(run_dir / f"{tab.lower()}.png") for tab in tabs]
        + ([str(memory_conflicts_shot)] if memory_conflicts_captured else [])
        + [str(run_dir / "overview-return.png")],
    }

    report_path = run_dir / "metrics.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"runDir": str(run_dir), "reportPath": str(report_path)}, indent=2))
finally:
    if created_tab_id:
        try:
            cdp("Target.closeTarget", targetId=created_tab_id)
        except Exception:
            pass

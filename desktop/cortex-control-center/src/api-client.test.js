import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  createApi,
  createPostApi,
  isAuthFailure,
  settledWithRethrow,
  settledCollectErrors,
  summarizeDashboardErrors,
} from "./api-client.js";

// -- Helpers ------------------------------------------------------------------

function makeDeps(overrides = {}) {
  return {
    getInvoke: () => overrides.invoke ?? null,
    getToken: () =>
      typeof overrides.getToken === "function" ? overrides.getToken() : (overrides.token ?? ""),
    cortexBase: overrides.cortexBase ?? "http://127.0.0.1:7437",
    onTokenRefresh: overrides.onTokenRefresh,
  };
}

function mockFetch(status, body, ok) {
  return vi.fn(() =>
    Promise.resolve({
      ok: ok ?? (status >= 200 && status < 300),
      status,
      json: () => Promise.resolve(body),
    })
  );
}

// =============================================================================
// api() throw paths
// =============================================================================

describe("createApi - api()", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("throws with path when withAuth=true and no token (no IPC)", async () => {
    const api = createApi(makeDeps({ token: "" }));
    await expect(api("/sessions", true)).rejects.toThrow(
      "/sessions: no auth token (Tauri IPC missing)"
    );
  });

  it("throws with path when withAuth=true and no token (IPC available)", async () => {
    const invoke = vi.fn();
    const api = createApi(makeDeps({ token: "", invoke }));
    await expect(api("/sessions", true)).rejects.toThrow(
      "/sessions: no auth token (Tauri IPC available)"
    );
    expect(invoke).not.toHaveBeenCalled();
  });

  it("throws on invalid IPC response (null)", async () => {
    globalThis.fetch = vi.fn(() => Promise.reject(new Error("network down")));
    const invoke = vi.fn(() => Promise.resolve(null));
    const api = createApi(makeDeps({ invoke, token: "tok" }));
    await expect(api("/health")).rejects.toThrow("/health: invalid IPC response");
  });

  it("times out hung IPC GET requests", async () => {
    vi.useFakeTimers();
    try {
      globalThis.fetch = vi.fn(() => Promise.reject(new Error("network down")));
      const invoke = vi.fn(() => new Promise(() => {}));
      const api = createApi(makeDeps({ invoke, token: "tok" }));
      const assertion = expect(api("/health")).rejects.toThrow(
        "/health: IPC request: timed out after 12000ms"
      );
      await vi.advanceTimersByTimeAsync(12000);
      await assertion;
    } finally {
      vi.useRealTimers();
    }
  });

  it("falls back to HTTP GET when IPC request times out", async () => {
    vi.useFakeTimers();
    try {
      const invoke = vi.fn(() => new Promise(() => {}));
      globalThis.fetch = mockFetch(200, { status: "ok" }, true);
      const api = createApi(makeDeps({ invoke, token: "tok" }));
      const pending = api("/probe");
      await vi.advanceTimersByTimeAsync(8000);
      await expect(pending).resolves.toEqual({ status: "ok" });
      expect(globalThis.fetch).toHaveBeenCalledWith(
        "http://127.0.0.1:7437/probe",
        expect.objectContaining({
          headers: expect.objectContaining({
            "X-Cortex-Request": "true",
          }),
        }),
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("throws on invalid IPC response (missing body string)", async () => {
    globalThis.fetch = vi.fn(() => Promise.reject(new Error("network down")));
    const invoke = vi.fn(() => Promise.resolve({ status: 200, body: 42 }));
    const api = createApi(makeDeps({ invoke, token: "tok" }));
    await expect(api("/health")).rejects.toThrow("/health: invalid IPC response");
  });

  it("throws on IPC HTTP non-2xx", async () => {
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 403, body: '{"error":"forbidden"}' })
    );
    const api = createApi(makeDeps({ invoke, token: "tok" }));
    await expect(api("/sessions", true)).rejects.toThrow("/sessions: HTTP 403");
  });

  it("throws on IPC JSON parse failure", async () => {
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 200, body: "not json{{{" })
    );
    const api = createApi(makeDeps({ invoke, token: "tok" }));
    await expect(api("/health")).rejects.toThrow(); // SyntaxError from JSON.parse
  });

  it("returns parsed JSON on IPC success", async () => {
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 200, body: '{"sessions":[]}' })
    );
    const api = createApi(makeDeps({ invoke, token: "tok" }));
    const result = await api("/sessions", true);
    expect(result).toEqual({ sessions: [] });
  });

  it("uses an extended transport timeout for MCP RPC IPC GET requests", async () => {
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 200, body: '{"ok":true}' })
    );
    const api = createApi(makeDeps({ invoke, token: "tok" }));
    await api("/mcp-rpc", true);
    expect(invoke).toHaveBeenCalledWith("fetch_cortex", {
      path: "/mcp-rpc",
      authToken: "tok",
      timeoutMs: 29500,
    });
  });

  it("routes absolute session URLs to core IPC timeout budgets", async () => {
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 200, body: '{"sessions":[]}' })
    );
    const api = createApi(makeDeps({ invoke, token: "tok" }));
    await api("http://127.0.0.1:7437/sessions", true);
    expect(invoke).toHaveBeenCalledWith("fetch_cortex", {
      path: "http://127.0.0.1:7437/sessions",
      authToken: "tok",
      timeoutMs: 14500,
    });
  });

  it("throws on browser fetch HTTP non-2xx", async () => {
    globalThis.fetch = mockFetch(500, {}, false);
    const api = createApi(makeDeps({ token: "tok" }));
    await expect(api("/health")).rejects.toThrow("/health: HTTP 500");
  });

  it("returns parsed JSON on browser fetch success", async () => {
    globalThis.fetch = mockFetch(200, { status: "ok" }, true);
    const api = createApi(makeDeps({ token: "tok" }));
    const result = await api("/health");
    expect(result).toEqual({ status: "ok" });
  });

  it("sends auth header on browser fetch when withAuth=true", async () => {
    globalThis.fetch = mockFetch(200, {}, true);
    const api = createApi(makeDeps({ token: "mytoken" }));
    await api("/sessions", true);
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://127.0.0.1:7437/sessions",
      expect.objectContaining({
        headers: expect.objectContaining({
          Authorization: "Bearer mytoken",
        }),
      })
    );
  });

  it("does NOT require auth when withAuth=false even with empty token", async () => {
    globalThis.fetch = mockFetch(200, { status: "ok" }, true);
    const api = createApi(makeDeps({ token: "" }));
    const result = await api("/health", false);
    expect(result).toEqual({ status: "ok" });
  });

  it("polls for a rotated GET token but does not reissue the request when refresh returns the same token", async () => {
    let token = "stale-token";
    const onTokenRefresh = vi.fn(async () => {
      token = "stale-token";
    });
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 401, body: '{"error":"Unauthorized"}' })
    );
    const api = createApi(
      makeDeps({
        getToken: () => token,
        invoke,
        onTokenRefresh,
      })
    );

    await expect(api("/sessions", true)).rejects.toThrow("/sessions: HTTP 401");
    expect(onTokenRefresh).toHaveBeenCalledTimes(4);
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});

// =============================================================================
// postApi() throw paths
// =============================================================================

describe("createPostApi - postApi()", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("throws when no token (always requires auth)", async () => {
    const postApi = createPostApi(makeDeps({ token: "" }));
    await expect(postApi("/resolve")).rejects.toThrow(
      "POST /resolve: no auth token"
    );
  });

  it("refreshes token once before POST when startup token is missing", async () => {
    let token = "";
    const onTokenRefresh = vi.fn(async () => {
      token = "fresh-token";
    });
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 200, body: '{"ok":true}' })
    );

    const postApi = createPostApi(
      makeDeps({
        getToken: () => token,
        invoke,
        onTokenRefresh,
      })
    );

    const result = await postApi("/resolve", { keepId: "a" });
    expect(result).toEqual({ ok: true });
    expect(onTokenRefresh).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("post_cortex", {
      path: "/resolve",
      authToken: "fresh-token",
      body: '{"keepId":"a"}',
      timeoutMs: 7500,
    });
  });

  it("throws on invalid IPC response", async () => {
    globalThis.fetch = vi.fn(() => Promise.reject(new Error("network down")));
    const invoke = vi.fn(() => Promise.resolve(undefined));
    const postApi = createPostApi(makeDeps({ invoke, token: "tok" }));
    await expect(postApi("/resolve")).rejects.toThrow(
      "POST /resolve: invalid IPC response"
    );
  });

  it("times out hung IPC POST requests", async () => {
    vi.useFakeTimers();
    try {
      globalThis.fetch = vi.fn(() => Promise.reject(new Error("network down")));
      const invoke = vi.fn(() => new Promise(() => {}));
      const postApi = createPostApi(makeDeps({ invoke, token: "tok" }));
      const assertion = expect(postApi("/resolve", { keepId: "a" })).rejects.toThrow(
        "POST /resolve: IPC request: timed out after 8000ms"
      );
      await vi.advanceTimersByTimeAsync(8000);
      await assertion;
    } finally {
      vi.useRealTimers();
    }
  });

  it("falls back to HTTP POST when IPC request times out", async () => {
    vi.useFakeTimers();
    try {
      const invoke = vi.fn(() => new Promise(() => {}));
      globalThis.fetch = mockFetch(200, { ok: true }, true);
      const postApi = createPostApi(makeDeps({ invoke, token: "tok" }));
      const pending = postApi("/resolve", { keepId: "a" });
      await vi.advanceTimersByTimeAsync(8000);
      await expect(pending).resolves.toEqual({ ok: true });
      expect(globalThis.fetch).toHaveBeenCalledWith(
        "http://127.0.0.1:7437/resolve",
        expect.objectContaining({
          method: "POST",
          headers: expect.objectContaining({
            Authorization: "Bearer tok",
            "X-Cortex-Request": "true",
          }),
        }),
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("throws on IPC HTTP non-2xx", async () => {
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 422, body: '{"error":"bad"}' })
    );
    const postApi = createPostApi(makeDeps({ invoke, token: "tok" }));
    await expect(postApi("/resolve")).rejects.toThrow("POST /resolve: HTTP 422");
  });

  it("returns parsed JSON on IPC success", async () => {
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 200, body: '{"ok":true}' })
    );
    const postApi = createPostApi(makeDeps({ invoke, token: "tok" }));
    const result = await postApi("/resolve", { keepId: "a" });
    expect(result).toEqual({ ok: true });
    expect(invoke).toHaveBeenCalledWith("post_cortex", {
      path: "/resolve",
      authToken: "tok",
      body: '{"keepId":"a"}',
      timeoutMs: 7500,
    });
  });

  it("retries POST once after IPC 401 using refreshed token", async () => {
    let token = "stale-token";
    const onTokenRefresh = vi.fn(async () => {
      token = "fresh-token";
    });
    const invoke = vi
      .fn()
      .mockResolvedValueOnce({ status: 401, body: '{"error":"Unauthorized"}' })
      .mockResolvedValueOnce({ status: 200, body: '{"ok":true}' });

    const postApi = createPostApi(
      makeDeps({
        getToken: () => token,
        invoke,
        onTokenRefresh,
      })
    );

    const result = await postApi("/resolve", { keepId: "a" });
    expect(result).toEqual({ ok: true });
    expect(onTokenRefresh).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenNthCalledWith(1, "post_cortex", {
      path: "/resolve",
      authToken: "stale-token",
      body: '{"keepId":"a"}',
      timeoutMs: 7500,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "post_cortex", {
      path: "/resolve",
      authToken: "fresh-token",
      body: '{"keepId":"a"}',
      timeoutMs: 7500,
    });
  });

  it("uses an extended transport timeout for MCP RPC IPC POST requests", async () => {
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 200, body: '{"ok":true}' })
    );
    const postApi = createPostApi(makeDeps({ invoke, token: "tok" }));
    await postApi("/mcp-rpc", { jsonrpc: "2.0", id: "1" });
    expect(invoke).toHaveBeenCalledWith("post_cortex", {
      path: "/mcp-rpc",
      authToken: "tok",
      body: '{"jsonrpc":"2.0","id":"1"}',
      timeoutMs: 29500,
    });
  });

  it("throws on browser fetch HTTP non-2xx", async () => {
    globalThis.fetch = mockFetch(500, {}, false);
    const postApi = createPostApi(makeDeps({ token: "tok" }));
    await expect(postApi("/resolve")).rejects.toThrow("POST /resolve: HTTP 500");
  });

  it("sends POST with correct headers on browser fetch", async () => {
    globalThis.fetch = mockFetch(200, { ok: true }, true);
    const postApi = createPostApi(makeDeps({ token: "tok" }));
    await postApi("/resolve", { x: 1 });
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "http://127.0.0.1:7437/resolve",
      expect.objectContaining({
        method: "POST",
        headers: expect.objectContaining({
          "Content-Type": "application/json",
          Authorization: "Bearer tok",
        }),
        body: '{"x":1}',
      })
    );
  });

  it("retries browser POST once after 401 using refreshed token", async () => {
    let token = "stale-token";
    const onTokenRefresh = vi.fn(async () => {
      token = "fresh-token";
    });
    globalThis.fetch = vi
      .fn()
      .mockResolvedValueOnce({
        ok: false,
        status: 401,
        json: () => Promise.resolve({ error: "Unauthorized" }),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: () => Promise.resolve({ ok: true }),
      });

    const postApi = createPostApi(
      makeDeps({
        getToken: () => token,
        onTokenRefresh,
      })
    );

    const result = await postApi("/resolve", { x: 1 });
    expect(result).toEqual({ ok: true });
    expect(onTokenRefresh).toHaveBeenCalledTimes(1);
    expect(globalThis.fetch).toHaveBeenNthCalledWith(
      1,
      "http://127.0.0.1:7437/resolve",
      expect.objectContaining({
        headers: expect.objectContaining({
          Authorization: "Bearer stale-token",
        }),
      })
    );
    expect(globalThis.fetch).toHaveBeenNthCalledWith(
      2,
      "http://127.0.0.1:7437/resolve",
      expect.objectContaining({
        headers: expect.objectContaining({
          Authorization: "Bearer fresh-token",
        }),
      })
    );
  });

  it("polls for a rotated POST token but does not reissue the request when refresh returns the same token", async () => {
    let token = "stale-token";
    const onTokenRefresh = vi.fn(async () => {
      token = "stale-token";
    });
    const invoke = vi.fn(() =>
      Promise.resolve({ status: 401, body: '{"error":"Unauthorized"}' })
    );

    const postApi = createPostApi(
      makeDeps({
        getToken: () => token,
        invoke,
        onTokenRefresh,
      })
    );

    await expect(postApi("/resolve", { keepId: "a" })).rejects.toThrow("POST /resolve: HTTP 401");
    expect(onTokenRefresh).toHaveBeenCalledTimes(4);
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});

// =============================================================================
// settledWithRethrow (refreshCoreData pattern)
// =============================================================================

describe("settledWithRethrow", () => {
  it("applies all values on full success", async () => {
    const results = [];
    await settledWithRethrow([
      { fn: () => Promise.resolve("a"), apply: (v) => results.push(v) },
      { fn: () => Promise.resolve("b"), apply: (v) => results.push(v) },
    ]);
    expect(results).toEqual(["a", "b"]);
  });

  it("applies partial results then re-throws on partial failure", async () => {
    const results = [];
    await expect(
      settledWithRethrow([
        { fn: () => Promise.resolve("ok"), apply: (v) => results.push(v) },
        {
          fn: () => Promise.reject(new Error("/sessions: HTTP 403")),
          apply: (v) => results.push(v),
        },
        { fn: () => Promise.resolve("also-ok"), apply: (v) => results.push(v) },
      ])
    ).rejects.toThrow("/sessions: HTTP 403");

    // partial results were still applied
    expect(results).toEqual(["ok", null, "also-ok"]);
  });

  it("joins multiple failure reasons", async () => {
    await expect(
      settledWithRethrow([
        {
          fn: () => Promise.reject(new Error("err1")),
          apply: () => {},
        },
        {
          fn: () => Promise.reject(new Error("err2")),
          apply: () => {},
        },
      ])
    ).rejects.toThrow("err1; err2");
  });
});

// =============================================================================
// settledCollectErrors (refreshAll pattern)
// =============================================================================

describe("settledCollectErrors", () => {
  it("returns empty array on full success", async () => {
    const errors = await settledCollectErrors([
      () => Promise.resolve(),
      () => Promise.resolve(),
    ]);
    expect(errors).toEqual([]);
  });

  it("never throws, collects error messages instead", async () => {
    const errors = await settledCollectErrors([
      () => Promise.resolve(),
      () => Promise.reject(new Error("daemon down")),
      () => Promise.reject(new Error("auth failed")),
    ]);
    expect(errors).toEqual(["daemon down", "auth failed"]);
  });

  it("deduplicates identical error messages", async () => {
    const errors = await settledCollectErrors([
      () => Promise.reject(new Error("no auth token")),
      () => Promise.reject(new Error("no auth token")),
      () => Promise.reject(new Error("different")),
    ]);
    expect(errors).toEqual(["no auth token", "different"]);
  });

  it("handles non-Error rejections gracefully", async () => {
    const errors = await settledCollectErrors([
      () => Promise.reject("raw string"),
      () => Promise.reject(42),
    ]);
    expect(errors).toEqual(["raw string", "42"]);
  });
});

describe("summarizeDashboardErrors", () => {
  it("collapses protected endpoint auth failures into one message", () => {
    expect(
      summarizeDashboardErrors([
        "/sessions: HTTP 401",
        "/locks: HTTP 401",
        "/tasks?status=all: HTTP 401",
        "/feed?since=1h: HTTP 401",
      ])
    ).toBe(
      "Sessions, Locks, Tasks, Feed could not authenticate. Refresh the token or restart the daemon from Control Center."
    );
  });

  it("falls back to the original joined output for mixed failures", () => {
    expect(
      summarizeDashboardErrors([
        "/sessions: HTTP 401",
        "/health: HTTP 500",
      ])
    ).toBe("/sessions: HTTP 401; /health: HTTP 500");
  });
});

describe("isAuthFailure", () => {
  it("matches http auth failures", () => {
    expect(isAuthFailure("/sessions: HTTP 401")).toBe(true);
  });

  it("matches missing token failures", () => {
    expect(isAuthFailure("POST /resolve: no auth token")).toBe(true);
  });

  it("ignores unrelated errors", () => {
    expect(isAuthFailure("/health: HTTP 500")).toBe(false);
  });
});

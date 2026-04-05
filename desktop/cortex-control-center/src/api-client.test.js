import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  createApi,
  createPostApi,
  settledWithRethrow,
  settledCollectErrors,
} from "./api-client.js";

// -- Helpers ------------------------------------------------------------------

function makeDeps(overrides = {}) {
  return {
    getInvoke: () => overrides.invoke ?? null,
    getToken: () => overrides.token ?? "",
    cortexBase: overrides.cortexBase ?? "http://127.0.0.1:7437",
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
    const invoke = vi.fn(() => Promise.resolve(null));
    const api = createApi(makeDeps({ invoke, token: "tok" }));
    await expect(api("/health")).rejects.toThrow("/health: invalid IPC response");
  });

  it("throws on invalid IPC response (missing body string)", async () => {
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

  it("throws on invalid IPC response", async () => {
    const invoke = vi.fn(() => Promise.resolve(undefined));
    const postApi = createPostApi(makeDeps({ invoke, token: "tok" }));
    await expect(postApi("/resolve")).rejects.toThrow(
      "POST /resolve: invalid IPC response"
    );
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

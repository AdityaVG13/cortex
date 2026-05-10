import { describe, expect, it } from "vitest";
import { fnv1a32 } from "../util/fnv1a.js";

describe("fnv1a32", () => {
  it("returns the canonical offset basis for empty input", () => {
    expect(fnv1a32("")).toBe(2166136261);
    expect(fnv1a32(new Uint8Array(0))).toBe(2166136261);
  });

  it("matches the well-known value for 'foobar'", () => {
    expect(fnv1a32("foobar")).toBe(0xbf9cf968);
  });

  it("is reproducible for identical input", () => {
    const a = fnv1a32("centroid-bytes-001");
    const b = fnv1a32("centroid-bytes-001");
    expect(a).toBe(b);
  });

  it("differs for different input", () => {
    expect(fnv1a32("a")).not.toBe(fnv1a32("b"));
  });
});

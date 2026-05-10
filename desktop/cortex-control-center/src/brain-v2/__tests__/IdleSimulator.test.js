import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createIdleSimulator, IDLE_THRESHOLD, FAKE_INTERVAL_RANGE } from "../IdleSimulator.js";

describe("IdleSimulator", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it("does not fire fakes before idle threshold", () => {
    const fired = [];
    const sim = createIdleSimulator({
      onFake: (id) => fired.push(id),
      getNodeIds: () => ["a", "b", "c"],
      seed: 12345,
    });
    vi.advanceTimersByTime(IDLE_THRESHOLD - 1_000);
    expect(fired.length).toBe(0);
    sim.dispose();
  });

  it("fires fakes after idle threshold elapses with no real events", () => {
    const fired = [];
    const sim = createIdleSimulator({
      onFake: (id) => fired.push(id),
      getNodeIds: () => ["a", "b", "c"],
      seed: 12345,
    });
    vi.advanceTimersByTime(IDLE_THRESHOLD + FAKE_INTERVAL_RANGE[1] + 100);
    expect(fired.length).toBeGreaterThan(0);
    expect(["a", "b", "c"]).toContain(fired[0]);
    sim.dispose();
  });

  it("noteRealEvent resets the idle timer", () => {
    const fired = [];
    const sim = createIdleSimulator({
      onFake: (id) => fired.push(id),
      getNodeIds: () => ["x"],
      seed: 1,
    });
    vi.advanceTimersByTime(IDLE_THRESHOLD - 500);
    sim.noteRealEvent();
    vi.advanceTimersByTime(IDLE_THRESHOLD - 500);
    expect(fired.length).toBe(0);
    sim.dispose();
  });

  it("seeded PRNG is reproducible", () => {
    const a = [];
    const simA = createIdleSimulator({
      onFake: (id) => a.push(id),
      getNodeIds: () => ["one", "two", "three", "four"],
      seed: 999,
    });
    vi.advanceTimersByTime(IDLE_THRESHOLD + FAKE_INTERVAL_RANGE[1] * 4);
    simA.dispose();

    vi.useRealTimers();
    vi.useFakeTimers();
    const b = [];
    const simB = createIdleSimulator({
      onFake: (id) => b.push(id),
      getNodeIds: () => ["one", "two", "three", "four"],
      seed: 999,
    });
    vi.advanceTimersByTime(IDLE_THRESHOLD + FAKE_INTERVAL_RANGE[1] * 4);
    simB.dispose();

    expect(a).toEqual(b);
    expect(a.length).toBeGreaterThan(0);
  });
});

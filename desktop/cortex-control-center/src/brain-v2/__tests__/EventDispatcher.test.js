import { describe, expect, it } from "vitest";

import { createEventDispatcher } from "../EventDispatcher.js";

function createHarness() {
  const slots = new Map([
    ["loose-1", { id: "loose-1", tier: "loose", x: 10, y: 0, z: 0 }],
    ["loose-2", { id: "loose-2", tier: "loose", x: 20, y: 0, z: 0 }],
    ["decision-2", { id: "decision-2", tier: "decision", x: 0, y: 10, z: 0 }],
    ["cluster-7", { id: "cluster-7", tier: "cluster", centroidKey: "7", x: 0, y: 0, z: 10 }],
  ]);
  const pulsed = [];
  const fired = [];
  const ticker = [];
  const spotlights = [];
  let corePulses = 0;

  const dispatcher = createEventDispatcher({
    satellites: {
      getSlotById: (id) => slots.get(id) || null,
      pulseSlot: (id) => pulsed.push(id),
    },
    beams: {
      fire: (payload) => fired.push(payload),
    },
    core: {},
    pulseCoreHalo: () => { corePulses += 1; },
    onTickerEntry: (entry) => ticker.push(entry),
    onSpotlight: (slot) => spotlights.push(slot.id),
  });

  return {
    dispatcher,
    fired,
    pulsed,
    ticker,
    spotlights,
    corePulseCount: () => corePulses,
  };
}

describe("Brain v2 EventDispatcher", () => {
  it("spotlights real member, cluster, link, and recall firing events", () => {
    const harness = createHarness();

    harness.dispatcher.dispatch({
      type: "member_added",
      member_id: "memory-1",
      cluster_id: 7,
    });
    harness.dispatcher.dispatch({
      type: "cluster_finalized",
      cluster_id: 7,
      member_count: 3,
    });
    harness.dispatcher.dispatch({
      type: "link_inferred",
      a: "memory-1",
      b: "decision-2",
    });
    harness.dispatcher.dispatch({
      type: "recall",
      node_ids: ["memory-1", "decision-2"],
    });

    expect(harness.spotlights).toEqual(["loose-1", "cluster-7", "loose-1", "loose-1"]);
    expect(harness.pulsed).toEqual(["loose-1", "cluster-7", "loose-1", "decision-2"]);
    expect(harness.fired).toHaveLength(4);
    expect(harness.corePulseCount()).toBe(1);
    expect(harness.ticker).toHaveLength(4);
  });

  it("does not camera-spotlight idle fake firing", () => {
    const harness = createHarness();

    harness.dispatcher.dispatchFake("loose-2");

    expect(harness.spotlights).toEqual([]);
    expect(harness.pulsed).toEqual(["loose-2"]);
    expect(harness.fired).toHaveLength(1);
  });
});

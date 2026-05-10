import { mulberry32 } from "./util/mulberry32.js";

const IDLE_THRESHOLD_MS = 6_000;
const FAKE_INTERVAL_MIN_MS = 900;
const FAKE_INTERVAL_MAX_MS = 2_400;
const BURST_MIN = 1;
const BURST_MAX = 3;
const BURST_STAGGER_MS = 90;

export function createIdleSimulator({ onFake, getNodeIds, seed = Date.now() }) {
  let lastReal = performance.now();
  let timer = null;
  const burstTimers = new Set();
  let disposed = false;
  const rand = mulberry32(seed);

  function fireBurst() {
    const ids = (typeof getNodeIds === "function" ? getNodeIds() : []) || [];
    if (!ids.length || typeof onFake !== "function") return;
    const burst = BURST_MIN + Math.floor(rand() * (BURST_MAX - BURST_MIN + 1));
    for (let i = 0; i < burst; i += 1) {
      const wait = i * BURST_STAGGER_MS;
      const t = setTimeout(() => {
        burstTimers.delete(t);
        if (disposed) return;
        if (performance.now() - lastReal < IDLE_THRESHOLD_MS) return;
        const pick = ids[Math.floor(rand() * ids.length)];
        if (pick) onFake(pick);
      }, wait);
      burstTimers.add(t);
    }
  }

  function schedule() {
    if (disposed) return;
    const wait = FAKE_INTERVAL_MIN_MS + rand() * (FAKE_INTERVAL_MAX_MS - FAKE_INTERVAL_MIN_MS);
    timer = setTimeout(() => {
      if (disposed) return;
      const idle = performance.now() - lastReal;
      if (idle >= IDLE_THRESHOLD_MS) fireBurst();
      schedule();
    }, wait);
  }

  function noteRealEvent() {
    lastReal = performance.now();
    for (const t of burstTimers) clearTimeout(t);
    burstTimers.clear();
    if (timer) {
      clearTimeout(timer);
      timer = null;
      schedule();
    }
  }

  schedule();

  return {
    noteRealEvent,
    dispose: () => {
      disposed = true;
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
      for (const t of burstTimers) clearTimeout(t);
      burstTimers.clear();
    },
  };
}

export const IDLE_THRESHOLD = IDLE_THRESHOLD_MS;
export const FAKE_INTERVAL_RANGE = [FAKE_INTERVAL_MIN_MS, FAKE_INTERVAL_MAX_MS];
export const BURST_RANGE = [BURST_MIN, BURST_MAX];

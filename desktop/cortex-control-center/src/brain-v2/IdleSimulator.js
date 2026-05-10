import { mulberry32 } from "./util/mulberry32.js";

const IDLE_THRESHOLD_MS = 6_000;
const FAKE_INTERVAL_MIN_MS = 900;
const FAKE_INTERVAL_MAX_MS = 2_400;

export function createIdleSimulator({ onFake, getNodeIds, seed = Date.now() }) {
  let lastReal = performance.now();
  let timer = null;
  let disposed = false;
  const rand = mulberry32(seed);

  function schedule() {
    if (disposed) return;
    const wait = FAKE_INTERVAL_MIN_MS + rand() * (FAKE_INTERVAL_MAX_MS - FAKE_INTERVAL_MIN_MS);
    timer = setTimeout(() => {
      if (disposed) return;
      const idle = performance.now() - lastReal;
      if (idle >= IDLE_THRESHOLD_MS) {
        const ids = (typeof getNodeIds === "function" ? getNodeIds() : []) || [];
        if (ids.length > 0) {
          const pick = ids[Math.floor(rand() * ids.length)];
          if (pick && typeof onFake === "function") onFake(pick);
        }
      }
      schedule();
    }, wait);
  }

  function noteRealEvent() {
    lastReal = performance.now();
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
    },
  };
}

export const IDLE_THRESHOLD = IDLE_THRESHOLD_MS;
export const FAKE_INTERVAL_RANGE = [FAKE_INTERVAL_MIN_MS, FAKE_INTERVAL_MAX_MS];

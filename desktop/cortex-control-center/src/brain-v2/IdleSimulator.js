import { mulberry32 } from "./util/mulberry32.js";
const IDLE_THRESHOLD_MS = 6e3, FAKE_INTERVAL_MIN_MS = 900, FAKE_INTERVAL_MAX_MS = 2400, BURST_MIN = 2, BURST_MAX = 4, BURST_STAGGER_MS = 140;
function createIdleSimulator({ onFake, getNodeIds, seed = Date.now() }) { let lastReal = performance.now(), timer = null;
  const burstTimers = new Set();
  let disposed = !1;
  const rand = mulberry32(seed);
  function fireBurst() { const ids = (typeof getNodeIds == "function" ? getNodeIds() : []) || [];
    if (!ids.length || typeof onFake != "function") return;
    const burst = BURST_MIN + Math.floor(rand() * (BURST_MAX - BURST_MIN + 1));
    for (let i = 0; i < burst; i += 1) { const wait = i * BURST_STAGGER_MS,
        t = setTimeout(() => { if ((burstTimers.delete(t), disposed || performance.now() - lastReal < IDLE_THRESHOLD_MS)) return;
          const pick = ids[Math.floor(rand() * ids.length)];
          pick && onFake(pick);
        }, wait);
      burstTimers.add(t);
    }
  }
  function schedule() { if (disposed) return;
    const wait = FAKE_INTERVAL_MIN_MS + rand() * (FAKE_INTERVAL_MAX_MS - FAKE_INTERVAL_MIN_MS);
    timer = setTimeout(() => { if (disposed) return;
      (performance.now() - lastReal >= IDLE_THRESHOLD_MS && fireBurst(), schedule());
    }, wait);
  }
  function noteRealEvent() { lastReal = performance.now();
    for (const t of burstTimers) clearTimeout(t);
    (burstTimers.clear(), timer && (clearTimeout(timer), (timer = null), schedule()));
  }
  return ( schedule(), { noteRealEvent, dispose: () => { ((disposed = !0), timer && (clearTimeout(timer), (timer = null)));
        for (const t of burstTimers) clearTimeout(t);
        burstTimers.clear(); }, } );
}
export { createIdleSimulator };

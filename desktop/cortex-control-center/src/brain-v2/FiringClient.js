// Native EventSource client for /brain/firing. Token rides as ?token=
// because EventSource cannot send custom headers. Browser handles
// auto-reconnect with built-in backoff; we log lifecycle for diagnostics.
export function createFiringClient({ baseUrl, token, onEvent }) {
  if (!baseUrl || !token || typeof onEvent !== "function") {
    return { disconnect: () => {} };
  }
  const url = `${baseUrl.replace(/\/+$/, "")}/brain/firing?token=${encodeURIComponent(token)}`;
  let source = null;

  function attach() {
    try {
      source = new EventSource(url);
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn("[brain-v2] firing client construct failed", err);
      return;
    }
    source.addEventListener("connected", () => {
      // eslint-disable-next-line no-console
      console.debug("[brain-v2] firing connected");
    });
    source.addEventListener("brain_batch", (msg) => {
      let parsed;
      try {
        parsed = JSON.parse(msg.data);
      } catch {
        return;
      }
      if (!Array.isArray(parsed)) return;
      for (const event of parsed) {
        try {
          onEvent(event);
        } catch (err) {
          // eslint-disable-next-line no-console
          console.error("[brain-v2] onEvent error", err);
        }
      }
    });
    source.addEventListener("error", () => {
      // Browser EventSource will auto-reconnect; just observe.
      // eslint-disable-next-line no-console
      console.debug("[brain-v2] firing connection error; browser will retry");
    });
  }

  attach();

  return {
    disconnect: () => {
      if (source) {
        source.close();
        source = null;
      }
    },
  };
}

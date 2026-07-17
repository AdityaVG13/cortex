function createFiringClient({ baseUrl, token, onEvent }) {
  if (!baseUrl || !token || typeof onEvent != "function") return { disconnect: () => {} };
  const url = `${baseUrl.replace(/\/+$/, "")}/brain/firing?token=${encodeURIComponent(token)}`;
  let source = null;
  function attach() {
    try {
      source = new EventSource(url);
    } catch (err) {
      console.warn("[brain-v2] firing client construct failed", err);
      return;
    }
    source.addEventListener("brain_batch", (msg) => {
      let parsed;
      try {
        parsed = JSON.parse(msg.data);
      } catch {
        return;
      }
      if (Array.isArray(parsed))
        for (const event of parsed)
          try {
            onEvent(event);
          } catch (err) {
            console.error("[brain-v2] onEvent error", err);
          }
    });
  }
  return (
    attach(),
    {
      disconnect: () => {
        source && (source.close(), (source = null));
      },
    }
  );
}
export { createFiringClient };

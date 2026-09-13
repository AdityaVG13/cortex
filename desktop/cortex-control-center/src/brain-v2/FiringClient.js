function createFiringClient({ baseUrl, token, onEvent }) { if (!baseUrl || !token || typeof onEvent != "function") return { disconnect: () => {} };
  const url = `${baseUrl.replace(/\/+$/, "")}/brain/firing?token=${encodeURIComponent(token)}`;
  let source = null, reconnectTimer = null, reconnectAttempt = 0, disposed = !1;
  function clearReconnect() { reconnectTimer && (clearTimeout(reconnectTimer), (reconnectTimer = null)); }
  function scheduleReconnect() { if (disposed || reconnectAttempt >= 8) return;
    const delay = Math.min(5e3, 1e3 * 2 ** reconnectAttempt);
    ((reconnectAttempt += 1), clearReconnect(), (reconnectTimer = setTimeout(() => { ((reconnectTimer = null), attach());
      }, delay)));
  }
  function attach() { if (disposed || source) return;
    try { source = new EventSource(url);
    } catch (err) { console.warn("[brain-v2] firing client construct failed", err);
      scheduleReconnect();
      return;
    }
    source.onopen = () => { reconnectAttempt = 0;
    };
    source.addEventListener("brain_batch", (msg) => { let parsed;
      try { parsed = JSON.parse(msg.data);
      } catch { return;
      }
      if (Array.isArray(parsed))
        for (const event of parsed)
          try { onEvent(event);
          } catch (err) { console.error("[brain-v2] onEvent error", err);
          }
    });
    source.onerror = () => { if (disposed || !source) return;
      (source.close(), (source = null), scheduleReconnect());
    };
  }
  return ( attach(), { disconnect: () => { ((disposed = !0), clearReconnect(), source && (source.close(), (source = null))); }, } );
}
export { createFiringClient };

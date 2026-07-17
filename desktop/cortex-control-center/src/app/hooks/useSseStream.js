import { useEffect } from "react";
import { SSE_RECONNECT_BASE_MS, SSE_RECONNECT_MAX_MS, SSE_REFRESH_THROTTLE_MS } from "../constants.js";

export function useSseStream(ctx) {
  const {
    daemonState,
    cortexBase,
    refreshAllRef,
    tokenRef,
    streamConnectedAtRef,
    streamSessionEventCountRef,
    streamDisconnectedAtRef,
  } = ctx;

  useEffect(() => {
    let stream = null;
    let refreshTimer = null;
    let reconnectTimer = null;
    let reconnectAttempt = 0;
    let lastRefreshAt = 0;
    let refreshInFlight = false;
    let refreshQueued = false;
    let disposed = false;

    const clearRefreshTimer = () => {
      if (refreshTimer) {
        window.clearTimeout(refreshTimer);
        refreshTimer = null;
      }
    };

    const clearReconnectTimer = () => {
      if (reconnectTimer) {
        window.clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
    };

    const scheduleRefresh = (immediate = false) => {
      if (disposed || refreshTimer) return;
      const elapsed = Date.now() - lastRefreshAt;
      const delay = immediate ? 0 : Math.max(SSE_REFRESH_THROTTLE_MS - elapsed, 0);

      refreshTimer = window.setTimeout(() => {
        refreshTimer = null;
        if (refreshInFlight) {
          refreshQueued = true;
          return;
        }

        refreshInFlight = true;
        Promise.resolve(refreshAllRef.current())
          .finally(() => {
            lastRefreshAt = Date.now();
            refreshInFlight = false;
            if (refreshQueued && !disposed) {
              refreshQueued = false;
              scheduleRefresh();
            }
          });
      }, delay);
    };

    const handleRealtimeEvent = () => {
      scheduleRefresh();
    };

    const closeStream = () => {
      if (!stream) return;
      stream.close();
      stream = null;
    };

    const scheduleReconnect = () => {
      if (disposed) return;
      const exponentialDelay = Math.min(
        SSE_RECONNECT_MAX_MS,
        SSE_RECONNECT_BASE_MS * 2 ** reconnectAttempt
      );
      const jitter = Math.floor(Math.random() * 250);
      reconnectAttempt += 1;

      clearReconnectTimer();
      reconnectTimer = window.setTimeout(() => {
        reconnectTimer = null;
        connect();
      }, exponentialDelay + jitter);
    };

    const connect = () => {
      if (disposed || stream) return;
      const token = tokenRef.current;
      if (!token) return;
      const streamUrl = `${cortexBase}/events/stream?token=${encodeURIComponent(token)}`;
      const nextStream = new EventSource(streamUrl);
      stream = nextStream;

      nextStream.onopen = () => {
        reconnectAttempt = 0;
        streamConnectedAtRef.current = Date.now();
        scheduleRefresh(true);
      };

      nextStream.onmessage = handleRealtimeEvent;
      nextStream.addEventListener("connected", handleRealtimeEvent);
      nextStream.addEventListener("task", handleRealtimeEvent);
      nextStream.addEventListener("session", () => {
        streamSessionEventCountRef.current += 1;
        handleRealtimeEvent();
      });
      nextStream.addEventListener("lock", handleRealtimeEvent);
      nextStream.addEventListener("feed", handleRealtimeEvent);
      nextStream.addEventListener("message", handleRealtimeEvent);
      nextStream.addEventListener("activity", handleRealtimeEvent);

      nextStream.onerror = () => {
        if (disposed || stream !== nextStream) return;
        streamDisconnectedAtRef.current = Date.now();
        handleRealtimeEvent();
        closeStream();
        scheduleReconnect();
      };
    };

    const handleOnline = () => {
      if (disposed) return;
      reconnectAttempt = 0;
      clearReconnectTimer();
      closeStream();
      connect();
      scheduleRefresh(true);
    };

    connect();
    window.addEventListener("online", handleOnline);

    return () => {
      disposed = true;
      window.removeEventListener("online", handleOnline);
      clearRefreshTimer();
      clearReconnectTimer();
      closeStream();
    };
  }, [cortexBase, daemonState.authTokenReady]);
  return ctx;
}

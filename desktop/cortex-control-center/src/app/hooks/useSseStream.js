import { useEffect } from "react";
import {
  SSE_RECONNECT_BASE_MS,
  SSE_RECONNECT_MAX_MS,
  SSE_REFRESH_THROTTLE_MS,
} from "../constants.js";
function useSseStream(ctx) {
  const {
    daemonState,
    cortexBase,
    refreshAllRef,
    tokenRef,
    streamConnectedAtRef,
    streamSessionEventCountRef,
    streamDisconnectedAtRef,
  } = ctx;
  return (
    useEffect(() => {
      let stream = null,
        refreshTimer = null,
        reconnectTimer = null,
        reconnectAttempt = 0,
        lastRefreshAt = 0,
        refreshInFlight = !1,
        refreshQueued = !1,
        disposed = !1;
      const clearRefreshTimer = () => {
          refreshTimer &&
            (window.clearTimeout(refreshTimer), (refreshTimer = null));
        },
        clearReconnectTimer = () => {
          reconnectTimer &&
            (window.clearTimeout(reconnectTimer), (reconnectTimer = null));
        },
        scheduleRefresh = (immediate = !1) => {
          if (disposed || refreshTimer) return;
          const elapsed = Date.now() - lastRefreshAt,
            delay = immediate
              ? 0
              : Math.max(SSE_REFRESH_THROTTLE_MS - elapsed, 0);
          refreshTimer = window.setTimeout(() => {
            if (((refreshTimer = null), refreshInFlight)) {
              refreshQueued = !0;
              return;
            }
            ((refreshInFlight = !0),
              Promise.resolve(refreshAllRef.current()).finally(() => {
                ((lastRefreshAt = Date.now()),
                  (refreshInFlight = !1),
                  refreshQueued &&
                    !disposed &&
                    ((refreshQueued = !1), scheduleRefresh()));
              }));
          }, delay);
        },
        handleRealtimeEvent = () => {
          scheduleRefresh();
        },
        closeStream = () => {
          stream && (stream.close(), (stream = null));
        },
        scheduleReconnect = () => {
          if (disposed) return;
          const exponentialDelay = Math.min(
              SSE_RECONNECT_MAX_MS,
              SSE_RECONNECT_BASE_MS * 2 ** reconnectAttempt,
            ),
            jitter = Math.floor(Math.random() * 250);
          ((reconnectAttempt += 1),
            clearReconnectTimer(),
            (reconnectTimer = window.setTimeout(() => {
              ((reconnectTimer = null), connect());
            }, exponentialDelay + jitter)));
        },
        connect = () => {
          if (disposed || stream) return;
          const token = tokenRef.current;
          if (!token) return;
          const streamUrl = `${cortexBase}/events/stream?token=${encodeURIComponent(token)}`,
            nextStream = new EventSource(streamUrl);
          ((stream = nextStream),
            (nextStream.onopen = () => {
              ((reconnectAttempt = 0),
                (streamConnectedAtRef.current = Date.now()),
                scheduleRefresh(!0));
            }),
            (nextStream.onmessage = handleRealtimeEvent),
            nextStream.addEventListener("connected", handleRealtimeEvent),
            nextStream.addEventListener("task", handleRealtimeEvent),
            nextStream.addEventListener("session", () => {
              ((streamSessionEventCountRef.current += 1),
                handleRealtimeEvent());
            }),
            nextStream.addEventListener("lock", handleRealtimeEvent),
            nextStream.addEventListener("feed", handleRealtimeEvent),
            nextStream.addEventListener("message", handleRealtimeEvent),
            nextStream.addEventListener("activity", handleRealtimeEvent),
            (nextStream.onerror = () => {
              disposed ||
                stream !== nextStream ||
                ((streamDisconnectedAtRef.current = Date.now()),
                handleRealtimeEvent(),
                closeStream(),
                scheduleReconnect());
            }));
        },
        handleOnline = () => {
          disposed ||
            ((reconnectAttempt = 0),
            clearReconnectTimer(),
            closeStream(),
            connect(),
            scheduleRefresh(!0));
        };
      return (
        connect(),
        window.addEventListener("online", handleOnline),
        () => {
          ((disposed = !0),
            window.removeEventListener("online", handleOnline),
            clearRefreshTimer(),
            clearReconnectTimer(),
            closeStream());
        }
      );
    }, [cortexBase, daemonState.authTokenReady]),
    ctx
  );
}
export { useSseStream };

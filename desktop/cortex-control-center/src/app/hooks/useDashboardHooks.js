import { useDashboardState } from "./useDashboardState.js";
import { useRefreshOrchestration } from "./useRefreshOrchestration.js";
import { useRefreshAll } from "./useRefreshAll.js";
import { useDashboardEffects } from "./useDashboardEffects.js";
import { useSseStream } from "./useSseStream.js";
import { useDaemonConnection } from "./useDaemonConnection.js";
import { useDashboardHandlers } from "./useDashboardHandlers.js";
function useDashboardHooks() {
  let ctx = useDashboardState();
  return (
    (ctx = useRefreshOrchestration(ctx)),
    (ctx = useRefreshAll(ctx)),
    (ctx = useDashboardEffects(ctx)),
    (ctx = useSseStream(ctx)),
    (ctx = useDaemonConnection(ctx)),
    useDashboardHandlers(ctx)
  );
}
export { useDashboardHooks };

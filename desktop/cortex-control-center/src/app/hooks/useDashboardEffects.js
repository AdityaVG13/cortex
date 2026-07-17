import{startTransition,useCallback,useEffect,useMemo}from"react";import{checkForUpdates}from"../../updater.js";import{MOTION_MS}from"../../design/motion.js"
;import{summarizeDashboardErrors}from"../../api-client.js";import{nextFeedAckId,sameAgent}from"../../live-surface.js"
;import{daemonStatusPill,daemonSystemStatus,daemonUtilityPill,isDaemonStartingState}from"../../daemon-startup.js"
;import{buildMonteCarloProjection}from"../../analytics-projection.js";import{summarizeBootThroughput}from"../../analytics-metrics.js"
;import{createBudgetDraftFromStatus,writeControlCenterSettings}from"../../settings/settings-state.js"
;import{ANALYTICS_REFRESH_MS,CONTROL_CENTER_VERSION,CORTEX_BASE_STORAGE_KEY,CORTEX_OPERATOR_STORAGE_KEY,CORTEX_PANEL_STORAGE_KEY,DEFAULT_CORTEX_BASE,FALLBACK_REFRESH_MS,RECALL_HEADLINE_MIN_QUERIES,
SIDEBAR_COLLAPSE_BREAKPOINT_PX}from"../constants.js"
;import{persistBrowserAuthToken}from"../browser-bootstrap.js";import{formatDaemonEndpoint,priorityRank}from"../utils/format.js"
;import{isDaemonSuppressibleErrorMessage}from"../utils/daemon.js";function useDashboardEffects(ctx){
const {
  panel:panel,daemonState:daemonState,healthMeta:healthMeta,tasks:tasks,
  locks:locks,feedEntries:feedEntries,activityEntries:activityEntries,savings:savings,
  selectedOperator:selectedOperator,setSelectedOperator:setSelectedOperator,messageTarget:messageTarget,setMessageTarget:setMessageTarget,
  messageDraft:messageDraft,setMessageDraft:setMessageDraft,setTaskCompletionDrafts:setTaskCompletionDrafts,setCompletionTaskId:setCompletionTaskId,
  daemonTimeoutStaleSummary:daemonTimeoutStaleSummary,cortexBase:cortexBase,setCortexBase:setCortexBase,setFeedbackMessage:setFeedbackMessage,
  hasVisitedAnalytics:hasVisitedAnalytics,analyticsReady:analyticsReady,controlSettings:controlSettings,budgetConfigStatus:budgetConfigStatus,
  budgetDraftDirty:budgetDraftDirty,budgetConfigBusy:budgetConfigBusy,ipcAvailable:ipcAvailable,analyticsMode:analyticsMode,
  effectiveReducedMotion:effectiveReducedMotion,refreshAllRef:refreshAllRef,tokenRef:tokenRef,isTauriRuntime:isTauriRuntime,
  normalizedSessions:normalizedSessions,knownAgents:knownAgents,selectedOperatorName:selectedOperatorName,messageTargetName:messageTargetName,
  safeCurrency:safeCurrency,runRefreshAll:runRefreshAll,reloadBudgetConfigDraft:reloadBudgetConfigDraft,refreshMessages:refreshMessages,
  refreshActivity:refreshActivity,refreshFeed:refreshFeed,refreshSavings:refreshSavings,postApi:postApi,
  activeBudgetStatus:activeBudgetStatus,budgetConfigLoadAttemptedRef:budgetConfigLoadAttemptedRef,setOsReducedMotion:setOsReducedMotion,setIsNarrowViewport:setIsNarrowViewport,
  setHasVisitedAnalytics:setHasVisitedAnalytics,setAnalyticsReady:setAnalyticsReady,clearRecoveryRetry:clearRecoveryRetry,setAvailableUpdate:setAvailableUpdate,
  skipInitialFeedRefreshRef:skipInitialFeedRefreshRef,skipInitialMessagesRefreshRef:skipInitialMessagesRefreshRef,skipInitialActivityRefreshRef:skipInitialActivityRefreshRef,startupCoreReadyState:startupCoreReadyState,

  setBusyActionKey:setBusyActionKey,refreshCoreData:refreshCoreData
}=ctx
;useEffect(()=>{localStorage.setItem(CORTEX_BASE_STORAGE_KEY,cortexBase),refreshAllRef.current()},[cortexBase]),useEffect(()=>{
isTauriRuntime&&(cortexBase!==DEFAULT_CORTEX_BASE&&setCortexBase(DEFAULT_CORTEX_BASE),tokenRef.current&&(tokenRef.current="",persistBrowserAuthToken("")))
},[cortexBase,isTauriRuntime]),useEffect(()=>{localStorage.setItem("cortex_currency",safeCurrency)},[safeCurrency]),useEffect(()=>{
budgetDraftDirty||setBudgetDraft(createBudgetDraftFromStatus(activeBudgetStatus))},[activeBudgetStatus,budgetDraftDirty]),useEffect(()=>{
if(panel!=="settings"||!ipcAvailable||budgetConfigStatus||budgetConfigBusy||budgetConfigLoadAttemptedRef.current)return
;const budgetReloadTimer=window.setTimeout(()=>{reloadBudgetConfigDraft({silent:!0})},effectiveReducedMotion?0:MOTION_MS.panel);return()=>{
window.clearTimeout(budgetReloadTimer)}},[budgetConfigBusy,budgetConfigStatus,effectiveReducedMotion,ipcAvailable,panel,reloadBudgetConfigDraft]),
useEffect(()=>{
writeControlCenterSettings(controlSettings),!(typeof document>"u")&&(document.documentElement.dataset.cortexReducedMotion=controlSettings.reducedMotion,
document.documentElement.dataset.cortexEffectiveReducedMotion=effectiveReducedMotion?"reduce":"full",
document.documentElement.dataset.cortexContrast=controlSettings.highContrast?"high":"standard",
document.documentElement.dataset.cortexKeyboardHints=controlSettings.keyboardHints?"on":"off",
document.documentElement.dataset.cortexCompactNavigation=controlSettings.compactNavigation?"on":"off")},[controlSettings,effectiveReducedMotion]),
useEffect(()=>{if(typeof window>"u"||typeof window.matchMedia!="function")return
;const query=window.matchMedia("(prefers-reduced-motion: reduce)"),syncReducedMotion=()=>setOsReducedMotion(!!query.matches);return syncReducedMotion(),
typeof query.addEventListener=="function"?(query.addEventListener("change",syncReducedMotion),
()=>query.removeEventListener("change",syncReducedMotion)):(query.addListener?.(syncReducedMotion),()=>query.removeListener?.(syncReducedMotion))},[]),
useEffect(()=>{localStorage.setItem("cortex_analytics_mode",analyticsMode)},[analyticsMode]),useEffect(()=>{if(typeof window>"u")return;const syncViewport=()=>{
setIsNarrowViewport(window.innerWidth<=SIDEBAR_COLLAPSE_BREAKPOINT_PX)};return syncViewport(),window.addEventListener("resize",syncViewport),
()=>window.removeEventListener("resize",syncViewport)},[]),useEffect(()=>{try{
selectedOperatorName?localStorage.setItem(CORTEX_OPERATOR_STORAGE_KEY,selectedOperatorName):localStorage.removeItem(CORTEX_OPERATOR_STORAGE_KEY)}catch{}
},[selectedOperatorName]),useEffect(()=>{try{localStorage.setItem(CORTEX_PANEL_STORAGE_KEY,panel)}catch{}},[panel]),useEffect(()=>{
panel==="analytics"&&setHasVisitedAnalytics(!0)},[panel]),useEffect(()=>{if(hasVisitedAnalytics)return;const warmupTimer=window.setTimeout(()=>{
startTransition(()=>{setHasVisitedAnalytics(!0),setAnalyticsReady(!0)})},250);return()=>{window.clearTimeout(warmupTimer)}},[hasVisitedAnalytics]),
useEffect(()=>{if(panel!=="analytics"||analyticsReady)return;let frameOne=0,frameTwo=0;return frameOne=requestAnimationFrame(()=>{
frameTwo=requestAnimationFrame(()=>{setAnalyticsReady(!0)})}),()=>{cancelAnimationFrame(frameOne),cancelAnimationFrame(frameTwo)}},[analyticsReady,panel]),
useEffect(()=>{refreshAllRef.current=runRefreshAll},[runRefreshAll]),useEffect(()=>()=>{clearRecoveryRetry()},[clearRecoveryRetry]),useEffect(()=>{
runRefreshAll();const interval=setInterval(()=>{refreshAllRef.current()},FALLBACK_REFRESH_MS);return()=>clearInterval(interval)},[]),useEffect(()=>{
checkForUpdates().then(update=>{update&&setAvailableUpdate(update)})},[]),useEffect(()=>{if(selectedOperator.trim())return;const defaultAgent=knownAgents[0]
;defaultAgent&&setSelectedOperator(defaultAgent)},[knownAgents,selectedOperator]),useEffect(()=>{if(messageTarget.trim())return
;const fallbackTarget=knownAgents.find(agent=>!sameAgent(agent,selectedOperator));fallbackTarget&&setMessageTarget(fallbackTarget)
},[knownAgents,messageTarget,selectedOperator]),useEffect(()=>{if(skipInitialFeedRefreshRef.current){skipInitialFeedRefreshRef.current=!1;return}
refreshFeed().catch(error=>{const message=error?.message||String(error)
;!message||isDaemonSuppressibleErrorMessage(message)||setFeedbackMessage(summarizeDashboardErrors([message])||message)})},[refreshFeed]),useEffect(()=>{
if(skipInitialMessagesRefreshRef.current){skipInitialMessagesRefreshRef.current=!1;return}refreshMessages().catch(error=>{
const message=error?.message||String(error)
;!message||isDaemonSuppressibleErrorMessage(message)||setFeedbackMessage(summarizeDashboardErrors([message])||message)})},[refreshMessages]),useEffect(()=>{
if(skipInitialActivityRefreshRef.current){skipInitialActivityRefreshRef.current=!1;return}refreshActivity().catch(error=>{
const message=error?.message||String(error)
;!message||isDaemonSuppressibleErrorMessage(message)||setFeedbackMessage(summarizeDashboardErrors([message])||message)})},[refreshActivity]),useEffect(()=>{
if(panel!=="analytics"||!analyticsReady||!daemonState.reachable||!daemonState.authTokenReady||!startupCoreReadyState)return;refreshSavings().catch(error=>{
const message=error?.message||String(error)
;!message||isDaemonSuppressibleErrorMessage(message)||setFeedbackMessage(summarizeDashboardErrors([message])||message)});const timer=setInterval(()=>{
refreshSavings().catch(error=>{const message=error?.message||String(error)
;!message||isDaemonSuppressibleErrorMessage(message)||setFeedbackMessage(summarizeDashboardErrors([message])||message)})},ANALYTICS_REFRESH_MS)
;return()=>clearInterval(timer)},[analyticsReady,daemonState.authTokenReady,daemonState.reachable,panel,refreshSavings,startupCoreReadyState])
;const pendingTasks=useMemo(()=>tasks.filter(task=>task.status==="pending").sort((a,b)=>priorityRank(b.priority)-priorityRank(a.priority)),[tasks]),claimedTasks=useMemo(()=>tasks.filter(task=>task.status==="claimed"),
[tasks]),completedTasks=useMemo(()=>tasks.filter(task=>task.status==="completed"),[tasks]),recentOverviewTasks=useMemo(()=>[...claimedTasks,...pendingTasks].slice(0,5),[claimedTasks,
pendingTasks]),pill=daemonStatusPill(daemonState),utilityPill=useMemo(()=>daemonUtilityPill(daemonState),[daemonState.reachable,daemonState.running]),daemonSysStatus=useMemo(()=>daemonSystemStatus(daemonState),
[daemonState.reachable,daemonState.running]),operationRows=useMemo(()=>Array.isArray(savings?.byOperation)?savings.byOperation:[],[savings]),operationMaxSaved=useMemo(()=>Math.max(...operationRows.map(row=>Number(row.saved||0)),
1),[operationRows]),dailySeries=useMemo(()=>Array.isArray(savings?.daily)?savings.daily:[],[savings]),cumulativeSeries=useMemo(()=>Array.isArray(savings?.cumulative)?savings.cumulative:[],
[savings]),cumulativeLatestTotal=useMemo(()=>Number(cumulativeSeries.at(-1)?.savedTotal||0),[cumulativeSeries]),recallTrendSeries=useMemo(()=>Array.isArray(savings?.recallTrend)?savings.recallTrend:[],
[savings]),activityHeatmap=useMemo(()=>Array.isArray(savings?.activityHeatmap)?savings.activityHeatmap:[],[savings]),activityHeatmapLookup=useMemo(()=>{
const map=new Map;return activityHeatmap.forEach(entry=>{map.set(`${entry.day}:${Number(entry.hour)}`,Number(entry.count||0))}),map
},[activityHeatmap]),activityHeatmapMax=useMemo(()=>Math.max(...activityHeatmap.map(entry=>Number(entry.count||0)),1),[activityHeatmap]),bootSavingsMomentum=useMemo(()=>{
if(dailySeries.length<4)return null;const recent=dailySeries.slice(-4),previous=dailySeries.slice(-8,-4);if(!previous.length)return null
;const recentAverage=recent.reduce((sum,point)=>sum+Number(point.saved||0),0)/recent.length,previousAverage=previous.reduce((sum,point)=>sum+Number(point.saved||0),0)/previous.length
;return previousAverage<=0?null:Math.round((recentAverage-previousAverage)/previousAverage*100)
},[dailySeries]),throughputSummary=useMemo(()=>summarizeBootThroughput(dailySeries,7),[dailySeries]),throughputBoots7d=throughputSummary.boots,throughputAvgPerDay7d=throughputSummary.avgPerDay,
throughputBoots30d=useMemo(()=>Number(savings?.summary?.totalBoots||0),[savings]),recentRecallWindow=useMemo(()=>recallTrendSeries.slice(-7),[recallTrendSeries]),latestRecallPoint=useMemo(()=>recallTrendSeries.at(-1)||null,
[recallTrendSeries]),stableRecallHeadlinePoint=useMemo(()=>latestRecallPoint?Number(latestRecallPoint.queries||0)>=RECALL_HEADLINE_MIN_QUERIES?latestRecallPoint:[...recentRecallWindow].reverse().find(point=>Number(point?.queries||0)>=RECALL_HEADLINE_MIN_QUERIES)||latestRecallPoint:null,
[latestRecallPoint,recentRecallWindow]),latestRecallHitRate=useMemo(()=>Math.round(Number(stableRecallHeadlinePoint?.hitRatePct||latestRecallPoint?.hitRatePct||0)),[latestRecallPoint,
stableRecallHeadlinePoint]),latestRecallSampleSize=useMemo(()=>Number(latestRecallPoint?.queries||0),[latestRecallPoint]),recallHeadlineUsesFallback=useMemo(()=>!!(latestRecallPoint&&stableRecallHeadlinePoint&&stableRecallHeadlinePoint!==latestRecallPoint&&latestRecallSampleSize<RECALL_HEADLINE_MIN_QUERIES),
[latestRecallPoint,latestRecallSampleSize,stableRecallHeadlinePoint]),recallWindowAverage=useMemo(()=>recentRecallWindow.length?Math.round(recentRecallWindow.reduce((sum,point)=>sum+Number(point.hitRatePct||0),
0)/recentRecallWindow.length):0,[recentRecallWindow]),recallWindowSpread=useMemo(()=>{
if(!recentRecallWindow.length)return 0;const values=recentRecallWindow.map(point=>Number(point.hitRatePct||0))
;return Math.round(Math.max(...values)-Math.min(...values))
},[recentRecallWindow]),monteCarloProjection=useMemo(()=>buildMonteCarloProjection(dailySeries,cumulativeSeries),[dailySeries,cumulativeSeries]),topFeedEntries=useMemo(()=>feedEntries.slice(0,
5),[feedEntries]),topActivityEntries=useMemo(()=>activityEntries.slice(0,5),[activityEntries]),topSavingsByAgent=useMemo(()=>[...Array.isArray(savings?.byAgent)?savings.byAgent:[]].sort((a,
b)=>Number(b.saved||0)-Number(a.saved||0)).slice(0,8),[savings?.byAgent]),sidebarUtilityStats=useMemo(()=>[{
label:"Queue",value:pendingTasks.length,tone:pendingTasks.length?"warning":"calm"},{label:"Locks",value:locks.length,tone:locks.length?"cyan":"calm"},{
label:"Recall",value:`${latestRecallHitRate||0}%`,tone:latestRecallHitRate>=85?"green":"warning"},{label:"Agents",value:normalizedSessions.length,
tone:normalizedSessions.length?"cyan":"calm"
}],[pendingTasks.length,locks.length,latestRecallHitRate,normalizedSessions.length]),runtimeVersionMismatch=useMemo(()=>!!healthMeta.runtimeVersion&&healthMeta.runtimeVersion!==CONTROL_CENTER_VERSION,
[healthMeta.runtimeVersion]),daemonStarting=useMemo(()=>isDaemonStartingState(daemonState),[daemonState.reachable,daemonState.running]),daemonStatusBadge=useMemo(()=>daemonStarting?{
className:"warning",label:"◌ STARTING",title:daemonState.message||"Cortex daemon process is running but not reachable yet."
}:daemonState.reachable?healthMeta.dbCorrupted?{className:"warning",label:"▲ DB WARN",
title:"Database integrity checks are failing. Restart Cortex to trigger repair."}:daemonTimeoutStaleSummary?{className:"warning",label:"▲ STALE",
title:`Daemon reachable, but recent IPC requests timed out. ${daemonTimeoutStaleSummary}`}:healthMeta.degraded?{className:"warning",label:"▲ DEGRADED",
title:"Semantic search is in fallback mode. Restart Cortex if this persists."}:{className:"online",label:"● ONLINE",
title:daemonState.message||"Cortex daemon reachable."}:{className:"offline",label:"○ OFFLINE",
title:daemonState.message||`Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`
},[cortexBase,daemonStarting,daemonState.message,daemonState.reachable,daemonTimeoutStaleSummary,healthMeta.dbCorrupted,healthMeta.degraded]),daemonRecoveryHint=useMemo(()=>daemonStarting?"Daemon process is up but still initializing. Control Center will keep retrying with bounded backoff.":daemonState.reachable?healthMeta.dbCorrupted?"Database integrity checks are failing. Restart Cortex to trigger repair and inspect the daemon if it stays degraded.":daemonTimeoutStaleSummary?"Daemon is reachable, but recent IPC requests timed out. Core and panel data may be temporarily stale.":runtimeVersionMismatch?`Connected to daemon v${healthMeta.runtimeVersion}. Restart from Control Center to switch to v${CONTROL_CENTER_VERSION}.`:healthMeta.degraded?"Semantic search is using keyword fallback right now. Restart Cortex if this state does not clear.":"":"",
[daemonStarting,daemonState.reachable,daemonTimeoutStaleSummary,healthMeta.dbCorrupted,healthMeta.degraded,healthMeta.runtimeVersion,runtimeVersionMismatch]),reportSurfaceError=useCallback(error=>{
const message=error?.message||String(error)
;!message||isDaemonSuppressibleErrorMessage(message)||setFeedbackMessage(summarizeDashboardErrors([message])||message)
},[]),handleTaskClaim=useCallback(async task=>{const operator=selectedOperatorName;if(!operator){setFeedbackMessage("Select an operator before claiming tasks.")
;return}setBusyActionKey(`claim:${task.taskId}`);try{await postApi("/tasks/claim",{taskId:task.taskId,agent:operator}),
setFeedbackMessage(`Claimed ${task.title}.`),await refreshCoreData()}catch(error){reportSurfaceError(error)}finally{setBusyActionKey("")}
},[postApi,refreshCoreData,reportSurfaceError,selectedOperatorName]),handleTaskAbandon=useCallback(async task=>{const operator=selectedOperatorName
;if(!operator){setFeedbackMessage("Select an operator before abandoning tasks.");return}setBusyActionKey(`abandon:${task.taskId}`);try{
await postApi("/tasks/abandon",{taskId:task.taskId,agent:operator}),setFeedbackMessage(`Returned ${task.title} to pending.`),setCompletionTaskId(""),
await refreshCoreData()}catch(error){reportSurfaceError(error)}finally{setBusyActionKey("")}
},[postApi,refreshCoreData,reportSurfaceError,selectedOperatorName]),handleTaskComplete=useCallback(async(task,summary)=>{const operator=selectedOperatorName
;if(!operator){setFeedbackMessage("Select an operator before completing tasks.");return}setBusyActionKey(`complete:${task.taskId}`);try{
await postApi("/tasks/complete",{taskId:task.taskId,agent:operator,summary:summary.trim()||void 0}),setFeedbackMessage(`Completed ${task.title}.`),
setCompletionTaskId(""),setTaskCompletionDrafts(current=>({...current,[task.taskId]:""})),await Promise.all([refreshCoreData(),refreshFeed()])}catch(error){
reportSurfaceError(error)}finally{setBusyActionKey("")}
},[postApi,refreshCoreData,refreshFeed,reportSurfaceError,selectedOperatorName]),handleTaskDelete=useCallback(async task=>{
setBusyActionKey(`delete:${task.taskId}`);try{await postApi("/tasks/delete",{taskId:task.taskId}),setFeedbackMessage(`Deleted ${task.title}.`),
await refreshCoreData()}catch(error){reportSurfaceError(error)}finally{setBusyActionKey("")}
},[postApi,refreshCoreData,reportSurfaceError]),handleUnlock=useCallback(async lock=>{const operator=selectedOperatorName;if(!operator){
setFeedbackMessage("Select an operator before unlocking files.");return}setBusyActionKey(`unlock:${lock.path}`);try{await postApi("/unlock",{path:lock.path,
agent:operator}),setFeedbackMessage(`Unlocked ${lock.path}.`),await refreshCoreData()}catch(error){reportSurfaceError(error)}finally{setBusyActionKey("")}
},[postApi,refreshCoreData,reportSurfaceError,selectedOperatorName]),handleSendMessage=useCallback(async event=>{event?.preventDefault()
;const operator=selectedOperatorName,recipient=messageTargetName,message=messageDraft.trim();if(!operator){
setFeedbackMessage("Select an operator before sending messages.");return}if(!recipient){setFeedbackMessage("Choose a recipient before sending a message.")
;return}if(!message){setFeedbackMessage("Write a message before sending it.");return}setBusyActionKey("message:send");try{await postApi("/message",{
from:operator,to:recipient,message:message}),setMessageDraft(""),setFeedbackMessage(`Sent message from ${operator} to ${recipient}.`),await refreshMessages()
}catch(error){reportSurfaceError(error)}finally{setBusyActionKey("")}
},[messageDraft,messageTargetName,postApi,refreshMessages,reportSurfaceError,selectedOperatorName]),handleFeedAck=useCallback(async()=>{
const operator=selectedOperatorName,lastSeenId=nextFeedAckId(feedEntries,operator);if(!operator){
setFeedbackMessage("Select an operator before acknowledging feed entries.");return}if(!lastSeenId){
setFeedbackMessage("No visible teammate feed entries to acknowledge.");return}setBusyActionKey("feed:ack");try{await postApi("/feed/ack",{agent:operator,
lastSeenId:lastSeenId}),setFeedbackMessage(`Acknowledged the visible feed for ${operator}.`),await refreshFeed()}catch(error){reportSurfaceError(error)}finally{
setBusyActionKey("")}},[feedEntries,postApi,refreshFeed,reportSurfaceError,selectedOperatorName]);return{...ctx,pendingTasks:pendingTasks,
claimedTasks:claimedTasks,completedTasks:completedTasks,recentOverviewTasks:recentOverviewTasks,utilityPill:utilityPill,daemonSysStatus:daemonSysStatus,
operationRows:operationRows,operationMaxSaved:operationMaxSaved,dailySeries:dailySeries,cumulativeSeries:cumulativeSeries,
cumulativeLatestTotal:cumulativeLatestTotal,recallTrendSeries:recallTrendSeries,activityHeatmap:activityHeatmap,activityHeatmapLookup:activityHeatmapLookup,
activityHeatmapMax:activityHeatmapMax,bootSavingsMomentum:bootSavingsMomentum,throughputSummary:throughputSummary,throughputBoots30d:throughputBoots30d,
recentRecallWindow:recentRecallWindow,latestRecallPoint:latestRecallPoint,stableRecallHeadlinePoint:stableRecallHeadlinePoint,
latestRecallHitRate:latestRecallHitRate,latestRecallSampleSize:latestRecallSampleSize,recallHeadlineUsesFallback:recallHeadlineUsesFallback,
recallWindowAverage:recallWindowAverage,recallWindowSpread:recallWindowSpread,monteCarloProjection:monteCarloProjection,topFeedEntries:topFeedEntries,
topActivityEntries:topActivityEntries,topSavingsByAgent:topSavingsByAgent,sidebarUtilityStats:sidebarUtilityStats,runtimeVersionMismatch:runtimeVersionMismatch,
daemonStarting:daemonStarting,daemonStatusBadge:daemonStatusBadge,daemonRecoveryHint:daemonRecoveryHint,reportSurfaceError:reportSurfaceError,
handleTaskClaim:handleTaskClaim,handleTaskAbandon:handleTaskAbandon,handleTaskComplete:handleTaskComplete,handleTaskDelete:handleTaskDelete,
handleUnlock:handleUnlock,handleSendMessage:handleSendMessage,handleFeedAck:handleFeedAck}}export{useDashboardEffects};

import React from "react";
import { EmptyItem } from "../components/common.jsx";
import { AgentItem } from "../components/AgentItem.jsx";
import { OperatorSelector } from "../components/OperatorSelector.jsx";
import { MessageItem } from "../components/MessageItem.jsx";
import { ActivityItem } from "../components/ActivityItem.jsx";
function AgentsPanel(p) {
  const {
    panel,
    messageEntries,
    activityEntries,
    selectedOperator,
    setSelectedOperator,
    activitySince,
    setActivitySince,
    changePanel,
    normalizedSessions,
    knownAgents,
    runRefreshAll,
    refreshMessages,
    refreshActivity,
    reportSurfaceError,
  } = p;
  return (
    <React.Fragment>
      {panel === "agents" ? (
        <section className="panel active">
          <div className="panel-header">
            <div>
              <h1>Agents</h1>
              <p className="panel-subtitle">Sessions, messages, and recent activity in one place.</p>
            </div>
            <div className="surface-actions">
              <button type="button" className="btn-sm" onClick={runRefreshAll}>
                Refresh
              </button>
              <button type="button" className="btn-sm" onClick={() => changePanel("brain")}>
                Brain View
              </button>
            </div>
          </div>
          <div className="surface-grid agents-grid">
            <div className="card agents-card-span-2">
              <div className="card-header">
                <h2>Active Sessions</h2>
                <span className="badge">{normalizedSessions.length}</span>
              </div>
              <ul className="item-list">
                {normalizedSessions.length ? (
                  normalizedSessions.map((session) => (
                    <AgentItem key={session.sessionId || session.agent} session={session} />
                  ))
                ) : (
                  <EmptyItem text="No agents online" />
                )}
              </ul>
            </div>
            <div className="card">
              <div className="card-header">
                <h2>Operator Inbox</h2>
                <span className="badge">{messageEntries.length}</span>
              </div>
              <div className="surface-toolbar">
                <OperatorSelector value={selectedOperator} knownAgents={knownAgents} onChange={setSelectedOperator} />
                <div className="surface-actions">
                  <button type="button" className="btn-sm" onClick={() => refreshMessages().catch(reportSurfaceError)}>
                    Refresh
                  </button>
                </div>
              </div>
              <ul className="item-list">
                {selectedOperator.trim() ? (
                  messageEntries.length ? (
                    messageEntries.map((entry) => <MessageItem key={entry.id} entry={entry} />)
                  ) : (
                    <EmptyItem text={`No inbox messages for ${selectedOperator.trim()}`} />
                  )
                ) : (
                  <EmptyItem text="Select an operator to view the inbox" />
                )}
              </ul>
            </div>
            <div className="card">
              <div className="card-header">
                <h2>Recent Activity</h2>
                <span className="badge">{activityEntries.length}</span>
              </div>
              <div className="surface-toolbar">
                <label className="feed-control">
                  <span>Since</span>
                  <select value={activitySince} onChange={(event) => setActivitySince(event.target.value)}>
                    <option value="15m">15m</option>
                    <option value="1h">1h</option>
                    <option value="4h">4h</option>
                    <option value="1d">1d</option>
                  </select>
                </label>
                <div className="surface-actions">
                  <button type="button" className="btn-sm" onClick={() => refreshActivity().catch(reportSurfaceError)}>
                    Refresh
                  </button>
                </div>
              </div>
              <ul className="item-list">
                {activityEntries.length ? (
                  activityEntries.map((entry) => <ActivityItem key={entry.id} entry={entry} />)
                ) : (
                  <EmptyItem text="No recent activity" />
                )}
              </ul>
            </div>
          </div>
        </section>
      ) : null}
    </React.Fragment>
  );
}
export { AgentsPanel };

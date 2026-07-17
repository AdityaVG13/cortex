import React from "react";
import { handleKeyboardActivation } from "../../keyboard-access.js";
import { conflictBadgeClass } from "../normalize/conflicts.js";
import { EmptyItem } from "../components/common.jsx";
import { ConflictPairCard } from "../components/ConflictPairCard.jsx";
function MemoryPanel(p) {
  const {
    panel,
    stats,
    memoryQuery,
    setMemoryQuery,
    memoryResults,
    memorySearching,
    conflictPairs,
    resolveDrafts,
    conflictLoading,
    permissionGrants,
    permissionLoading,
    permissionAccessDenied,
    permissionsEndpointAvailable,
    permissionDraft,
    setPermissionDraft,
    changePanel,
    handleMemorySearch,
    handleMemoryExpand,
    handleResolveConflict,
    handleResolveDraftChange,
    handleGrantPermission,
    handleRevokePermission,
    refreshConflicts,
    refreshPermissions,
    reportSurfaceError,
    pill,
    latestRecallHitRate,
    recallWindowAverage,
  } = p;
  return (
    <React.Fragment>
      {panel === "memory" ? (
        <section className="panel active">
          <div className="panel-header">
            <div>
              <h1>Memory</h1>
              <p className="panel-subtitle">
                Search the brain, inspect recall health, manage client permissions, and resolve conflicts without
                leaving the same tab.
              </p>
            </div>
            <div className="surface-actions">
              <button type="button" className="btn-sm" onClick={() => refreshConflicts().catch(reportSurfaceError)}>
                Refresh Conflicts
              </button>
              <button type="button" className="btn-sm" onClick={() => changePanel("analytics")}>
                Analytics
              </button>
            </div>
          </div>
          <div className="memory-layout">
            <div className="card full">
              <div className="card-header">
                <h2 id="memory-explorer-title">Memory Explorer</h2>
                <span className="badge" aria-live="polite">
                  {memoryResults.length}
                  <span className="sr-only">{" memory matches"}</span>
                </span>
              </div>
              <form className="memory-search" aria-labelledby="memory-explorer-title" onSubmit={handleMemorySearch}>
                <input
                  type="text"
                  className="memory-input"
                  aria-label="Search Cortex memories"
                  placeholder="Search the brain... (uses cortex_peek)"
                  value={memoryQuery}
                  onChange={(event) => setMemoryQuery(event.target.value)}
                />
                <button type="submit" className="btn-sm btn-primary" disabled={memorySearching}>
                  {memorySearching ? "Searching..." : "Peek"}
                </button>
              </form>
              {memoryResults.length > 0 ? (
                <div className="memory-stats">
                  <span className="badge">
                    {memoryResults.length}
                    {" matches"}
                  </span>
                  <span className="muted-inline">via cortex_peek -- click to expand full recall</span>
                </div>
              ) : null}
              <ul className="item-list">
                {memoryResults.length ? (
                  memoryResults.map((match, index) => (
                    <li
                      key={`${match.source}-${index}`}
                      className={`memory-item ${match.expanded ? "" : "memory-item-action"}`}
                      role={match.expanded ? void 0 : "button"}
                      tabIndex={match.expanded ? void 0 : 0}
                      aria-expanded={match.expanded ? void 0 : !1}
                      onClick={() => !match.expanded && handleMemoryExpand(match.source)}
                      onKeyDown={(event) =>
                        !match.expanded && handleKeyboardActivation(event, () => handleMemoryExpand(match.source))
                      }
                    >
                      <div className="memory-header">
                        <span className="memory-method">{match.method}</span>
                        <span className="memory-relevance">{(match.relevance * 100).toFixed(0)}%</span>
                      </div>
                      <div className="memory-source">{match.source}</div>
                      {match.expanded && match.excerpt ? <div className="memory-excerpt">{match.excerpt}</div> : null}
                      {match.expanded ? null : <div className="memory-expand-hint">Press Enter or Space to expand</div>}
                    </li>
                  ))
                ) : memoryQuery ? (
                  <EmptyItem text="No matches -- try different keywords" />
                ) : (
                  <EmptyItem text="Search to explore Cortex memories" />
                )}
              </ul>
            </div>
            <div className="memory-side-stack">
              <div className="card">
                <div className="card-header">
                  <h2>Memory Health</h2>
                  <span className="badge">{latestRecallHitRate || 0}%</span>
                </div>
                <div className="overview-status-list">
                  <div className="overview-status-row">
                    <span>Memories</span>
                    <strong>{stats.memories}</strong>
                  </div>
                  <div className="overview-status-row">
                    <span>Decisions</span>
                    <strong>{stats.decisions}</strong>
                  </div>
                  <div className="overview-status-row">
                    <span>7-day recall avg</span>
                    <strong>{recallWindowAverage || 0}%</strong>
                  </div>
                  <div className="overview-status-row">
                    <span>Open conflicts</span>
                    <strong>{conflictPairs.length}</strong>
                  </div>
                </div>
              </div>
              <div className="card">
                <div className="card-header">
                  <h2>Conflict Radar</h2>
                  <span className="badge">{conflictPairs.length}</span>
                </div>
                <ul className="item-list compact-list">
                  {conflictPairs.length ? (
                    conflictPairs.slice(0, 4).map((pair) => (
                      <li key={pair.key}>
                        <div className="item-meta">
                          <span className="item-name">
                            #{pair.left.id ?? "?"}
                            {" vs #"}
                            {pair.right.id ?? "?"}
                          </span>
                          <span className={conflictBadgeClass("conflict-pill conflict-class", pair.classification)}>
                            {pair.classification}
                          </span>
                        </div>
                        <div className="item-detail">
                          {pair.left.sourceAgent || "unknown"}
                          {" / "}
                          {pair.right.sourceAgent || "unknown"}
                          {" - "}
                          {pair.status}
                        </div>
                      </li>
                    ))
                  ) : (
                    <EmptyItem text="No active conflicts" />
                  )}
                </ul>
              </div>
              <div className="card">
                <div className="card-header">
                  <h2>Client Permissions</h2>
                  <span className="badge">{permissionGrants.length}</span>
                </div>
                {permissionsEndpointAvailable ? (
                  permissionAccessDenied ? (
                    <ul className="item-list compact-list permission-list">
                      <EmptyItem text="Permission controls require admin role in team mode." />
                    </ul>
                  ) : (
                    <React.Fragment>
                      <div className="permission-form">
                        <input
                          type="text"
                          className="memory-input"
                          aria-label="Client id for permission grant"
                          placeholder="client id (e.g. codex, claude, *)"
                          value={permissionDraft.client}
                          onChange={(event) =>
                            setPermissionDraft((current) => ({
                              ...current,
                              client: event.target.value,
                            }))
                          }
                        />
                        <div className="permission-form-row">
                          <label className="feed-control">
                            <span>Permission</span>
                            <select
                              value={permissionDraft.permission}
                              onChange={(event) =>
                                setPermissionDraft((current) => ({
                                  ...current,
                                  permission: event.target.value,
                                }))
                              }
                            >
                              <option value="read">read</option>
                              <option value="write">write</option>
                              <option value="admin">admin</option>
                            </select>
                          </label>
                          <label className="feed-control">
                            <span>Scope</span>
                            <input
                              type="text"
                              placeholder="* or tool name"
                              value={permissionDraft.scope}
                              onChange={(event) =>
                                setPermissionDraft((current) => ({
                                  ...current,
                                  scope: event.target.value,
                                }))
                              }
                            />
                          </label>
                        </div>
                        <div className="permission-actions">
                          <button
                            type="button"
                            className="btn-sm btn-primary"
                            disabled={permissionLoading}
                            onClick={() => handleGrantPermission().catch(reportSurfaceError)}
                          >
                            {permissionLoading ? "Applying..." : "Grant"}
                          </button>
                          <button
                            type="button"
                            className="btn-sm"
                            disabled={permissionLoading}
                            onClick={() => refreshPermissions({ force: !0 }).catch(reportSurfaceError)}
                          >
                            Refresh
                          </button>
                        </div>
                      </div>
                      <ul className="item-list compact-list permission-list">
                        {permissionGrants.length ? (
                          permissionGrants.slice(0, 8).map((grant) => (
                            <li key={grant.key}>
                              <div className="item-meta">
                                <span className="item-name">{grant.client}</span>
                                <span className="badge">{grant.permission}</span>
                              </div>
                              <div className="item-detail">
                                scope=
                                {grant.scope} {grant.grantedBy ? `- by ${grant.grantedBy}` : ""}
                              </div>
                              <div className="permission-item-actions">
                                <button
                                  type="button"
                                  className="btn-sm btn-danger"
                                  disabled={permissionLoading}
                                  onClick={() => handleRevokePermission(grant).catch(reportSurfaceError)}
                                >
                                  Revoke
                                </button>
                              </div>
                            </li>
                          ))
                        ) : (
                          <EmptyItem text="No explicit grants yet (legacy permissive mode)." />
                        )}
                      </ul>
                    </React.Fragment>
                  )
                ) : (
                  <div className="permission-form">
                    <div className="permission-actions">
                      <button
                        type="button"
                        className="btn-sm"
                        disabled={permissionLoading}
                        onClick={() => refreshPermissions({ force: !0 }).catch(reportSurfaceError)}
                      >
                        Recheck
                      </button>
                    </div>
                    <ul className="item-list compact-list permission-list">
                      <EmptyItem text="Permission endpoint unavailable on this daemon build." />
                    </ul>
                  </div>
                )}
              </div>
            </div>
          </div>
          <div className="memory-conflicts-section">
            <div className="panel-header panel-header-inline">
              <h2>Conflict Resolution</h2>
              <div className="panel-header-actions">
                <span className="badge">
                  {conflictPairs.length}
                  {" dispute"}
                  {conflictPairs.length !== 1 ? "s" : ""}
                </span>
                <button type="button" className="btn-sm" onClick={() => refreshConflicts().catch(reportSurfaceError)}>
                  Refresh
                </button>
              </div>
            </div>
            {conflictPairs.length === 0 ? (
              <div className="card full">
                <ul className="item-list">
                  <EmptyItem text="No active conflicts -- all decisions are in harmony" />
                </ul>
              </div>
            ) : (
              conflictPairs.map((pair) => (
                <ConflictPairCard
                  key={pair.key}
                  pair={pair}
                  conflictLoading={conflictLoading}
                  onResolveQuick={handleResolveConflict}
                  onResolveDraft={handleResolveConflict}
                  resolveDraft={resolveDrafts[pair.key]}
                  onResolveDraftChange={handleResolveDraftChange}
                />
              ))
            )}
          </div>
        </section>
      ) : null}
    </React.Fragment>
  );
}
export { MemoryPanel };

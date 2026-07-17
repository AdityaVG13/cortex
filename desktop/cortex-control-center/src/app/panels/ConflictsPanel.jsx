import React from "react";
import { EmptyItem } from "../components/common.jsx";
import { ConflictPairCard } from "../components/ConflictPairCard.jsx";
function ConflictsPanel(p) {
  const {
    panel,
    conflictPairs,
    resolveDrafts,
    conflictLoading,
    handleResolveConflict,
    handleResolveDraftChange,
    refreshConflicts,
    reportSurfaceError,
  } = p;
  return (
    <React.Fragment>
      {panel === "conflicts" ? (
        <section className="panel active">
          <div className="panel-header">
            <h1>Conflict Resolution</h1>
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
              <ul>
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
        </section>
      ) : null}
    </React.Fragment>
  );
}
export { ConflictsPanel };

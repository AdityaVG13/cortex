import { timeAgo } from "../../constants.js";
import {
  conflictBadgeClass,
  formatConfidencePercent,
  formatTimestamp,
  formatTrustScore,
} from "../normalize/conflicts.js";
import { agentColor } from "../utils/agent-color.js";

export function ConflictPairCard({
  pair,
  conflictLoading = false,
  onResolveQuick = null,
  onResolveDraft = null,
  resolveDraft = null,
  onResolveDraftChange = null,
}) {
  const draftAction = resolveDraft?.action || "keep";
  const draftWinner = resolveDraft?.winner || "left";
  const leftId = pair?.left?.id;
  const rightId = pair?.right?.id;
  const canResolve = leftId !== null && leftId !== undefined && rightId !== null && rightId !== undefined;
  const winner = draftWinner === "right" ? pair.right : pair.left;
  const loser = draftWinner === "right" ? pair.left : pair.right;

  return (
    <div key={pair.key} className="conflict-pair">
      <div className="conflict-topline">
        <div className="conflict-topline-left">
          <span className="conflict-id">Conflict #{pair.conflictId || pair.key}</span>
          <span className={conflictBadgeClass("conflict-pill conflict-class", pair.classification)}>{pair.classification}</span>
          <span className={conflictBadgeClass("conflict-pill conflict-status", pair.status)}>{pair.status}</span>
        </div>
        <div className="conflict-timestamps">
          <span>Created {formatTimestamp(pair.createdAt)}</span>
          {pair.resolvedAt ? <span>Resolved {formatTimestamp(pair.resolvedAt)}</span> : null}
        </div>
      </div>

      <div className="conflict-cards">
        <div className="card conflict-card">
          <div className="conflict-card-header">
            <span className="conflict-id">#{pair.left.id ?? "?"}</span>
            <span className="agent-indicator" style={{
              background: agentColor(pair.left.sourceAgent),
              boxShadow: `0 0 8px ${agentColor(pair.left.sourceAgent)}`,
            }} />
            <span className="item-name">{pair.left.sourceAgent || "unknown"}</span>
            <span className="muted-inline">{timeAgo(pair.left.createdAt)}</span>
          </div>
          <p className="conflict-text">{pair.left.decision}</p>
          {pair.left.context ? <p className="conflict-context">{pair.left.context}</p> : null}
          <div className="conflict-meta">
            <span>Confidence: {formatConfidencePercent(pair.left.confidence)}</span>
            <span>Trust: {formatTrustScore(pair.left.trustScore)}</span>
          </div>
        </div>

        <div className="conflict-vs">VS</div>

        <div className="card conflict-card">
          <div className="conflict-card-header">
            <span className="conflict-id">#{pair.right.id ?? "?"}</span>
            <span className="agent-indicator" style={{
              background: agentColor(pair.right.sourceAgent),
              boxShadow: `0 0 8px ${agentColor(pair.right.sourceAgent)}`,
            }} />
            <span className="item-name">{pair.right.sourceAgent || "unknown"}</span>
            <span className="muted-inline">{timeAgo(pair.right.createdAt)}</span>
          </div>
          <p className="conflict-text">{pair.right.decision}</p>
          {pair.right.context ? <p className="conflict-context">{pair.right.context}</p> : null}
          <div className="conflict-meta">
            <span>Confidence: {formatConfidencePercent(pair.right.confidence)}</span>
            <span>Trust: {formatTrustScore(pair.right.trustScore)}</span>
          </div>
        </div>
      </div>

      {pair.resolution ? (
        <div className="conflict-resolution-summary">
          <div className="conflict-resolution-grid">
            <span>
              <strong>Winner:</strong>{" "}
              {pair.resolution.winnerId !== null && pair.resolution.winnerId !== undefined
                ? `#${pair.resolution.winnerId}`
                : "n/a"}
              {pair.resolution.winnerAgent ? ` (${pair.resolution.winnerAgent})` : ""}
            </span>
            <span>
              <strong>Loser:</strong>{" "}
              {pair.resolution.loserId !== null && pair.resolution.loserId !== undefined
                ? `#${pair.resolution.loserId}`
                : "n/a"}
              {pair.resolution.loserAgent ? ` (${pair.resolution.loserAgent})` : ""}
            </span>
            {pair.resolution.action ? <span><strong>Action:</strong> {pair.resolution.action}</span> : null}
            {pair.resolution.method ? <span><strong>Method:</strong> {pair.resolution.method}</span> : null}
            {pair.resolution.resolvedBy ? <span><strong>Resolved by:</strong> {pair.resolution.resolvedBy}</span> : null}
            {pair.resolution.trustDelta !== null ? (
              <span className="conflict-trust-highlight"><strong>Trust delta:</strong> {pair.resolution.trustDelta.toFixed(3)}</span>
            ) : null}
          </div>
          {pair.resolution.notes ? <div className="conflict-resolution-notes">{pair.resolution.notes}</div> : null}
        </div>
      ) : null}

      <div className="conflict-actions">
        <button
          className="btn-sm btn-primary"
          disabled={conflictLoading || !canResolve}
          onClick={() => onResolveQuick?.(pair.left.id, "keep", pair.right.id, pair)}
        >
          Keep Left
        </button>
        <button
          className="btn-sm btn-primary"
          disabled={conflictLoading || !canResolve}
          onClick={() => onResolveQuick?.(pair.right.id, "keep", pair.left.id, pair)}
        >
          Keep Right
        </button>
        <button
          className="btn-sm"
          disabled={conflictLoading || !canResolve}
          onClick={() => onResolveQuick?.(pair.left.id, "merge", pair.right.id, pair)}
        >
          Merge Both
        </button>
        <button
          className="btn-sm btn-danger"
          disabled={conflictLoading || !canResolve}
          onClick={() => onResolveQuick?.(pair.left.id, "archive", pair.right.id, pair)}
        >
          Archive Both
        </button>
      </div>

      <div className="conflict-manual-controls">
        <span className="conflict-manual-label">Manual resolve</span>
        <label className="conflict-control-group">
          <span>Action</span>
          <select
            className="conflict-select"
            value={draftAction}
            onChange={(event) => onResolveDraftChange?.(pair.key, { action: event.target.value })}
          >
            <option value="keep">Keep</option>
            <option value="merge">Merge</option>
            <option value="archive">Archive</option>
          </select>
        </label>
        {draftAction === "keep" ? (
          <label className="conflict-control-group">
            <span>Winner</span>
            <select
              className="conflict-select"
              value={draftWinner}
              onChange={(event) => onResolveDraftChange?.(pair.key, { winner: event.target.value })}
            >
              <option value="left">Left ({pair.left.sourceAgent || "unknown"})</option>
              <option value="right">Right ({pair.right.sourceAgent || "unknown"})</option>
            </select>
          </label>
        ) : null}
        <button
          className="btn-sm btn-primary"
          disabled={conflictLoading || !canResolve}
          onClick={() => {
            if (draftAction === "keep") {
              onResolveDraft?.(winner.id, "keep", loser.id, pair);
              return;
            }
            if (draftAction === "merge") {
              onResolveDraft?.(pair.left.id, "merge", pair.right.id, pair);
              return;
            }
            onResolveDraft?.(pair.left.id, "archive", pair.right.id, pair);
          }}
        >
          Apply
        </button>
      </div>
    </div>
  );
}

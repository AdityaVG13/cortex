import React from "react";
import { canClaimTask, canFinalizeTask } from "../../live-surface.js";
import { timeAgo } from "../../constants.js";
function TaskItem({
  task,
  selectedOperator = "",
  completionDraft = "",
  completionExpanded = !1,
  onClaim = null,
  onAbandon = null,
  onComplete = null,
  onDelete = null,
  onCompletionDraftChange = null,
  onToggleComplete = null,
  busyActionKey = "",
}) {
  const operator = String(selectedOperator || "").trim(),
    claimBusy = busyActionKey === `claim:${task.taskId}`,
    abandonBusy = busyActionKey === `abandon:${task.taskId}`,
    completeBusy = busyActionKey === `complete:${task.taskId}`,
    deleteBusy = busyActionKey === `delete:${task.taskId}`,
    operatorOwnsTask = canFinalizeTask(task, operator),
    files = Array.isArray(task.files) ? task.files.slice(0, 4) : [],
    detail = task.claimedBy
      ? `${task.claimedBy}${task.summary ? ` \u2014 ${task.summary}` : ""} - ${timeAgo(task.claimedAt || task.completedAt)}`
      : task.project || "\u2014";
  return (
    <li>
      <div className="task-top">
        <span className={`status-dot ${task.status}`} />
        <span className={`priority priority-${task.priority}`}>{task.priority}</span>
        <span className="item-name">{task.title}</span>
      </div>
      <div className="item-detail">{detail}</div>
      {task.description ? <div className="item-detail">{task.description}</div> : null}
      {files.length ? (
        <div className="feed-files">
          {files.map((file) => (
            <span key={`${task.taskId}-${file}`} className="lock-path">
              {file}
            </span>
          ))}
        </div>
      ) : null}
      <div className="task-actions">
        {canClaimTask(task, operator) && onClaim ? (
          <button
            type="button"
            className="btn-sm btn-primary"
            aria-label={`Claim task ${task.title}`}
            disabled={claimBusy}
            onClick={() => onClaim(task)}
          >
            {claimBusy ? "Claiming..." : "Claim"}
          </button>
        ) : null}
        {task.status === "claimed" && operatorOwnsTask && onToggleComplete ? (
          <button
            type="button"
            className="btn-sm"
            aria-label={`${completionExpanded ? "Cancel completion for" : "Complete task"} ${task.title}`}
            disabled={completeBusy}
            onClick={() => onToggleComplete(task.taskId)}
          >
            {completionExpanded ? "Cancel Complete" : "Complete"}
          </button>
        ) : null}
        {task.status === "claimed" && operatorOwnsTask && onAbandon ? (
          <button
            type="button"
            className="btn-sm btn-danger"
            aria-label={`Abandon task ${task.title}`}
            disabled={abandonBusy}
            onClick={() => onAbandon(task)}
          >
            {abandonBusy ? "Abandoning..." : "Abandon"}
          </button>
        ) : null}
        {task.status === "claimed" && !operatorOwnsTask && task.claimedBy ? (
          <span className="surface-inline-hint">
            {"Held by "}
            {task.claimedBy}
          </span>
        ) : null}
        {task.status === "completed" && onDelete ? (
          <button
            type="button"
            className="btn-sm"
            aria-label={`Delete task ${task.title}`}
            disabled={deleteBusy}
            onClick={() => onDelete(task)}
          >
            {deleteBusy ? "Deleting..." : "Delete"}
          </button>
        ) : null}
      </div>
      {completionExpanded && operatorOwnsTask && onComplete && onCompletionDraftChange ? (
        <div className="task-complete-panel">
          <textarea
            value={completionDraft}
            onChange={(event) => onCompletionDraftChange(task.taskId, event.target.value)}
            aria-label={`Completion summary for ${task.title}`}
            placeholder="Optional completion summary for the task feed"
            rows={3}
          />
          <div className="surface-actions">
            <button
              type="button"
              className="btn-sm"
              aria-label={`Keep task ${task.title} open`}
              onClick={() => onToggleComplete?.(task.taskId)}
            >
              Keep Open
            </button>
            <button
              type="button"
              className="btn-sm btn-primary"
              aria-label={`Confirm complete task ${task.title}`}
              disabled={completeBusy}
              onClick={() => onComplete(task, completionDraft)}
            >
              {completeBusy ? "Completing..." : "Confirm Complete"}
            </button>
          </div>
        </div>
      ) : null}
    </li>
  );
}
export { TaskItem };

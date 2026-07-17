import React from "react";
import { useDashboard } from "../DashboardContext.jsx";
import { sameAgent } from "../../live-surface.js";
import { CardHeader, EmptyItem, ListCard, SurfaceStatGrid } from "../components/common.jsx";
import { OperatorSelector } from "../components/OperatorSelector.jsx";
import { TaskItem } from "../components/TaskItem.jsx";
import { LockItem } from "../components/LockItem.jsx";
import { FeedItem } from "../components/FeedItem.jsx";
import { MessageItem } from "../components/MessageItem.jsx";
function WorkPanel() { const { panel, tasks, locks, feedEntries, messageEntries, feedFilters,
    setFeedFilters, selectedOperator, setSelectedOperator, messageTarget, setMessageTarget, messageDraft, setMessageDraft, taskCompletionDrafts,
    setTaskCompletionDrafts, completionTaskId, setCompletionTaskId, busyActionKey, setFeedbackMessage, changePanel, knownAgents, selectedOperatorName,
    messageTargetName, runRefreshAll, handleTaskClaim, handleTaskAbandon, handleTaskComplete, handleTaskDelete, handleUnlock, handleSendMessage,
    handleFeedAck, refreshMessages, refreshFeed, reportSurfaceError, postApi, pendingTasks, claimedTasks, completedTasks, } = useDashboard();
  const workStats = [ { label: "Pending", value: pendingTasks.length },
    { label: "Claimed", value: claimedTasks.length }, { label: "Completed", value: completedTasks.length }, { label: "Locks", value: locks.length }, ];
  return ( <React.Fragment>
      {panel === "work" ? ( <section className="panel active">
          <div className="panel-header">
            <div>
              <h1>Work</h1>
              <p className="panel-subtitle">
                Queue, inbox, locks, and shared feed run through the same live operator surface.
              </p>
            </div>
            <div className="surface-actions">
              <button type="button" className="btn-sm" onClick={runRefreshAll}>
                Refresh
              </button>
              <button type="button" className="btn-sm" onClick={() => changePanel("agents")}>
                Agents
              </button>
            </div>
          </div>
          <div className="surface-toolbar work-operator-toolbar">
            <OperatorSelector value={selectedOperator} knownAgents={knownAgents} onChange={setSelectedOperator} />
            <div className="surface-actions">
              <span className="badge">{selectedOperator.trim() || "Unset"}</span>
              <span className="surface-inline-hint">Live actions use the selected operator label.</span>
            </div>
          </div>
          <SurfaceStatGrid stats={workStats} />
          <div className="work-grid">
            <div className="task-columns work-task-columns">
              <ListCard
                title="Pending"
                items={pendingTasks}
                emptyText="No pending tasks"
                renderItem={(task) => (
                  <TaskItem key={task.taskId} task={task} selectedOperator={selectedOperator} onClaim={handleTaskClaim} busyActionKey={busyActionKey} />
                )} />
              <div className="card">
                <CardHeader title="In Progress" badge={claimedTasks.length} />
                <ul className="item-list">
                  {claimedTasks.length ? ( claimedTasks.map((task) => ( <TaskItem
                        key={task.taskId}
                        task={task}
                        selectedOperator={selectedOperator}
                        completionDraft={taskCompletionDrafts[task.taskId] || ""}
                        completionExpanded={completionTaskId === task.taskId}
                        onAbandon={handleTaskAbandon}
                        onComplete={handleTaskComplete}
                        onCompletionDraftChange={(taskId, value) => { setTaskCompletionDrafts((current) => ({ ...current, [taskId]: value, }));
                        }}
                        onToggleComplete={(taskId) => { setCompletionTaskId((current) => (current === taskId ? "" : taskId));
                        }}
                        busyActionKey={busyActionKey} />
                    ))
                  ) : ( <EmptyItem text="Nothing in progress" />
                  )}
                </ul>
              </div>
              <div className="card">
                <div className="card-header">
                  <h2>Done</h2>
                  <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                    <span className="badge">{completedTasks.length}</span>
                    {completedTasks.length > 0 ? ( <button
                        type="button"
                        className="btn-sm"
                        onClick={async () => { try { const failed = ( await Promise.allSettled( completedTasks
                                  .filter((task) => task?.taskId)
                                  .map((task) => postApi("/tasks/delete", { taskId: task.taskId, }), ), )
                            ).filter((result) => result.status === "rejected");
                            (failed.length && setFeedbackMessage(`${failed.length} task delete(s) failed: ${failed[0].reason}`), await runRefreshAll());
                          } catch (error) { reportSurfaceError(error);
                          }
                        }} >
                        Clear
                      </button>
                    ) : null}
                  </div>
                </div>
                <ul className="item-list">
                  {completedTasks.length ? ( completedTasks
                      .slice(0, 10)
                      .map((task) => (
                        <TaskItem key={task.taskId} task={task} selectedOperator={selectedOperator} onDelete={handleTaskDelete} busyActionKey={busyActionKey} />
                      ))
                  ) : ( <EmptyItem text="No completed tasks" />
                  )}
                </ul>
              </div>
            </div>
            <div className="work-side-stack">
              <div className="card">
                <div className="card-header">
                  <h2>Operator Inbox</h2>
                  <span className="badge">{messageEntries.length}</span>
                </div>
                <div className="surface-toolbar">
                  <OperatorSelector value={selectedOperator} knownAgents={knownAgents} onChange={setSelectedOperator} />
                  <label className="feed-control">
                    <span>Recipient</span>
                    <input
                      type="text"
                      list="message-recipient-list"
                      placeholder="factory-droid"
                      value={messageTarget}
                      onChange={(event) => setMessageTarget(event.target.value)} />
                    <datalist id="message-recipient-list">
                      {knownAgents
                        .filter((agent) => !sameAgent(agent, selectedOperator))
                        .map((agent) => ( <option key={agent} value={agent} />
                        ))}
                    </datalist>
                  </label>
                  <div className="surface-actions">
                    <button
                      type="button"
                      className="btn-sm"
                      onClick={() => refreshMessages().catch(reportSurfaceError)} >
                      Refresh Inbox
                    </button>
                  </div>
                </div>
                <form className="surface-compose" onSubmit={handleSendMessage}>
                  <textarea
                    value={messageDraft}
                    onChange={(event) => setMessageDraft(event.target.value)}
                    aria-label={ selectedOperatorName && messageTargetName
                        ? `Message from ${selectedOperatorName} to ${messageTargetName}`
                        : "Operator message body"
                    }
                    placeholder={ selectedOperator.trim()
                        ? `Message from ${selectedOperator.trim()}`
                        : "Select an operator to send messages"
                    }
                    rows={3} />
                  <div className="surface-actions">
                    <button type="submit" className="btn-sm btn-primary" disabled={busyActionKey === "message:send"}>
                      {busyActionKey === "message:send" ? "Sending..." : "Send Message"}
                    </button>
                  </div>
                </form>
                <ul className="item-list compact-list">
                  {selectedOperator.trim() ? ( messageEntries.length ? ( messageEntries.map((entry) => <MessageItem key={entry.id} entry={entry} />)
                    ) : ( <EmptyItem text={`No inbox messages for ${selectedOperator.trim()}`} /> )
                  ) : ( <EmptyItem text="Select an operator to view the inbox" />
                  )}
                </ul>
              </div>
              <ListCard
                title="Locks"
                items={locks}
                emptyText="No active locks"
                renderItem={(lock) => ( <LockItem
                    key={lock.id || `${lock.path}:${lock.agent}`}
                    lock={lock}
                    selectedOperator={selectedOperator}
                    onUnlock={handleUnlock}
                    busyActionKey={busyActionKey} />
                )} />
              <div className="card">
                <div className="card-header">
                  <h2>Shared Feed</h2>
                  <span className="badge">{feedEntries.length}</span>
                </div>
                <div className="feed-toolbar work-feed-toolbar">
                  <label className="feed-control">
                    <span>Since</span>
                    <select
                      value={feedFilters.since}
                      onChange={(event) => setFeedFilters((current) => ({ ...current, since: event.target.value, }))
                      } >
                      <option value="15m">15m</option>
                      <option value="1h">1h</option>
                      <option value="4h">4h</option>
                      <option value="1d">1d</option>
                    </select>
                  </label>
                  <label className="feed-control">
                    <span>Kind</span>
                    <select
                      value={feedFilters.kind}
                      onChange={(event) => setFeedFilters((current) => ({ ...current, kind: event.target.value, }))
                      } >
                      <option value="all">All</option>
                      <option value="prompt">Prompt</option>
                      <option value="completion">Completion</option>
                      <option value="task_complete">Task Complete</option>
                      <option value="system">System</option>
                    </select>
                  </label>
                  <label className="feed-control">
                    <span>Agent</span>
                    <input
                      type="text"
                      placeholder="factory-droid"
                      value={feedFilters.agent}
                      onChange={(event) => setFeedFilters((current) => ({ ...current, agent: event.target.value, }))
                      } />
                  </label>
                  <div className="surface-actions">
                    <button
                      type="button"
                      className="btn-sm"
                      disabled={busyActionKey === "feed:ack" || !selectedOperator.trim()}
                      onClick={() => handleFeedAck().catch(reportSurfaceError)} >
                      {busyActionKey === "feed:ack" ? "Acking..." : "Acknowledge Visible"}
                    </button>
                    <button type="button" className="btn-sm" onClick={() => refreshFeed().catch(reportSurfaceError)}>
                      Refresh
                    </button>
                  </div>
                </div>
                <ul className="item-list">
                  {feedEntries.length ? ( feedEntries.map((entry) => <FeedItem key={entry.id} entry={entry} />)
                  ) : ( <EmptyItem text="No feed entries" />
                  )}
                </ul>
              </div>
            </div>
          </div>
        </section>
      ) : null}
    </React.Fragment> );
}
export { WorkPanel };

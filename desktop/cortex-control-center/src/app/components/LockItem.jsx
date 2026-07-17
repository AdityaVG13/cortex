import React from "react";
import { canUnlockLock } from "../../live-surface.js";
function LockItem({ lock, selectedOperator = "", onUnlock = null, busyActionKey = "" }) {
  const expiryMinutes = Math.max(0, Math.ceil((new Date(lock.expiresAt).getTime() - Date.now()) / 6e4)),
    unlockBusy = busyActionKey === `unlock:${lock.path}`,
    unlockable = canUnlockLock(lock, selectedOperator);
  return (
    <li>
      <div className="lock-path">{lock.path}</div>
      <div className="item-meta">
        <span className="lock-agent">{lock.agent}</span>
        <span className="lock-expiry">{expiryMinutes}m remaining</span>
      </div>
      {unlockable && onUnlock ? (
        <div className="task-actions">
          <button type="button" className="btn-sm" disabled={unlockBusy} onClick={() => onUnlock(lock)}>
            {unlockBusy ? "Unlocking..." : "Unlock"}
          </button>
        </div>
      ) : null}
    </li>
  );
}
export { LockItem };

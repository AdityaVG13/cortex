import React from "react";
import { canUnlockLock } from "../../live-surface.js";
function LockItem({
  lock,
  selectedOperator = "",
  onUnlock = null,
  busyActionKey = "",
}) {
  const expiryMinutes = Math.max(
      0,
      Math.ceil((new Date(lock.expiresAt).getTime() - Date.now()) / 6e4),
    ),
    unlockBusy = busyActionKey === `unlock:${lock.path}`,
    unlockable = canUnlockLock(lock, selectedOperator);
  return React.createElement(
    "li",
    null,
    React.createElement("div", { className: "lock-path" }, lock.path),
    React.createElement(
      "div",
      { className: "item-meta" },
      React.createElement("span", { className: "lock-agent" }, lock.agent),
      React.createElement(
        "span",
        { className: "lock-expiry" },
        expiryMinutes,
        "m remaining",
      ),
    ),
    unlockable && onUnlock
      ? React.createElement(
          "div",
          { className: "task-actions" },
          React.createElement(
            "button",
            {
              type: "button",
              className: "btn-sm",
              disabled: unlockBusy,
              onClick: () => onUnlock(lock),
            },
            unlockBusy ? "Unlocking..." : "Unlock",
          ),
        )
      : null,
  );
}
export { LockItem };

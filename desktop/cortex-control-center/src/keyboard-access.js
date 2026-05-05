const EDITABLE_TAGS = new Set(["INPUT", "SELECT", "TEXTAREA"]);

export function isKeyboardActivationKey(key) {
  return key === "Enter" || key === " " || key === "Spacebar";
}

export function handleKeyboardActivation(event, callback) {
  if (!isKeyboardActivationKey(event?.key)) {
    return false;
  }

  event.preventDefault();
  callback?.(event);
  return true;
}

export function shouldIgnoreGlobalShortcut(event, modalOpen = false) {
  if (modalOpen || !event) {
    return true;
  }

  if (event.altKey || event.ctrlKey || event.metaKey) {
    return true;
  }

  const target = event.target;
  if (!target || typeof target !== "object") {
    return false;
  }

  const tagName = String(target.tagName || "").toUpperCase();
  if (EDITABLE_TAGS.has(tagName) || target.isContentEditable) {
    return true;
  }

  if (typeof target.closest === "function") {
    return Boolean(target.closest(
      "button, a[href], [role='button'], [role='tab'], [role='switch'], [contenteditable='true']",
    ));
  }

  return false;
}

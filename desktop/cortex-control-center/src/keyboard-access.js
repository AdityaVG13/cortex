const EDITABLE_TAGS = new Set(["INPUT", "SELECT", "TEXTAREA"]);
const FOCUSABLE_SELECTOR = [
  "button:not([disabled])",
  "a[href]",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[role='button']",
  "[role='tab']",
  "[role='switch']",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

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

export function getFocusableElements(container) {
  if (!container || typeof container.querySelectorAll !== "function") {
    return [];
  }

  return Array.from(container.querySelectorAll(FOCUSABLE_SELECTOR)).filter((element) => {
    if (!element || element.disabled) return false;
    if (element.getAttribute?.("aria-hidden") === "true") return false;
    return Number(element.tabIndex ?? 0) >= 0;
  });
}

export function trapFocusInContainer(event, container, activeElement = globalThis.document?.activeElement) {
  if (event?.key !== "Tab" || !container) {
    return false;
  }

  const focusable = getFocusableElements(container);
  if (focusable.length === 0) {
    event.preventDefault();
    container.focus?.();
    return true;
  }

  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  const activeInside = typeof container.contains === "function" && container.contains(activeElement);

  if (!activeInside) {
    event.preventDefault();
    (event.shiftKey ? last : first).focus?.();
    return true;
  }

  if (event.shiftKey && activeElement === first) {
    event.preventDefault();
    last.focus?.();
    return true;
  }

  if (!event.shiftKey && activeElement === last) {
    event.preventDefault();
    first.focus?.();
    return true;
  }

  return false;
}

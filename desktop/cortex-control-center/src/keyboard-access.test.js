import { describe, expect, it, vi } from "vitest";

import {
  getFocusableElements,
  handleKeyboardActivation,
  isKeyboardActivationKey,
  shouldIgnoreGlobalShortcut,
  trapFocusInContainer,
} from "./keyboard-access.js";

function focusableElement({
  tabIndex = 0,
  disabled = false,
  hidden = false,
} = {}) {
  return {
    disabled,
    tabIndex,
    focus: vi.fn(),
    getAttribute: (name) => (hidden && name === "aria-hidden" ? "true" : null),
  };
}

describe("keyboard access helpers", () => {
  it("recognizes Enter and Space as activation keys", () => {
    expect(isKeyboardActivationKey("Enter")).toBe(true);
    expect(isKeyboardActivationKey(" ")).toBe(true);
    expect(isKeyboardActivationKey("Spacebar")).toBe(true);
    expect(isKeyboardActivationKey("ArrowDown")).toBe(false);
  });

  it("runs keyboard activation callbacks once and prevents page scroll", () => {
    const preventDefault = vi.fn();
    const callback = vi.fn();

    expect(
      handleKeyboardActivation({ key: " ", preventDefault }, callback),
    ).toBe(true);

    expect(preventDefault).toHaveBeenCalledTimes(1);
    expect(callback).toHaveBeenCalledTimes(1);
  });

  it("does not steal global shortcuts from form controls or buttons", () => {
    expect(shouldIgnoreGlobalShortcut({ target: { tagName: "INPUT" } })).toBe(
      true,
    );
    expect(
      shouldIgnoreGlobalShortcut({ target: { tagName: "TEXTAREA" } }),
    ).toBe(true);
    expect(shouldIgnoreGlobalShortcut({ target: { tagName: "SELECT" } })).toBe(
      true,
    );
    expect(
      shouldIgnoreGlobalShortcut({
        target: {
          tagName: "SPAN",
          closest: (selector) => (selector.includes("button") ? {} : null),
        },
      }),
    ).toBe(true);
  });

  it("keeps global shortcuts available from passive content only when no modal is open", () => {
    const passiveTarget = { tagName: "DIV", closest: () => null };

    expect(shouldIgnoreGlobalShortcut({ target: passiveTarget })).toBe(false);
    expect(shouldIgnoreGlobalShortcut({ target: passiveTarget }, true)).toBe(
      true,
    );
    expect(
      shouldIgnoreGlobalShortcut({ target: passiveTarget, ctrlKey: true }),
    ).toBe(true);
  });

  it("collects focusable dialog controls while skipping disabled and hidden controls", () => {
    const button = focusableElement();
    const disabledButton = focusableElement({ disabled: true });
    const hiddenButton = focusableElement({ hidden: true });
    const skipped = focusableElement({ tabIndex: -1 });
    const container = {
      querySelectorAll: () => [button, disabledButton, hiddenButton, skipped],
    };

    expect(getFocusableElements(container)).toEqual([button]);
  });

  it("keeps Tab navigation inside an active dialog", () => {
    const first = focusableElement();
    const last = focusableElement();
    const preventDefault = vi.fn();
    const container = {
      contains: (element) => element === first || element === last,
      querySelectorAll: () => [first, last],
    };

    expect(
      trapFocusInContainer(
        { key: "Tab", shiftKey: true, preventDefault },
        container,
        first,
      ),
    ).toBe(true);
    expect(preventDefault).toHaveBeenCalledTimes(1);
    expect(last.focus).toHaveBeenCalledTimes(1);

    expect(
      trapFocusInContainer(
        { key: "Tab", shiftKey: false, preventDefault },
        container,
        last,
      ),
    ).toBe(true);
    expect(first.focus).toHaveBeenCalledTimes(1);
  });

  it("moves focus into a dialog when Tab starts outside it", () => {
    const first = focusableElement();
    const preventDefault = vi.fn();
    const container = {
      contains: () => false,
      querySelectorAll: () => [first],
    };

    expect(
      trapFocusInContainer(
        { key: "Tab", shiftKey: false, preventDefault },
        container,
        {},
      ),
    ).toBe(true);
    expect(first.focus).toHaveBeenCalledTimes(1);
  });
});

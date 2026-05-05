import { describe, expect, it, vi } from "vitest";

import {
  handleKeyboardActivation,
  isKeyboardActivationKey,
  shouldIgnoreGlobalShortcut,
} from "./keyboard-access.js";

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

    expect(handleKeyboardActivation({ key: " ", preventDefault }, callback)).toBe(true);

    expect(preventDefault).toHaveBeenCalledTimes(1);
    expect(callback).toHaveBeenCalledTimes(1);
  });

  it("does not steal global shortcuts from form controls or buttons", () => {
    expect(shouldIgnoreGlobalShortcut({ target: { tagName: "INPUT" } })).toBe(true);
    expect(shouldIgnoreGlobalShortcut({ target: { tagName: "TEXTAREA" } })).toBe(true);
    expect(shouldIgnoreGlobalShortcut({ target: { tagName: "SELECT" } })).toBe(true);
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
    expect(shouldIgnoreGlobalShortcut({ target: passiveTarget }, true)).toBe(true);
    expect(shouldIgnoreGlobalShortcut({ target: passiveTarget, ctrlKey: true })).toBe(true);
  });
});

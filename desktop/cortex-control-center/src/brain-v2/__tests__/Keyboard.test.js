import { describe, expect, it } from "vitest";

import {
  brainKeyboardHelpText,
  isBrainNavigationKey,
  nextBrainNodeIndex,
} from "../Keyboard.js";

const nodes = [{ id: "a" }, { id: "b" }, { id: "c" }];

describe("Brain map keyboard navigation", () => {
  it("recognizes navigation keys without claiming text input keys", () => {
    expect(isBrainNavigationKey("ArrowRight")).toBe(true);
    expect(isBrainNavigationKey("Enter")).toBe(true);
    expect(isBrainNavigationKey("Escape")).toBe(true);
    expect(isBrainNavigationKey("a")).toBe(false);
  });

  it("moves through nodes with wrapping arrow keys", () => {
    expect(nextBrainNodeIndex({ key: "ArrowRight", currentIndex: -1, nodes })).toBe(0);
    expect(nextBrainNodeIndex({ key: "ArrowRight", currentIndex: 2, nodes })).toBe(0);
    expect(nextBrainNodeIndex({ key: "ArrowLeft", currentIndex: 0, nodes })).toBe(2);
  });

  it("supports Home, End, Enter, Space, and selected-id recovery", () => {
    expect(nextBrainNodeIndex({ key: "Home", currentIndex: 2, nodes })).toBe(0);
    expect(nextBrainNodeIndex({ key: "End", currentIndex: 0, nodes })).toBe(2);
    expect(nextBrainNodeIndex({ key: "Enter", currentIndex: -1, nodes })).toBe(0);
    expect(nextBrainNodeIndex({ key: " ", currentIndex: -1, selectedId: "b", nodes })).toBe(1);
  });

  it("clears selection on Escape and has a screen-reader instruction string", () => {
    expect(nextBrainNodeIndex({ key: "Escape", currentIndex: 1, nodes })).toBe(-1);
    expect(brainKeyboardHelpText(3)).toContain("Use arrow keys");
    expect(brainKeyboardHelpText(0)).toContain("No nodes");
  });
});

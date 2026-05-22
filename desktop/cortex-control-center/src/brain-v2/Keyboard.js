const BRAIN_NAVIGATION_KEYS = new Set([
  "ArrowRight",
  "ArrowDown",
  "ArrowLeft",
  "ArrowUp",
  "Home",
  "End",
  "Enter",
  " ",
  "Spacebar",
  "Escape",
]);

export function isBrainNavigationKey(key) {
  return BRAIN_NAVIGATION_KEYS.has(key);
}

export function nextBrainNodeIndex({ key, currentIndex = -1, selectedId = "", nodes = [] }) {
  const count = nodes.length;
  if (count === 0 || key === "Escape") {
    return -1;
  }

  let base = currentIndex;
  if (base < 0 && selectedId) {
    base = nodes.findIndex((node) => node?.id === selectedId);
  }

  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  if (key === "Enter" || key === " " || key === "Spacebar") {
    return base >= 0 ? base : 0;
  }

  if (key === "ArrowLeft" || key === "ArrowUp") {
    return base <= 0 ? count - 1 : base - 1;
  }

  if (key === "ArrowRight" || key === "ArrowDown") {
    return base < 0 ? 0 : (base + 1) % count;
  }

  return base;
}

export function brainKeyboardHelpText(nodeCount) {
  if (nodeCount > 0) {
    return `Cortex Brain Map with ${nodeCount} nodes. Use arrow keys, Home, or End to inspect nodes. Press Escape to clear the selected node.`;
  }
  return "Cortex Brain Map. No nodes are available yet.";
}

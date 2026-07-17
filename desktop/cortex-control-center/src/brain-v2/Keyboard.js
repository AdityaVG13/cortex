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
function isBrainNavigationKey(key) {
  return BRAIN_NAVIGATION_KEYS.has(key);
}
function nextBrainNodeIndex({
  key,
  currentIndex = -1,
  selectedId = "",
  nodes = [],
}) {
  const count = nodes.length;
  if (count === 0 || key === "Escape") return -1;
  let base = currentIndex;
  return (
    base < 0 &&
      selectedId &&
      (base = nodes.findIndex((node) => node?.id === selectedId)),
    key === "Home"
      ? 0
      : key === "End"
        ? count - 1
        : key === "Enter" || key === " " || key === "Spacebar"
          ? base >= 0
            ? base
            : 0
          : key === "ArrowLeft" || key === "ArrowUp"
            ? base <= 0
              ? count - 1
              : base - 1
            : key === "ArrowRight" || key === "ArrowDown"
              ? base < 0
                ? 0
                : (base + 1) % count
              : base
  );
}
function brainKeyboardHelpText(nodeCount) {
  return nodeCount > 0
    ? `Cortex Brain Map with ${nodeCount} nodes. Use arrow keys, Home, or End to inspect nodes. Press Escape to clear the selected node.`
    : "Cortex Brain Map. No nodes are available yet.";
}
export { brainKeyboardHelpText, isBrainNavigationKey, nextBrainNodeIndex };

function agentColor(name) {
  if (!name) return "var(--cyan)";
  const n = name.toLowerCase();
  return n.includes("claude")
    ? "var(--agent-claude)"
    : n.includes("droid") || n.includes("factory")
      ? "var(--agent-droid)"
      : n.includes("gemini")
        ? "var(--agent-gemini)"
        : n.includes("qwen") || n.includes("deepseek")
          ? "#22c55e"
          : "var(--cyan)";
}
export { agentColor };

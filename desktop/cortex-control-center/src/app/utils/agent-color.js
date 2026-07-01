export function agentColor(name) {
  if (!name) return "var(--cyan)";
  const n = name.toLowerCase();
  if (n.includes("claude")) return "var(--agent-claude)";
  if (n.includes("droid") || n.includes("factory")) return "var(--agent-droid)";
  if (n.includes("gemini")) return "var(--agent-gemini)";
  if (n.includes("qwen") || n.includes("deepseek")) return "#22c55e";
  return "var(--cyan)";
}

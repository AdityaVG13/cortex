export const AGENT_COLORS = {
  claude: "#4a9eff",
  droid: "#ff9800",
  "factory-droid": "#ff9800",
  gemini: "#a855f7",
  mcp: "#00d4ff",
  system: "#546580",
};

export function getAgentColor(agent) {
  if (!agent) return "#00d4ff";
  const key = agent.toLowerCase();
  for (const [k, v] of Object.entries(AGENT_COLORS)) {
    if (key.includes(k)) return v;
  }
  return "#00d4ff";
}

export function truncate(str, len) {
  if (!str) return "";
  return str.length > len ? str.slice(0, len) + "..." : str;
}

export function timeAgo(iso) {
  if (!iso) return "unknown";
  const minutes = Math.floor((Date.now() - new Date(iso).getTime()) / 60000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}h`;
  return `${Math.floor(minutes / 1440)}d`;
}

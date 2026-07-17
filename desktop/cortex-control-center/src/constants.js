const AGENT_COLORS = {
    claude: "#4a9eff",
    droid: "#ff9800",
    "factory-droid": "#ff9800",
    gemini: "#a855f7",
    mcp: "#00d4ff",
    system: "#546580",
  },
  CURRENCY_OPTIONS = [
    "USD",
    "EUR",
    "GBP",
    "INR",
    "JPY",
    "CAD",
    "AUD",
    "BRL",
    "KRW",
    "CNY",
  ],
  USD_TO_CURRENCY_RATE = {
    USD: 1,
    EUR: 0.92,
    GBP: 0.79,
    INR: 83.1,
    JPY: 151.2,
    CAD: 1.36,
    AUD: 1.52,
    BRL: 5.08,
    KRW: 1340,
    CNY: 7.24,
  },
  SAVINGS_OPERATION_LABELS = {
    boot: "Boot Compression",
    recall: "Recall Savings",
    store: "Store Savings",
    tool: "Tool-call Savings",
  };
function getAgentColor(agent) {
  if (!agent) return "#00d4ff";
  const key = agent.toLowerCase();
  for (const [k, v] of Object.entries(AGENT_COLORS))
    if (key.includes(k)) return v;
  return "#00d4ff";
}
function truncate(str, len) {
  return str ? (str.length > len ? str.slice(0, len) + "..." : str) : "";
}
function timeAgo(iso) {
  if (!iso) return "unknown";
  const minutes = Math.floor((Date.now() - new Date(iso).getTime()) / 6e4);
  return minutes < 1
    ? "now"
    : minutes < 60
      ? `${minutes}m`
      : minutes < 1440
        ? `${Math.floor(minutes / 60)}h`
        : `${Math.floor(minutes / 1440)}d`;
}
export {
  CURRENCY_OPTIONS,
  SAVINGS_OPERATION_LABELS,
  USD_TO_CURRENCY_RATE,
  timeAgo,
};

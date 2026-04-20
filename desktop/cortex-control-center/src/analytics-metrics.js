const DAY_MS = 24 * 60 * 60 * 1000;
const ISO_DAY_RE = /^\d{4}-\d{2}-\d{2}$/;

function toIsoUtcDay(date) {
  if (!(date instanceof Date) || Number.isNaN(date.getTime())) return "";
  const year = date.getUTCFullYear();
  const month = String(date.getUTCMonth() + 1).padStart(2, "0");
  const day = String(date.getUTCDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function parseIsoUtcDay(isoDay) {
  if (!ISO_DAY_RE.test(String(isoDay || ""))) return null;
  const parsed = new Date(`${isoDay}T00:00:00Z`);
  if (Number.isNaN(parsed.getTime())) return null;
  return parsed;
}

function trailingIsoDays(windowDays, nowDate = new Date()) {
  const safeWindow = Math.max(1, Math.floor(Number(windowDays) || 1));
  const now = nowDate instanceof Date && !Number.isNaN(nowDate.getTime()) ? nowDate : new Date();
  const endDay = parseIsoUtcDay(toIsoUtcDay(now));
  if (!endDay) return [];
  return Array.from({ length: safeWindow }, (_, index) => {
    const day = new Date(endDay.getTime() - (safeWindow - 1 - index) * DAY_MS);
    return toIsoUtcDay(day);
  });
}

function daysBetweenInclusive(startIsoDay, endIsoDay) {
  const start = parseIsoUtcDay(startIsoDay);
  const end = parseIsoUtcDay(endIsoDay);
  if (!start || !end || start.getTime() > end.getTime()) return 0;
  return Math.floor((end.getTime() - start.getTime()) / DAY_MS) + 1;
}

function normalizeBootRowsByDay(dailySeries) {
  const rows = Array.isArray(dailySeries) ? dailySeries : [];
  const byDay = new Map();
  for (const row of rows) {
    const day = String(row?.date || "");
    if (!ISO_DAY_RE.test(day)) continue;
    const boots = Number(row?.boots || 0);
    if (!Number.isFinite(boots)) continue;
    byDay.set(day, (byDay.get(day) || 0) + boots);
  }
  return byDay;
}

export function summarizeBootThroughput(dailySeries, windowDays = 7, nowDate = new Date()) {
  const safeWindow = Math.max(1, Math.floor(Number(windowDays) || 7));
  const byDay = normalizeBootRowsByDay(dailySeries);
  const windowDaysIso = trailingIsoDays(safeWindow, nowDate);
  const windowStart = windowDaysIso[0] || "";
  const windowEnd = windowDaysIso.at(-1) || "";
  const boots = windowDaysIso.reduce((sum, day) => sum + Number(byDay.get(day) || 0), 0);
  const sortedObservedDays = [...byDay.keys()].sort();
  const firstObservedDay = sortedObservedDays[0] || "";

  let daysRepresented = safeWindow;
  if (!firstObservedDay) {
    daysRepresented = 0;
  } else if (windowStart && firstObservedDay > windowStart) {
    daysRepresented = Math.min(safeWindow, daysBetweenInclusive(firstObservedDay, windowEnd));
  }

  const avgPerDay = daysRepresented > 0
    ? Math.round((boots / daysRepresented) * 10) / 10
    : 0;

  return {
    windowDays: safeWindow,
    daysRepresented,
    isPartialHistory: daysRepresented > 0 && daysRepresented < safeWindow,
    windowStart,
    windowEnd,
    boots,
    avgPerDay,
  };
}

const DAY_MS = 864e5, ISO_DAY_RE = /^\d{4}-\d{2}-\d{2}$/;
function toIsoUtcDay(date) { if (!(date instanceof Date) || Number.isNaN(date.getTime())) return "";
  const year = date.getUTCFullYear(), month = String(date.getUTCMonth() + 1).padStart(2, "0"), day = String(date.getUTCDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}
function parseIsoUtcDay(isoDay) { if (!ISO_DAY_RE.test(String(isoDay || ""))) return null;
  const parsed = new Date(`${isoDay}T00:00:00Z`);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}
function trailingIsoDays(windowDays, nowDate = new Date()) { const safeWindow = Math.max(1, Math.floor(Number(windowDays) || 1)),
    now = nowDate instanceof Date && !Number.isNaN(nowDate.getTime()) ? nowDate : new Date(), endDay = parseIsoUtcDay(toIsoUtcDay(now));
  return endDay
    ? Array.from({ length: safeWindow }, (_, index) => { const day = new Date(endDay.getTime() - (safeWindow - 1 - index) * 864e5);
        return toIsoUtcDay(day);
      })
    : [];
}
function daysBetweenInclusive(startIsoDay, endIsoDay) { const start = parseIsoUtcDay(startIsoDay), end = parseIsoUtcDay(endIsoDay);
  return !start || !end || start.getTime() > end.getTime()
    ? 0
    : Math.floor((end.getTime() - start.getTime()) / 864e5) + 1;
}
function normalizeBootRowsByDay(dailySeries) { const rows = Array.isArray(dailySeries) ? dailySeries : [], byDay = new Map();
  for (const row of rows) { const day = String(row?.date || "");
    if (!ISO_DAY_RE.test(day)) continue;
    const boots = Number(row?.boots || 0);
    Number.isFinite(boots) && byDay.set(day, (byDay.get(day) || 0) + boots);
  }
  return byDay;
}
function summarizeBootThroughput(dailySeries, windowDays = 7, nowDate = new Date()) { const safeWindow = Math.max(1, Math.floor(Number(windowDays) || 7)),
    byDay = normalizeBootRowsByDay(dailySeries), windowDaysIso = trailingIsoDays(safeWindow, nowDate),
    windowStart = windowDaysIso[0] || "", windowEnd = windowDaysIso.at(-1) || "",
    boots = windowDaysIso.reduce((sum, day) => sum + Number(byDay.get(day) || 0), 0), firstObservedDay = [...byDay.keys()].sort()[0] || "";
  let daysRepresented = safeWindow;
  firstObservedDay
    ? windowStart && firstObservedDay > windowStart && (daysRepresented = Math.min(safeWindow, daysBetweenInclusive(firstObservedDay, windowEnd)))
    : (daysRepresented = 0);
  const avgPerDay = daysRepresented > 0 ? Math.round((boots / daysRepresented) * 10) / 10 : 0;
  return { windowDays: safeWindow, daysRepresented, isPartialHistory: daysRepresented > 0 && daysRepresented < safeWindow,
    windowStart, windowEnd, boots, avgPerDay, };
}
export { summarizeBootThroughput };

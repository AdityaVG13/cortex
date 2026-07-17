function createSeededRng(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 1831565813) >>> 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    return (
      (t ^= t + Math.imul(t ^ (t >>> 7), 61 | t)),
      ((t ^ (t >>> 14)) >>> 0) / 4294967296
    );
  };
}
function gaussianRandom(rng) {
  let u = 0,
    v = 0;
  for (; u === 0;) u = rng();
  for (; v === 0;) v = rng();
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
}
function percentileFromSorted(sorted, percentile) {
  if (!sorted.length) return 0;
  const index = (sorted.length - 1) * percentile,
    lower = Math.floor(index),
    upper = Math.ceil(index);
  if (lower === upper) return sorted[lower];
  const weight = index - lower;
  return sorted[lower] * (1 - weight) + sorted[upper] * weight;
}
function clampNumber(value, min, max) {
  return Math.min(Math.max(value, min), max);
}
const ABSOLUTE_DAILY_BASIS_CAP = 1e9,
  ABSOLUTE_PROJECTED_GAIN_CAP = ABSOLUTE_DAILY_BASIS_CAP * 90 * 2;
function projectionBasisFromSeries(dailySeries, cumulativeSeries) {
  const dailyBasis = (Array.isArray(dailySeries) ? dailySeries : [])
    .map((point) => Number(point?.saved || 0))
    .filter((value) => Number.isFinite(value) && value > 0);
  return dailyBasis.length
    ? dailyBasis
    : (Array.isArray(cumulativeSeries) ? cumulativeSeries : [])
        .map((point) => Number(point?.savedDelta || 0))
        .filter((value) => Number.isFinite(value) && value > 0);
}
function sanitizeProjectionBasis(basis) {
  if (!Array.isArray(basis) || basis.length < 2) return [];
  const finite = basis.filter((value) => Number.isFinite(value) && value > 0);
  if (finite.length < 2) return [];
  const sorted = [...finite].sort((left, right) => left - right),
    median = percentileFromSorted(sorted, 0.5),
    upperLimit = Math.min(Math.max(median * 40, 1), ABSOLUTE_DAILY_BASIS_CAP),
    lowerLimit = Math.max(median * 0.02, 1);
  return finite.map((value) => clampNumber(value, lowerLimit, upperLimit));
}
function buildMonteCarloProjection(
  dailySeries,
  cumulativeSeries,
  horizonDays = 30,
  simulationCount = 64,
) {
  const safeHorizonDays = Math.max(
      1,
      Math.min(90, Math.floor(Number(horizonDays) || 30)),
    ),
    safeSimulationCount = Math.max(
      20,
      Math.min(1e3, Math.floor(Number(simulationCount) || 64)),
    ),
    basis = sanitizeProjectionBasis(
      projectionBasisFromSeries(dailySeries, cumulativeSeries),
    );
  if (basis.length < 2) return null;
  const recent = basis.slice(-14),
    recentAverage =
      recent.reduce((sum, value) => sum + value, 0) / recent.length,
    recentMedian = percentileFromSorted(
      [...recent].sort((left, right) => left - right),
      0.5,
    ),
    recentPeak = Math.max(...recent, 1),
    logReturns = [];
  for (let index = 1; index < recent.length; index += 1) {
    const previous = Math.max(recent[index - 1], 1),
      current = Math.max(recent[index], 1);
    logReturns.push(clampNumber(Math.log(current / previous), -0.6, 0.6));
  }
  const rawDrift = logReturns.length
      ? logReturns.reduce((sum, value) => sum + value, 0) / logReturns.length
      : 0.012,
    shortHistory = recent.length < 4,
    drift = clampNumber(rawDrift, -0.08, shortHistory ? 0.05 : 0.12),
    variance = logReturns.length
      ? logReturns.reduce((sum, value) => sum + (value - rawDrift) ** 2, 0) /
        logReturns.length
      : 0.05,
    volatilityFloor = shortHistory ? 0.06 : 0.08,
    volatilityCeiling = shortHistory ? 0.22 : 0.35,
    volatility = clampNumber(
      Math.max(Math.sqrt(variance), volatilityFloor),
      volatilityFloor,
      volatilityCeiling,
    ),
    lastDaily = Math.max(recent[recent.length - 1], 1),
    startTotal = Number(
      cumulativeSeries?.at?.(-1)?.savedTotal ||
        cumulativeSeries?.at?.(-1)?.saved ||
        basis.reduce((sum, value) => sum + value, 0),
    ),
    boundedSeedBase = Number.isFinite(startTotal)
      ? Math.abs(startTotal % 1e9)
      : 0,
    rng = createSeededRng(
      Math.round(boundedSeedBase + lastDaily + recent.length * 13),
    ),
    meanReversionStrength = shortHistory ? 0.03 : 0.04,
    dailyCeiling = Math.min(
      Math.max(recentPeak * 4, recentAverage * 6, recentMedian * 10, 1),
      ABSOLUTE_DAILY_BASIS_CAP,
    ),
    maxProjectedGain = Math.min(
      dailyCeiling * safeHorizonDays * 2,
      ABSOLUTE_PROJECTED_GAIN_CAP,
    ),
    runs = Array.from({ length: safeSimulationCount }, (_, simIndex) => {
      let dailyValue = lastDaily,
        gainValue = 0;
      const series = [];
      for (let day = 0; day < safeHorizonDays; day += 1) {
        const shock = gaussianRandom(rng) * volatility,
          meanReversion =
            ((recentAverage - dailyValue) / Math.max(dailyValue, 1)) *
            meanReversionStrength,
          step = clampNumber(drift + meanReversion + shock, -0.6, 0.6),
          growth = Math.exp(step);
        ((dailyValue = clampNumber(dailyValue * growth, 0, dailyCeiling)),
          (gainValue = clampNumber(
            gainValue + dailyValue,
            0,
            maxProjectedGain,
          )),
          series.push({
            day: day + 1,
            daily: dailyValue,
            cumulative: startTotal + gainValue,
            gain: gainValue,
          }));
      }
      return {
        key: `sim-${simIndex}`,
        series,
        final: series.at(-1)?.gain || 0,
      };
    }),
    bandSeries = Array.from({ length: safeHorizonDays }, (_, dayIndex) => {
      const values = runs
        .map((run) => run.series[dayIndex]?.gain || 0)
        .sort((left, right) => left - right);
      return {
        day: dayIndex + 1,
        p10: percentileFromSorted(values, 0.1),
        p25: percentileFromSorted(values, 0.25),
        p50: percentileFromSorted(values, 0.5),
        p75: percentileFromSorted(values, 0.75),
        p90: percentileFromSorted(values, 0.9),
      };
    }),
    samples = runs
      .filter((_, index) => index % Math.ceil(safeSimulationCount / 14) === 0)
      .slice(0, 14)
      .map((run) => run.series.map((point) => point.gain)),
    endingValues = runs
      .map((run) => run.final)
      .sort((left, right) => left - right),
    summary = {
      startTotal,
      p10Gain: percentileFromSorted(endingValues, 0.1),
      p50Gain: percentileFromSorted(endingValues, 0.5),
      p90Gain: percentileFromSorted(endingValues, 0.9),
      avgDaily: recentAverage,
    };
  return (
    (summary.p10Total = startTotal + summary.p10Gain),
    (summary.p50Total = startTotal + summary.p50Gain),
    (summary.p90Total = startTotal + summary.p90Gain),
    {
      bandSeries,
      samples,
      summary,
      horizonDays: safeHorizonDays,
      simulationCount: safeSimulationCount,
    }
  );
}
export { buildMonteCarloProjection };

function createSeededRng(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t ^= t + Math.imul(t ^ (t >>> 7), 61 | t);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function gaussianRandom(rng) {
  let u = 0;
  let v = 0;
  while (u === 0) u = rng();
  while (v === 0) v = rng();
  return Math.sqrt(-2.0 * Math.log(u)) * Math.cos(2.0 * Math.PI * v);
}

function percentileFromSorted(sorted, percentile) {
  if (!sorted.length) return 0;
  const index = (sorted.length - 1) * percentile;
  const lower = Math.floor(index);
  const upper = Math.ceil(index);
  if (lower === upper) return sorted[lower];
  const weight = index - lower;
  return sorted[lower] * (1 - weight) + sorted[upper] * weight;
}

function clampNumber(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

function projectionBasisFromSeries(dailySeries, cumulativeSeries) {
  const dailyBasis = (Array.isArray(dailySeries) ? dailySeries : [])
    .map((point) => Number(point?.saved || 0))
    .filter((value) => Number.isFinite(value) && value > 0);
  if (dailyBasis.length) return dailyBasis;

  return (Array.isArray(cumulativeSeries) ? cumulativeSeries : [])
    .map((point) => Number(point?.savedDelta || 0))
    .filter((value) => Number.isFinite(value) && value > 0);
}

export function buildMonteCarloProjection(dailySeries, cumulativeSeries, horizonDays = 30, simulationCount = 180) {
  const basis = projectionBasisFromSeries(dailySeries, cumulativeSeries);
  if (basis.length < 2) return null;

  const recent = basis.slice(-14);
  const recentAverage = recent.reduce((sum, value) => sum + value, 0) / recent.length;
  const logReturns = [];
  for (let index = 1; index < recent.length; index += 1) {
    const previous = Math.max(recent[index - 1], 1);
    const current = Math.max(recent[index], 1);
    logReturns.push(clampNumber(Math.log(current / previous), -0.6, 0.6));
  }

  const rawDrift = logReturns.length
    ? logReturns.reduce((sum, value) => sum + value, 0) / logReturns.length
    : 0.012;
  const shortHistory = recent.length < 4;
  const drift = clampNumber(rawDrift, -0.08, shortHistory ? 0.05 : 0.12);
  const variance = logReturns.length
    ? logReturns.reduce((sum, value) => sum + (value - rawDrift) ** 2, 0) / logReturns.length
    : 0.05;
  const volatilityFloor = shortHistory ? 0.06 : 0.08;
  const volatilityCeiling = shortHistory ? 0.22 : 0.35;
  const volatility = clampNumber(Math.max(Math.sqrt(variance), volatilityFloor), volatilityFloor, volatilityCeiling);
  const lastDaily = Math.max(recent[recent.length - 1], 1);
  const startTotal = Number(
    cumulativeSeries?.at?.(-1)?.savedTotal
    || cumulativeSeries?.at?.(-1)?.saved
    || basis.reduce((sum, value) => sum + value, 0)
  );
  const rng = createSeededRng(Math.round(startTotal + lastDaily + recent.length * 13));
  const meanReversionStrength = shortHistory ? 0.03 : 0.04;

  const runs = Array.from({ length: simulationCount }, (_, simIndex) => {
    let dailyValue = lastDaily;
    let cumulativeValue = startTotal;
    const series = [];
    for (let day = 0; day < horizonDays; day += 1) {
      const shock = gaussianRandom(rng) * volatility;
      const meanReversion = ((recentAverage - dailyValue) / Math.max(dailyValue, 1)) * meanReversionStrength;
      const step = clampNumber(drift + meanReversion + shock, -0.6, 0.6);
      const growth = Math.exp(step);
      dailyValue = Math.max(0, dailyValue * growth);
      cumulativeValue += dailyValue;
      series.push({
        day: day + 1,
        daily: dailyValue,
        cumulative: cumulativeValue,
        gain: cumulativeValue - startTotal,
      });
    }
    return {
      key: `sim-${simIndex}`,
      series,
      final: cumulativeValue - startTotal,
    };
  });

  const bandSeries = Array.from({ length: horizonDays }, (_, dayIndex) => {
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
  });

  const samples = runs
    .filter((_, index) => index % Math.ceil(simulationCount / 14) === 0)
    .slice(0, 14)
    .map((run) => run.series.map((point) => point.gain));

  const endingValues = runs.map((run) => run.final).sort((left, right) => left - right);
  const summary = {
    startTotal,
    p10Gain: percentileFromSorted(endingValues, 0.1),
    p50Gain: percentileFromSorted(endingValues, 0.5),
    p90Gain: percentileFromSorted(endingValues, 0.9),
    avgDaily: recentAverage,
  };

  summary.p10Total = startTotal + summary.p10Gain;
  summary.p50Total = startTotal + summary.p50Gain;
  summary.p90Total = startTotal + summary.p90Gain;

  return { bandSeries, samples, summary, horizonDays, simulationCount };
}

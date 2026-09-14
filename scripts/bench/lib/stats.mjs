// Statistics + timing helpers for the perf baseline harness. Zero deps.

export function percentile(sortedAsc, p) {
  if (sortedAsc.length === 0) return NaN;
  const idx = Math.min(
    sortedAsc.length - 1,
    Math.ceil((p / 100) * sortedAsc.length) - 1,
  );
  return sortedAsc[Math.max(0, idx)];
}

export function summarize(values) {
  const a = values
    .filter((v) => Number.isFinite(v))
    .slice()
    .sort((x, y) => x - y);
  if (a.length === 0) return { n: 0 };
  const mean = a.reduce((s, v) => s + v, 0) / a.length;
  return {
    n: a.length,
    min: a[0],
    mean,
    p50: percentile(a, 50),
    p90: percentile(a, 90),
    p95: percentile(a, 95),
    p99: percentile(a, 99),
    p999: percentile(a, 99.9),
    max: a[a.length - 1],
  };
}

export function round(v, d = 3) {
  return Number.isFinite(v) ? Number(v.toFixed(d)) : v;
}

export function roundStats(s) {
  if (!s || s.n === 0) return s;
  const o = { n: s.n };
  for (const k of ["min", "mean", "p50", "p90", "p95", "p99", "p999", "max"]) {
    o[k] = round(s[k]);
  }
  return o;
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export const nowMs = () => performance.now();

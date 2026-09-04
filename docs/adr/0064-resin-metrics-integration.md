# ADR-0064: Resin metrics integration — minimal set first (2 of 12 endpoints)

- **Status**: ACCEPTED
- **Date**: 2026-09-04 (round5 T19 / 裂痕 #5 部分治理 — 波次 W4)
- **Supersedes**: nothing (new decision area; does not touch ADR-0050, which
  deleted the dead kernel face — this ADR only adds client methods, it does
  not resurrect any local metrics implementation).
- **Research basis**: `resin/internal/api/handler_metrics.go:146-516`
  (upstream handler source, read verbatim); reports/03 §2 row #5 + §5 metrics
  row; T13 `docs/architecture/RESIN_API_COVERAGE.md` rows R47–R58.

## Context

Resin v1.2.0 registers 12 metrics endpoints under `/api/v1/metrics/*`:

- **realtime × 3** (`throughput` / `connections` / `leases`) — ring-buffer
  samples, `parseMetricsTimeRange` defaults apply (to=now, from=to-1h);
- **history × 6** (`traffic` / `requests` / `access-latency` / `probes` /
  `node-pool` / `lease-lifetime`) — metrics.db buckets;
- **snapshots × 3** (`node-pool` / `platform-node-pool` /
  `node-latency-distribution`) — unpersisted.

The shell integrated exactly one of them (`snapshots/node-pool` →
`node_pool_snapshot`, coverage row R56). The remaining 11 were blank rows in
RESIN_API_COVERAGE.md, which made diagnostics like "过去 24h 出口 IP 切换次数"
(impossible without probe history) unavailable despite the upstream data
existing. Round 5 spec §7.3-3 asked the user to ratify the scope; the ticket
default (issue F1) is the **minimal set of 2 endpoints**, ratified as:

- `GET /api/v1/metrics/history/probes` (R53) → probe-count history buckets
- `GET /api/v1/metrics/realtime/throughput` (R47) → realtime ingress/egress ring

## Decision

**D1 — Minimal set now, full set later.** This round wraps exactly the 2
endpoints above with `ResinClient::probe_history` +
`ResinClient::realtime_throughput` (crates/resin-core/src/resin_client.rs), 2
IPC commands (`metrics_probe_history`, `metrics_realtime_throughput` in
src-tauri/src/commands/diagnostics.rs), 2 TS wrappers
(`ipcMetricsProbeHistory`, `ipcMetricsRealtimeThroughput` in src/lib/ipc.ts),
and a DiagnosticsView card. The other 10 endpoints (R48/R50/R51/R52/R54/R55/
R57/R58 + realtime connections) stay documented blank in
RESIN_API_COVERAGE.md; expanding to the full set (issue F3: 14 new commands,
full Metrics tab) is a separate round decision and must revisit this ADR.

**D2 — from/to are RFC3339, not Unix seconds.** Issue F4 drafted the
validation as "Unix timestamp"; the upstream source is authoritative:
`handler_metrics.go:15` parses `time.RFC3339Nano` and returns
`400 INVALID_ARGUMENT` for anything else. The shell therefore validates and
passes RFC3339 strings. The IPC boundary (§7.5) rejects, before Resin is
called:

- non-RFC3339 from/to (TS regex + Rust `chrono::DateTime::parse_from_rfc3339`,
  both sides — the §7.5 dual-cover rule);
- `from >= to` (mirrors handler_metrics.go:38 `!from.Before(to)`);
- `to` more than 300 s in the future (clock-skew allowance; a future `to` is
  a caller bug);
- windows longer than 7 days (`to - from`, or `now - from` when `to` is
  omitted) so a hostile caller cannot make Resin scan an unbounded metrics.db
  range;
- timestamps longer than 64 chars (length cap before parse).

The realtime endpoint takes no params (issue F4: "realtime 无入参") — Resin's
own defaults (last hour) apply, so the shell never re-derives time semantics.

**D3 — Pull model, no push.** Metrics are fetched on the existing
DiagnosticsView poll cycle (`usePoll`); no WebSocket/SSE channel is added
(spec: 拉模型). The Ghost G3 health poll is a separate channel and is
untouched (issue 不做清单).

**D4 — Zero-dependency rendering.** Recharts (+ React Query) was considered
per issue F2 and rejected for the minimal set: no new dependency for a
two-card diagnostic (ponytail discipline — add deps only when explicitly
requested). The charts are inline-SVG sparklines
(`MetricsSparkline` in DiagnosticsView.tsx). Introducing Recharts is part of
the full-set decision (D1 revisit), not this round.

**D5 — GUI surface.** One "Resin Metrics" card in DiagnosticsView: realtime
egress-throughput sparkline (latest value caption) + probe-history sparkline
with a 1h/24h/7d range selector (24h default; issue's motivating question
"过去 24h 出口 IP 切换次数" reads at the 24h setting). A dedicated Metrics tab
belongs to the full-set round.

**D6 — Wire data is untrusted.** Resin's `items` array is coerced to `[]`
when missing/malformed (TS wrapper), so a Resin wire-shape regression blanks
the chart instead of throwing inside React.

## Consequences

- IPC manifest grows 70 → 72 (`metrics_probe_history`,
  `metrics_realtime_throughput`); AGENTS.md §7.6 regenerated in the same
  commit; `pnpm ipc:check` enforces the three-way alignment.
- The remaining 10 blank metrics rows remain traceable in
  RESIN_API_COVERAGE.md (T13 is the coverage source of truth, not this ADR).
- Tests: 3 mockito unit tests (client happy path + from/to encoding +
  to-omitted), 7 Rust boundary tests (`validate_metrics_range`), 8 TS wrapper
  tests, 3 DiagnosticsView card tests. verify-build.sh is the gate.

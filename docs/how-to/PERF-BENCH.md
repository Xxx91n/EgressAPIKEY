# Performance baseline benchmark (ticket 04)

Reproducible measurement of the Resin sidecar data-plane against the D-004
acceptance table. Produces `bench-results/results.json` (machine-readable,
CI-stamped) + `bench-results/SUMMARY.md` (human gate table).

## What it measures

| phase | method | metric |
|---|---|---|
| idle | sidecar only, no traffic, 60s | WorkingSet (Win) / VmRSS (Linux) + CPU |
| paired | 600 interleaved direct-vs-proxied `/echo` pairs | added-latency δ p50/p95/p99 |
| rps | open-loop fixed-rate 500 req/s × 60s (node driver; vegeta auto-used when installed) | achieved RPS + latency p99 + `driverLimited` flag |
| sse | forward-GET + CONNECT-tunnel SSE probes | TTFB δ, per-event lag, 4 behavior assertions (flush/cancel/backpressure/in-band error) |
| soak | 200 concurrent SSE streams, 240s (short) / 30min (long) | connected count, unexpected closes, load RSS/CPU, per-event gap |
| healthz | cold start + 5 warm respawns | startup ms p50/p95/max |
| app | optional `--app-exe`, whole-process-tree WorkingSet | app steady-state MB |

## Run

```bash
# full local run (~10 min, short profile)
node scripts/bench/run-bench.mjs --duration short --gate warn

# with the desktop-app memory phase (needs a built exe)
node scripts/bench/run-bench.mjs --duration short --gate warn \
  --app-exe target/x86_64-pc-windows-msvc/release/EgressAPIKEY.exe

# single phase, custom output dir
node scripts/bench/run-bench.mjs --phases soak --out bench-results/diag

# knobs (env): BENCH_SOAK_STREAMS / BENCH_SOAK_SECONDS / BENCH_SOAK_INTERVAL_MS
#              RESIN_BIN / BENCH_OUT / BENCH_DURATION / BENCH_GATE
```

CI: `.github/workflows/bench.yml` (`workflow_dispatch`, weekly cron) runs the
same suite on `windows-latest` + `ubuntu-latest` and uploads results artifacts;
CI run id/url/sha/runner image are stamped into `results.env` automatically.

## Topology under test

```text
client ──forward-GET / CONNECT──► endpoint ──► node (mock Clash HTTP CONNECT)
                                      │
                                      └──► mock upstream (/echo /sse /bytes)
```

The mock node permits external `CONNECT` targets because Resin's probe manager
requires real egress (`cloudflare.com/cdn-cgi/trace`, `gstatic.com/generate_204`)
before a node becomes routable — a loopback-only mock node leaves the bench
dead in `ensureRoutable` (the original fatal).

## Known measured behavior (round-1 local evidence, see report)

- Resin's forward-GET response path uses `io.Copy(w, resp.Body)`
  (`resin/internal/proxy/forward.go`) with no `Flush` — SSE over the forward
  path is buffered in ~2-4KB windows; events arrive in bursts, not per-event.
  The CONNECT-tunnel path (`tunnel.go`, raw socket copy) streams correctly —
  `flush` assertion evidence: `proxySpread≈0` vs `tunnelSpread≈1.08`.
- Windows WorkingSet of a just-started resin includes the mapped binary image;
  idle RSS ~73-79MB on this host.

## Two-layer baseline (r11 wave-C D-004)

Absolute gates are set only from representative hardware. The shared GitHub
runner numbers are informational reference data — useful for spotting
orders of magnitude, never a hard gate (hosted-runner variance is too high).

| metric | shared-runner reference (`bench.yml`, `ubuntu-latest`, warn-only) | representative hardware (`bench-selfhosted.yml`, operator 1C1G VPS — authoritative) |
| --- | --- | --- |
| SSE TTFB delta p95 (ms) | **26.153** (run 35619850050, resin e42f6ea2) | _pending first self-hosted run_ |
| SSE event delta p99 (ms) | **4619** — proxy path buffered (`io.Copy`, no `Flush`; known behavior, 3/4 assertions) | _pending_ |
| idle RSS (MB) | **95.6** — above the 60 MB warn threshold and the ≤80 MB idle budget line; wait on the representative column before retargeting | _pending; absolute budget ≤ 150 MB total / ≤ 80 MB idle_ |
| soak RSS @ 200 streams (MB) | **85.9** (200/200 connected, 0 unexpected closes) | _pending_ |
| paired latency delta p50/p95/p99 (ms) | **0.212 / 0.303 / 0.389** | _pending_ |
| rps sustained / p99 latency | **499.9 req/s @ 1.053 ms p99** (driver not limited) | _pending_ |
| cold-start / warm p50 (ms) | **170.8 / 206.9** | _pending_ |

Numbers above are from `bench.yml` run `35619850050` (2026-09-21,
`ubuntu-latest` 4-core, resin `e42f6ea2`, short profile). The wave-B
calibration handover's "SSE first-token p99 = 1 ms" target does not hold on
shared runners: the buffered forward path dominates event delivery
(proxySpread 0 vs tunnelSpread ~1.0 — the same known behavior recorded in
this file's "Known measured behavior" section).

Rules (D-004):

- `bench.yml` (github-hosted) records reference numbers — **no gate**.
- `bench-selfhosted.yml` is `workflow_dispatch`-only and must run on the
  operator's representative hardware (a self-hosted runner on the target
  1C1G VPS preferred; this Windows box as fallback). Its numbers are the
  **only** basis for tightening the absolute RSS / latency gates above.
- Never let a `pull_request`- or `push`-triggered job reference the
  `self-hosted` label (public-repo runner-hijack rail, GitHub's own warning).
- wave-B calibration handover: SSE first-token p99 target was 1 ms and the
  local Windows measurement was 1.14 ms — the Linux shared-runner number
  must be recorded before the median ≤1 ms / p99 ≤3 ms assertion shape can
  be tightened; the 512 KiB/conn buffer estimate and the 500-concurrency
  RSS ceiling likewise wait on representative numbers.

## Sidecar-upgrade regression procedure

1. `bash scripts/fetch_resin.sh` (or drop the new binary in `src-tauri/binaries/`).
2. `node scripts/bench/run-bench.mjs --duration short --gate warn` (add
   `--app-exe` if the app artifact changed too).
3. Compare `bench-results/results.json` gates vs the locked baseline in
   `.scratch/architecture-recovery/reports/04-perf-baseline-report.md`.
4. Any sidecar-memory regression check: diff `idle.rssMB` + `soak.rssMB`
   across the old and new binary sha256 (`results.env.resinSha256`).

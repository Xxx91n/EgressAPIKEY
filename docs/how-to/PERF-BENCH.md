# Performance baseline & acceptance line (R12-00)

Reproducible measurement of the Resin sidecar + shell data plane against the
**r12-wave-a acceptance line** (decision-ledger D-001/D-002 — this table
supersedes the round-8 D-004 single-threshold budget). Produces
`bench-results/results.json` (machine-readable, CI-stamped) +
`bench-results/SUMMARY.md` (split measurement/assertion tables).

## The acceptance line

Dual thresholds per deployment profile. `†` = provisional, activates only
after a representative-hardware (self-hosted) run locks it. Shared-runner
numbers are **warn-only reference columns** — never gate evidence.

| metric | caliber | target | danger | profile |
|---|---|---|---|---|
| SSE TTFB added delta | paired p95 | ≤5 ms | ≤30 ms | both |
| SSE event-gap added delta | good-event ratio (per-event lag − direct median) | ≤10 ms @≥99% | ≤500 ms @≥99% | desktop (Mode A tunnel) |
| idle RSS, whole process tree | WorkingSet64/VmRSS descendant sum p95 | ≤80 MB † | ≤120 MB † | both |
| soak RSS @200 SSE streams | p95 over soak window | ≤128 MB † | ≤250 MB † | both |
| paired added latency | p99 | ≤1 ms | ≤3 ms | both |
| throughput | achieved/offered ratio + error rate | ≥99% @500 rps, ≤1% errors | ≥95%, ≤5% errors | both |
| cold start → `MainWindowTitle` ready | N=5 launches, p50/p95 | ≤500 ms | ≤1 s | desktop |
| Windows desktop full process tree | WorkingSet64 incl. WebView2 subgroup | ≤128 MB † | — | desktop |

Registered exemption: **Mode B forward-GET SSE buffering** — see
*Known measured behavior*. The exemption is deleted when upstream adds
per-event flush to the forward path; until then the event-gap line gates the
**Mode A shell CONNECT tunnel** only (the desktop-shipped path).

## What it measures

| phase | method | metric |
|---|---|---|
| idle | sidecar only, no traffic, 60s | WorkingSet (Win) / VmRSS (Linux) + CPU |
| paired | 600 interleaved direct-vs-proxied `/echo` pairs | added-latency δ p50/p95/p99 |
| rps | open-loop fixed-rate 500 req/s × 60s (+vegeta cross-check when installed); 1 rps direct baseline runs concurrently | achieved/offered ratio, error rate, proxy-added δp99, `driverLimited` flag |
| sse | forward-GET (Mode B) + CONNECT-tunnel legs, and Mode A CONNECT through `bench-forwarder` when `--forwarder` is passed | TTFB δ (paired), event-gap good-event ratio @10/500 ms, 4 behavior assertions (flush/cancel/backpressure/in-band error) |
| soak | 200 concurrent SSE streams, 240s (short) / 30min (long) | connected count, unexpected closes, load RSS/CPU, per-event gap |
| healthz | cold start + 5 warm respawns | startup ms p50/p95/max |
| app | `--app-exe`: steady-state descendant-tree WorkingSet | total + `webview2MB` subgroup + per-process list |
| appstart | `--app-exe`: N=5 cold launches → `MainWindowTitle` ready | p50/p95 + title-match count |

## Run

```bash
# full local run (~10 min, short profile)
node scripts/bench/run-bench.mjs --duration short --gate warn

# Mode A legs (needs the feature-gated harness bin)
cargo build --release -p resin-core --features bench --bin bench-forwarder
node scripts/bench/run-bench.mjs --forwarder target/release/bench-forwarder.exe

# with the desktop-app phases (needs a built exe)
node scripts/bench/run-bench.mjs --forwarder target/release/bench-forwarder.exe \
  --app-exe target/x86_64-pc-windows-msvc/release/EgressAPIKEY.exe

# single phase, custom output dir
node scripts/bench/run-bench.mjs --phases soak --out bench-results/diag

# knobs (env): BENCH_SOAK_STREAMS / BENCH_SOAK_SECONDS / BENCH_SOAK_INTERVAL_MS
#              BENCH_APPSTART_N / RESIN_BIN / FORWARDER_BIN / BENCH_OUT
#              BENCH_DURATION / BENCH_GATE / BENCH_CPU_PIN
```

CI: `.github/workflows/bench.yml` (`workflow_dispatch`, weekly cron) builds the
forwarder on both runners, builds the portable exe on the windows leg
(`with_app` input), and uploads results artifacts; CI run id/url/sha/runner
image are stamped into `results.env` automatically.

## Failure semantics (D-002)

Two disjoint planes, never mixed:

- **Measurements** (`kind=measure` in `acceptance.json`) are *always recorded*
  — a breach is evidence and lands in the summary as `degraded`/`breach`, but
  never fails the job. Absolute-memory rows stay `†`-provisional until the
  self-hosted representative run.
- **Assertions & smoke** (`kind=assert`/`smoke`) are the only rows that can
  hard-fail a run under `--gate enforce` (bench) or turn the smoke workflow
  red. The webview smoke itself is report-only this round.

## Topology under test

```text
                 ┌─ Mode B: forward-GET / CONNECT ─► endpoint ───────────┐
client ──────────┤                                                       ├─► node (mock Clash CONNECT)
                 └─ Mode A: CONNECT ─► bench-forwarder ─► consolidated ──┘        │
                     (port = identity, credential injected by the shell)          └─► mock upstream (/echo /sse)
```

The mock node permits external `CONNECT` targets because Resin's probe manager
requires real egress (`cloudflare.com/cdn-cgi/trace`, `gstatic.com/generate_204`)
before a node becomes routable — a loopback-only mock node leaves the bench
dead in `ensureRoutable`.

## Process-tree attribution (Windows)

`--app-exe` steady-state memory walks the **descendant tree** of the spawned
PID (`Get-CimInstance Win32_Process` ParentProcessId BFS, re-walked every
sample). Process-name matching is banned: WebView2 (`msedgewebview2`) is a
shared runtime pooled per user-data-dir and re-parents across apps, so a name
glob would count foreign webview processes. The `webview2MB` subgroup is
reported separately from the app total.

## Known measured behavior

- **Registered exemption `mode-b-sse-buffering`** — Resin's forward-GET
  response path uses `io.Copy(w, resp.Body)` (`resin/internal/proxy/forward.go`)
  with no `Flush`: SSE over the forward path is buffered in ~2-4KB windows and
  events arrive in bursts (local evidence: `proxySpread≈0` vs
  `tunnelSpread≈1.08`; recorded per-run at `phases.sse.modeB` +
  `assertions.flush.modeB`). Also noted in `docs/RESIN_UPSTREAM_MANIFEST.yaml`.
  **Removal condition: upstream implements per-event flush on the forward
  path** — the exemption row, this paragraph, and the manifest note are then
  deleted together.
- Windows WorkingSet of a just-started resin includes the mapped binary image;
  idle RSS ~73-79MB on the reference host.

## Two-layer evidence rule

Absolute gates are set only from representative hardware. The shared GitHub
runner numbers are informational reference data — useful for spotting orders
of magnitude, never a hard gate.

| metric | shared-runner reference (`bench.yml`, warn-only) | representative hardware (`bench-selfhosted.yml` — authoritative) |
| --- | --- | --- |
| SSE TTFB delta p95 (ms) | 26.153 (run 35619850050, Mode B leg) | _pending first self-hosted run with the new harness_ |
| SSE event-gap good-ratio @10ms | _pending (new caliber, Mode A leg)_ | _pending_ |
| idle RSS (MB) | 95.6 | _pending; line ≤80/≤120 †_ |
| soak RSS @200 streams (MB) | 85.9 (200/200 connected) | _pending; ≤128/≤250 †_ |
| paired latency delta p50/p95/p99 (ms) | 0.212 / 0.303 / 0.389 | _pending_ |
| rps achieved ratio / δp99 | 499.9 req/s; δp99 caliber added this round | _pending_ |
| cold-start→title p50/p95 (ms) | _pending (new phase, windows leg)_ | _pending_ |

Rules:

- `bench.yml` (github-hosted) records reference numbers — **no gate**.
- `bench-selfhosted.yml` is `workflow_dispatch`-only and must run on the
  operator's representative hardware (a self-hosted runner on the target
  1C1G VPS preferred; this Windows box as fallback). Its numbers are the
  **only** basis for activating the `†` absolute gates.
- Never let a `pull_request`- or `push`-triggered job reference the
  `self-hosted` label (public-repo runner-hijack rail).
- Round-12 runner-deadline clause: if the self-hosted runner still has not
  materialized ~4 weeks after this round, this Windows box is the documented
  fallback profile and the absolute pins get locked from its numbers.

## Real-WebView smoke (assertion plane)

`.github/workflows/webview-smoke.yml` drives the CI-built portable exe through
WebdriverIO + the embedded `tauri-plugin-wdio-webdriver` server (built with
`--features "custom-protocol wdio-smoke"`; the wdio capability file is copied
in by CI only, so release builds never carry it). Assertions: window up +
native title, nav switch, IPC roundtrip (`port_list`), platform create/remove
round trip, ar/hi/th glyph non-overflow. Triggered post-merge + nightly;
report-only (never a PR gate). macOS stays a documented gap — the embedded
provider supports it, but runner cost/keys keep it out of this round's matrix.

## Sidecar-upgrade regression procedure

1. `bash scripts/fetch_resin.sh` (or drop the new binary in `src-tauri/binaries/`).
2. `cargo build --release -p resin-core --features bench --bin bench-forwarder`
3. `node scripts/bench/run-bench.mjs --duration short --gate warn --forwarder <bin>`
   (add `--app-exe` if the app artifact changed too).
4. Compare `bench-results/results.json` gates vs the locked baseline above.
5. Any sidecar-memory regression check: diff `idle.rssMB` + `soak.rssMB`
   across the old and new binary sha256 (`results.env.resinSha256`).

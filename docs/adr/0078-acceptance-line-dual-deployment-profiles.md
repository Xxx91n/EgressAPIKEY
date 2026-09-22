# ADR-0078: Acceptance line - dual deployment profiles, dual thresholds, measured-vs-smoke semantics

Status: DRAFT (R12-00; awaits representative-hardware numbers before locking `†` absolute pins)
- **Date**: 2026-09-22 (r12-wave-a grill decision-ledger D-001/D-002; execution ticket R12-00)
- **Supersedes**: the round-8 D-004 single-threshold perf budget table (its locks remain recorded as `locked` columns in `scripts/bench/acceptance.json`)
- **Research basis**: cloudflare-streaming-resources-1, tail-flicker-flicker-1 indexed research sessions (TTFT / event-gap / backpressure calibers)

## Context

Round-8 legislated a single absolute perf budget (D-004) measured on shared
GitHub runners — which round-11 evidence showed is unsound: shared-runner
variance dwarfs the deltas being gated, and the two deployment profiles
(headless VPS vs Windows desktop + WebView2) have materially different
resource ceilings. Round-12 D-001/D-002 instead legislate a **single
acceptance line** expressed as an eight-metric dual-threshold table, with the
runner-vs-representative split made a first-class rule of the data itself.

## Decision

1. **Dual profile, one line.** Metrics are authored once in
   `scripts/bench/acceptance.json` (`schema: 2`) and scoped per profile
   (`vps-headless` / `windows-desktop` / `both`). The desktop profile includes
   the Tauri + WebView2 process tree; the headless profile is the
   `egressapikey-headless` + resin sidecar pair.

2. **Dual thresholds.** Every measurement row carries `target` (the line we
   believe the product deserves on representative hardware) and `danger`
   (the floor past which a regression is unambiguous). Between them:
   `degraded`. Absolute-memory rows stay `†`-provisional until the
   self-hosted representative run — runner numbers are warn-only reference
   columns and may never be used to infer target-hardware goals.

3. **Two disjoint failure planes.** `kind=measure` rows are *recorded-only*:
   a breach is evidence in the summary, never a job failure. `kind=assert` /
   `smoke` rows are the only ones that hard-fail under `--gate enforce`
   (bench) or turn a run red (webview-smoke). Measurement-plane degradation
   never masquerades as a release blocker; smoke-plane failure never hides
   inside a measured median.

4. **Caliber-first throughput.** Throughput is measured as `achieved /
   offered` ratio + `errorRate` + proxy-added `deltaP99` against a
   concurrent direct baseline — not raw RPS percentiles (Goodhart).

5. **Mode A is measured through the real shell path.** A feature-gated
   `bench-forwarder` binary (`resin-core --features bench`) binds a loopback
   entry port, injects `Platform.Account:token`, and relays to the
   consolidated engine port — the same code path the desktop ships — so the
   event-gap gate exercises the shipping data plane, not a JS stand-in.
   Mode B forward-GET SSE buffering is a **Registered Exemption**
   (`acceptance.json` exempt row + `RESIN_UPSTREAM_MANIFEST.yaml`
   `known_exemptions` + PERF-BENCH.md Known-measured-behavior), removed when
   upstream adds per-event flush.

6. **Process-tree attribution, never name matching.** Windows whole-app
   WorkingSet is the descendant sum from the spawned app PID
   (`Win32_Process` ParentProcessId BFS, re-walked per sample). WebView2 is
   a shared runtime whose processes pool and re-parent; name globs would
   count foreign webviews. The `webview2MB` subgroup is reported separately.

7. **Real-webview smoke is the assertion plane's desktop leg.**
   `webview-smoke.yml` builds the portable exe with `--features
   "custom-protocol wdio-smoke"` (embedded `tauri-plugin-wdio-webdriver`,
   no external driver to version-match) and asserts: window up +
   MainWindowTitle, nav switch, `port_list` IPC roundtrip, the platform
   create/remove path (F-11 restore), and ar/hi/th glyph non-overflow with
   screenshots. Post-merge + nightly; report-only this round. macOS stays a
   documented gap.

## Consequences

- `acceptance.json` `schema: 2` is the single source of truth; gate math
  lives in `run-bench.mjs::evalGates` (target/danger/kind/measureAlt).
- The `bench.yml` windows leg now also builds the portable exe (`with_app`
  input) so metrics ⑦⑧ produce numbers there; it doubles job time on
  windows only.
- `bench.yml`'s `gate` input now maps to the real `enforce` keyword (the
  previous `fail` option was silently ignored by the harness).
- Once `bench-selfhosted.yml` runs on the representative VPS, `†` pins
  activate and its column becomes authoritative.

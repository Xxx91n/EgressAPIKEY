# T2 Handoff: Industrial-grade lifecycle + pipeline + packaging

Date: 2026-08-06
Status: PLANNED (grill Q1-Q7 complete, execution pending)

## Grill decisions (all ACCEPTED)

| Q | Decision | ADR | Key fact |
|---|----------|-----|---------|
| Q1 | Hand-roll sidecar lifecycle (CoreManager pattern, not plugin) | 0016 | clash-verge-rev hand-rolls too |
| Q2a | 500-line VecDeque ring buffer (Resin emits 29 stderr lines at boot, 0 after) | 0016 | Measured locally on v1.2.0 binary |
| Q2b | IPC snapshot + Tauri event push (both, like clash-verge-rev) | 0016 | |
| Q3 | Crash auto-restart: 3x bounded, 1s/2s/4s exponential backoff | 0016 | |
| Q4 | Upstream Resin version manifest YAML (not auto-fetch) | 0017 | Breaking changes between v1.1 and v1.2 |
| Q5 | Read-only IPC auto-retry (2x 500ms, ResinClient layer) | 0018 | Write methods no retry |
| Q6 | build-all.ps1 PowerShell pipeline + SHA256 + version from manifest | 0019 | Linux/macOS still via CI |
| Q7 | Test per capability closed loop (same commit as impl) | 0020 | |

## T2 Execution plan (11 steps, dependency graph)

T2-1 -> (T2-2 || T2-3 || T2-4 || T2-5) -> T2-6 -> (T2-7 || T2-8) -> T2-9 -> T2-10 -> T2-11

See docs/GRILL_ISSUES_BACKLOG.md T2 Execution Plan table.

## Startup prompt for next window

`
Follow the T2 execution plan in docs/GRILL_ISSUES_BACKLOG.md.
ADRs 0016-0020 are the spec. CONTEXT.md has the domain terms.

Start with T2-1 (sidecar.rs: RunningMode enum + ArcSwap<State>),
then parallel T2-2 through T2-5 (ring buffer, crash restart,
port cleanup, two-phase shutdown), each with its own closed-loop
test (ADR-0020). Then T2-6 (manifest YAML), T2-7 (AGENTS.md
v1.1.2 refs), T2-8 (resin_client retry + mockito), T2-9
(build-all.ps1), T2-10 (accumulated test gate), T2-11 (rebuild+stage+smoke).

AGENTS sec5 hard close-loop: each commit that touches src/crates
must rebuild + stage the portable exe + verify Vite chunk hash
embedded. AGENTS sec1: codegraph sync . after every code change.
Ponytail full: shortest working diff, stdlib first, no new crate
unless ADR-0016/0017/0018/0019 explicitly adds one.

Upstream: https://github.com/Resinat/Resin v1.2.0 MIT.
`

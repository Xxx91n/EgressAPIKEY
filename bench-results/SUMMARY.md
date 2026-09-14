# perf-baseline summary

- os: win32/x64 node=v24.11.0 cpus=8
- resin: D:\Aworker\EgressAPIKEY\src-tauri\binaries\resin-x86_64-pc-windows-gnu.exe sha256=b76dbab7c505d959
- cpuPin: null  duration=short gate=warn
- ci run: (local)

| item | op | threshold | measured | unit | status |
|---|---|---|---|---|---|
| app-steady-state-mb | <= | 128 | 567.77 | MB | fail |
| sidecar-idle-rss-mb | <= | 60 | 89.563 | MB | fail |
| sidecar-load-rss-mb | <= | 128 | 101.676 | MB | pass |
| sidecar-cpu-idle-pct | < | 1 | 0 | % of one core | pass |
| sidecar-cpu-load-cores | < | 0.5 | 0.034 | cores | pass |
| latency-delta-p95-ms | <= | 5 | 0.997 | ms | pass |
| latency-delta-p99-ms | <= | 10 | 1.656 | ms | pass |
| rps-sustained | >= | 500 | 499.9 | req/s | fail |
| rps-latency-p99-ms | < | 50 | 2872.249 | ms | fail |
| sse-ttfb-delta-p95-ms | <= | 5 | 41.258 | ms | fail |
| sse-event-delta-p99-ms | <= | 10 | 5598 | ms | fail |
| sse-behavior-assertions | == | 4 | 3 | count | fail |
| soak-200-streams | == | 200 | 200 | streams | pass |
| healthz-startup-ms | measure-only | - | 263.3 | ms | measured |

# Ponytail Debt Ledger

Re-verified: 2026-08-13 (post-T7 diagnostics page refactor + clear_os_proxy audit fix). Source-tagged markers are the authoritative scan target: `git grep -rnE '(#|//) ?ponytail:' -- "*.rs" "*.ts" "*.tsx" "*.json"` returns 2 hits (unchanged from 2026-08-12).

| File | Reference | Status | Note |
|---|---|---|---|
| `src-tauri/src/trace.rs` L30 | `/// ponytail: hand-rolled to avoid adding uuid crate for a 1-field validation.` | keep | `is_uuid_v4` validates the trace_id shape at the IPC boundary. The uuid-v4 format is a 36-char fixed-shape check that any developer can read; saving the uuid crate here is genuinely lazy. |
| `src/lib/ipc.ts` L646 | `// ponytail: DONE (P31-T6, commit 900ddc5) — extractIpcErr now used by translateError in all GUI catch blocks` | done | The marker is a closed-loop receipt: the prior restriction (no translateError wiring) was lifted when P31-T6 unified every `catch` path into `translateError(e, t)`. |

## Status legend
- **keep** = permanent stub/dead code deliberately kept; rationalized inline.
- **done** = previously-tagged shortcut resolved; status retained for audit trail.
- **deferred** = planned follow-up; safe to leave until the upgrade lands.
- **stale-tag** = the in-source `ponytail:` marker names an upgrade path that already landed without consuming the deferred code. The marker is misdated; the code is the real question.

## Removed during earlier passes (audit trail)
| Removed ref | Was at | What resolved it |
|---|---|---|
| `src-tauri/src/sidecar.rs` (notification crate) | no longer in tree | React-shell banner superseded the native notification plan; the marker was deleted when the banner landed (AGENTS s13). |
| `crates/resin-core/src/lane.rs` `route_id` / `normalize_auth` | removed in P1 route-correction (ADR-0014, commit c71bab5). `git grep -rn route_id -- crates/ src-tauri/ src/` is silent. | ADR-0012 port=identity replaced the whole identification line. The "deferred-dead-code" row in earlier ledger revisions was stale; the functions are gone. |
| `src-tauri/src/sidecar.rs` `MAX_CRASH_RESTARTS` / `crash_backoff_ms` | wires landed in spawn_health_poll (AGENTS s13). | The source marker was deleted once the wiring was committed (ADR-0016 Q3). Tests at sidecar.rs pin 1s/2s/4s + MAX=3. |
| `crates/resin-core/src/strategy_engine.rs` `account_for_bclass` / `account_for_fixed` | removed 2026-08-12 (this pass). `git grep -rn account_for_bclass -- crates/ src-tauri/ src/` silent. | The functions were created in P29-T5 (commit `4e469c4`) as orphan code: the commit modified `strategy_engine.rs` + `PlatformsView.tsx` + $1 test file, but never wired the helpers into any IPC (notably `port_upsert`). P30-AUDIT (`ec1f629`) tagged them as "no-caller, wire-into-port_upsert-when-IpcError-refactor-done" — but the upgrade path landed in P30-T5 via `StrategyConfig`, not these per-port label helpers. Resin v1.2.0's `allocation_policy` enum only accepts `BALANCED` / `PREFER_LOW_LATENCY` / `PREFER_IDLE_IP`; it has no `sequential` / `quality` / `bandwidth` / `protocol_weight` values, so the `port-N::{random,rr,latex,quality}` account strings the helpers produce have no Resin consumer. ADR-0014 (port=identity) had already retired the entire account-tag/route-id line. Resolved by deleting the two functions, their 4 `#[cfg(test)]` unit tests, and the stale `// ponytail: no-caller` marker. Tests: `cargo test -p resin-core --lib` = 118 pass (was 122; -4 deleted tests). `cargo build -p egressapikey-app --features custom-protocol` green. |

## Re-audit of the prior "done" rows (2026-08-12)
| Row | Prior status | Re-audit |
|---|---|---|
| `src/lib/ipc.ts` `extractIpcErr` in all GUI catch blocks | done | **done (confirmed)** — `src/views/*.tsx` `.catch` blocks call `translateError(e, t)` which narrows via `extractIpcErr`. Source marker retained at L646 is a closed-loop receipt, not debt. |

## Audit scan

```bash
git grep -rnE '(#|//) ?ponytail:' -- '*.rs' '*.ts' '*.tsx' '*.json'
```

2 source hits on 2026-08-13 (unchanged from 2026-08-12). One keep, one done. No stale markers remain. T7 diff reviewed: DiagnosticsView.tsx (361 lines) — 0 ponytail-review findings, DiagCard reused 6× (valid abstraction). No new ponytail: tags in T7 code. None blocks the next phase.

## Audit context-mode protocol
Future passes must load and use ctx_* tools (mcp__context_mode__ctx_batch_execute / ctx_search) as the first operational priority. Raw bytes stay out of the conversation; the scan above is a one-liner for shell, all reads of the hits should go through ctx_search when the surrounding context is needed.

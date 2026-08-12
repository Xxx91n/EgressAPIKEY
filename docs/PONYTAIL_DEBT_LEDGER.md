# Ponytail Debt Ledger

Re-verified: 2026-08-12 (post-T6 network-layer audit). Source-tagged markers are the authoritative scan target: `git grep -rnE '(#|//) ?ponytail:' -- "*.rs" "*.ts" "*.tsx" "*.json"` returns 3 hits.

| File | Reference | Status | Note |
|---|---|---|---|
| `crates/resin-core/src/strategy_engine.rs` L240 | `// ponytail: no-caller, wire-into-port_upsert-when-IpcError-refactor-done` | stale-tag | `account_for_bclass` and `account_for_fixed` have NO caller outside their own `#[cfg(test)]` block (`git grep -n account_for_bclass -- src-tauri/ src/` is silent). The IpcError refactor (P30-T5) connected `StrategyConfig` to `port_upsert`, not these two helpers. The upgrade path named in the marker already landed without consuming the deferred code, so the marker is stale. Either wire them or delete them on the next pass. |
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

## Re-audit of the prior "done" rows (2026-08-12)
| Row | Prior status | Re-audit |
|---|---|---|
| `strategy_engine.rs` `account_for_bclass` "wired into port_upsert via the T5 IpcError refactor" | done | **stale** — `git grep -n account_for_bclass -- src-tauri/ src/` is silent. The P30-T5 refactor connected the `StrategyConfig` struct and the `ipcStrategyConfigGet/Put` IPC, not these two per-port label helpers. Corrected above. |
| `src/lib/ipc.ts` `extractIpcErr` in all GUI catch blocks | done | **done (confirmed)** — `src/views/*.tsx` `.catch` blocks call `translateError(e, t)` which narrows via `extractIpcErr`. Source marker retained at L646 is a closed-loop receipt, not debt. |

## Audit scan

```bash
git grep -rnE '(#|//) ?ponytail:' -- '*.rs' '*.ts' '*.tsx' '*.json'
```

3 source hits on 2026-08-12. One stale-tag, one keep, one done. None blocks the next phase.

## Audit context-mode protocol
Future passes must load and use ctx_* tools (mcp__context_mode__ctx_batch_execute / ctx_search) as the first operational priority. Raw bytes stay out of the conversation; the scan above is a one-liner for shell, all reads of the hits should go through ctx_search when the surrounding context is needed.

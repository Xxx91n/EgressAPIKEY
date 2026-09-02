# Ponytail Debt Ledger

Re-verified: 2026-08-17 (post-T17 canvas v4 audit + ADR-0041 S1 spec-gap fix). Source-tagged markers are the authoritative scan target: `rg -n "ponytail:" --glob "!*.md" --glob "!docs/**"` returns 6 hits (was 2 on 2026-08-13). Three of the 6 were present-but-unlisted since T15 / T16 / T8-era diffs; the 4 new hits across that span were not synced into this ledger, which is the drift fixed here. The 2026-08-13 entry below preserved verbatim as history, then a 2026-08-17 section supersedes the count.

Re-verified: 2026-08-13 (post-T7 diagnostics page refactor + clear_os_proxy audit fix). Source-tagged markers are the authoritative scan target: `git grep -rnE '(#|//) ?ponytail:' -- "*.rs" "*.ts" "*.tsx" "*.json"` returns 2 hits (unchanged from 2026-08-12).

## Active markers (2026-08-17)

| File | Reference | Status | Note |
|---|---|---|---|
| `src-tauri/src/trace.rs` L30 | `/// ponytail: hand-rolled to avoid adding uuid crate for a 1-field validation.` | keep | `is_uuid_v4` validates the trace_id shape at the IPC boundary. The uuid-v4 format is a 36-char fixed-shape check that any developer can read; saving the uuid crate here is genuinely lazy. |
| `src-tauri/src/sidecar.rs` L330 | `// ponytail: known race — if the GUI is killed between spawn and assign, ...` | keep | Windows Job Object assign happens after `cmd.spawn()`; if the GUI is killed between spawn and assign the child becomes orphan. This is an accepted limitation (same as clash-verge-rev PR #6853); a suspended-create fix requires patching `tauri-plugin-shell`. Not touched. |
| `src-tauri/src/sidecar.rs` L882 | `// - ponytail: do NOT add tauri-plugin-notification just for this — the webview already renders a banner on events.` | keep | The ghost safety net (`spawn_health_poll`) reports unhealthy sidecar via a `sidecar-status` event + OS system-proxy clear. A native notification crate would be a redundant surface; the React-shell banner is the single notification channel. No new native crate. |
| `src-tauri/src/commands/mod.rs` L908 | `let mut guard = sidecar.child.lock().unwrap_or_else(\|e\| e.into_inner()); // ponytail: poison-safe, matches AGENTS §7.5 no-panic-in-production` | keep | `Mutex::lock().unwrap()` would panic the IPC command thread if a prior holder panicked; `unwrap_or_else(e => e.into_inner())` recovers the poisoned lock and continues shutdown. Matches AGENTS §7.5 `LeaseTable::evict_lane` pattern — no panic in production paths. |
| `src/lib/ipc.ts` L669 | `// ponytail: DONE (P31-T6, commit 900ddc5) — extractIpcErr now used by translateError in all GUI catch blocks` | done | The marker is a closed-loop receipt: `extractIpcErr` narrows every `catch (e)` in `src/views/*.tsx` via `translateError(e, t)` (P31-T6 unified the GUI error path). No remaining `catch` block stringifies `e` directly. Line number updated L646 → L669 (T15 strategyConfig JSON editor edit shifted the file). |
| `src/views/TopologyView.tsx`L234 |`// ponytail: S2 helper dedupNodesByHash (defined near the top of this file) is only called from the subscription route above + its unit test, not from this region route.`| keep | ADR-0041 S2 contract says both routes must dedup by`node_hash`. The subscription route calls the shared `dedupNodesByHash`helper; the region route dedups inline via`seenGlobal`Set *while* simultaneously building`regionMap` (total/healthy/subs/nodeRows). Fusing those two concerns into the dedup-only helper would couple aggregation into a single-purpose function and grow the regression surface. Track here; promote to the helper only when aggregation-vs-dedup can be cleanly separated. S2 contract honored, different shape. |

## Status legend

- **keep** = permanent stub/dead code deliberately kept; rationalized inline.
- **done** = previously-tagged shortcut resolved; status retained for audit trail.
- **deferred** = planned follow-up; safe to leave until the upgrade lands.
- **stale-tag** = the in-source `ponytail:` marker names an upgrade path that already landed without consuming the deferred code. The marker is misdated; the code is the real question.

**Stale-tag count: 0** (2026-08-17). All 6 source-tagged markers either actively `keep` (5) or `done` (1). No marker left after the upgrade it tracked landed.

## Removed during earlier passes (audit trail)

| Removed ref | Was at | What resolved it |
|---|---|---|
| `src-tauri/src/sidecar.rs` (notification crate) | no longer in tree | React-shell banner superseded the native notification plan; the marker was deleted when the banner landed (AGENTS s13). The keep marker at L882 documents this decision in-source. |
| `crates/resin-core/src/lane.rs` `route_id` / `normalize_auth` | removed in P1 route-correction (ADR-0014, commit c71bab5). `git grep -rn route_id -- crates/ src-tauri/ src/` is silent. | ADR-0012 port=identity replaced the whole identification line. The "deferred-dead-code" row in earlier ledger revisions was stale; the functions are gone. |
| `src-tauri/src/sidecar.rs` `MAX_CRASH_RESTARTS` / `crash_backoff_ms` | wires landed in spawn_health_poll (AGENTS s13). | The source marker was deleted once the wiring was committed (ADR-0016 Q3). Tests at sidecar.rs pin 1s/2s/4s + MAX=3. |
| `crates/resin-core/src/strategy_engine.rs` `account_for_bclass` / `account_for_fixed` | removed 2026-08-12 (this pass). `git grep -rn account_for_bclass -- crates/ src-tauri/ src/` silent. | The functions were created in P29-T5 (commit `4e469c4`) as orphan code: the commit modified `strategy_engine.rs` + `PlatformsView.tsx` + 1 test file, but never wired the helpers into any IPC (notably `port_upsert`). P30-AUDIT (`ec1f629`) tagged them as "no-caller, wire-into-port_upsert-when-IpcError-refactor-done" — but the upgrade path landed in P30-T5 via `StrategyConfig`, not these per-port label helpers. Resin v1.2.0's `allocation_policy` enum only accepts `BALANCED` / `PREFER_LOW_LATENCY` / `PREFER_IDLE_IP`; it has no `sequential` / `quality` / `bandwidth` / `protocol_weight` values, so the `port-N::{random,rr,latex,quality}` account strings the helpers produce have no Resin consumer. ADR-0014 (port=identity) had already retired the entire account-tag/route-id line. Resolved by deleting the two functions, their 4 `#[cfg(test)]` unit tests, and the stale `// ponytail: no-caller` marker. Tests: `cargo test -p resin-core --lib` = 118 pass (was 122; -4 deleted tests). `cargo build -p egressapikey-app --features custom-protocol` green. |

## Re-audit of the prior "done" rows (2026-08-12)

| Row | Prior status | Re-audit |
|---|---|---|
| `src/lib/ipc.ts` `extractIpcErr` in all GUI catch blocks | done | **done (confirmed)** — `src/views/*.tsx` `.catch` blocks call `translateError(e, t)` which narrows via `extractIpcErr`. Source marker retained at L669 (was L646 pre-T15) is a closed-loop receipt, not debt. |

## Audit scan

```bash
# Living snapshot: uses ripgrep (rg) so docs are excluded; ordering is by file.
rg -n "ponytail:" --glob "!*.md" --glob "!docs/**"
```

6 source hits on 2026-08-17 (was 2 on 2026-08-13). Five keep, one done. No stale markers remain. Drift surfaced in the post-T17 audit: the T15 + T16 + T17 commits added 4 keep markers (`commands/mod.rs` L908 poison-safe mutex, `sidecar.rs` L330 Job Object race, `sidecar.rs` L882 notification crate, `TopologyView.tsx` L763 S2 helper deferral) without a corresponding ledger update. Fixed by listing all 6 in one pass. T17 audit also fixed the T17-1a vitest that claimed to assert ADR-0041 S1 region guard but never switched viewMode off the default subscription path — the storage-seam pre-populate pattern (T15-3 precedent at L635-644 of `TopologyView.test.tsx`) is now used to hydrate region viewMode before render.

## Historical note on AGENTS.md §51 (T7 diff)

AGENTS.md §51 `Ponytail-debt` line L622 records "exactly 2 hits" because at the T7 commit boundary `trace.rs` L30 + `ipc.ts` L646 were the only tagged markers. That row is a frozen historical snapshot per AGENTS.md §10 — do not rewrite it; the count is the T7-time truth and the ledger above is the living current truth. The two stay reconciled because the ledger header now states both numbers side-by-side.

## Audit context-mode protocol

Future passes must load and use ctx_* tools (mcp__context_mode__ctx_batch_execute / ctx_search) as the first operational priority. Raw bytes stay out of the conversation; the scan above is a one-liner for shell, all reads of the hits should go through ctx_search when the surrounding context is needed.

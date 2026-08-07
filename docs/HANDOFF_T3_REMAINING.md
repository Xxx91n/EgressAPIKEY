# HANDOFF T3 — Remaining Work After Folder Rename (2026-08-07)

## Status: T1 all done, T2 all done, T3 bugs identified + spec ready

## Verified Baseline (post-rename)

- repo folder: D:\Aworker\EgressAPIKEY (renamed from ai-api-route)
- git branch: codex/rust-port, clean
- cargo test -p resin-core --lib = 91 passed
- pnpm test = 106 passed / 10 files
- pnpm exec tsc --noEmit = green
- pnpm i18n:check = 201 keys / 18 locales
- codegraph sync . = done
- AGENTS.md path refs updated (commit 6c095ec)
- GRILL_ISSUES_BACKLOG T2 status updated to done (commit b0f3e6b)

## T3 Remaining Items (from grill round 2026-08-07)

### T3-Q3: PlatformsView i18n error mapping (1-line fix, blocking user UX)

**Root cause**: commands/mod.rs:74 `map_resin_error` already maps Resin error
strings to i18n keys (e.g. "cannot delete Default platform" -> 
"error.cannotDeleteDefaultPlatform"). But PlatformsView.tsx:117 catch block
displays `e.message` literally without calling `t()`.

**Fix**: change the catch block to translate the returned key:
```ts
catch (e: any) {
  const key = String(e?.message ?? '');
  showToast("err", t(key, { defaultValue: key }));
}
```

**Test**: PlatformsView.test.tsx already has delete-platform test — extend to
verify the toast shows the translated string, not the raw English key.

**Priority**: HIGH (user-reported, every platform error shows English)
**Estimate**: 1-line TS + 1 vitest assertion + exe rebuild

### T3-Q1: HTTPS entry listener (design decision needed)

**Fact**: Resin v1.2.0 supports plain HTTP only (DESIGN.md section 3). No
native TLS. For HTTPS upstream proxies, a TLS terminator is needed.

**Options** (Ponytail-ordered):
1. **Do nothing** — user configures omniroute to connect via HTTP proxy (not
   HTTPS). Resin forward proxy supports HTTP CONNECT for HTTPS targets. This
   is the current working path. (0 lines, recommended for now)
2. **rustls front-end** — add a rustls-based TLS terminator in the shell
   that accepts HTTPS and forwards plain HTTP to Resin. Needs cert generation
   (self-signed). ~150 lines + 1 crate (rustls). Deferred until user
   explicitly needs HTTPS entry.
3. **External TLS terminator** — use caddy/nginx in front. Not our problem.

**Priority**: LOW (current HTTP path works; ADR-0012 route correction uses
HTTP for unified port identity)
**Estimate**: N/A for now

### T3-Q2: HTTP 525 from subscription fetch

**Root cause**: Cloudflare SSL handshake failed at origin (anniu.eepzau.org's
SSL config is broken). NOT a UA bug. UA rotation won't help — the origin
itself returns 525.

**Fix**: surface the 525 error to the user in the subscription UI with a
clear message ("origin SSL error, not a client issue") instead of a generic
"all UA attempts failed". The i18n key already exists for fetch errors;
just need to include the HTTP status code in the error message.

**Priority**: MEDIUM
**Estimate**: ~5 lines in subscription fetch error handling

### T3-Q5: gatewayBind/mihomoApi Settings dead fields

**Fact**: main.rs L151-L154 reads gatewayBind/mihomoApi from settings.json,
then L170 does `let _ = cfg;` (drops). Runtime uses port_mappings DB +
pick_free_loopback_port. These Settings inputs have zero runtime effect.

**Fix options**:
1. Remove the Settings UI inputs (clean — but changes UX)
2. Wire them to PortForwarder/Resin sidecar config (functional — but
   overlaps with port_mappings DB)

**Priority**: LOW (cosmetic, no functional impact)
**Estimate**: N/A (needs user decision)

### T3-Q6: Console flash on tray Quit

**Status**: CREATE_NO_WINDOW at sidecar.rs:273 already in place (A1 fix).
User reported residual flash — may be Tauri sub-process or panic hook.
Needs deeper investigation with stderr capture.

**Priority**: MEDIUM
**Estimate**: needs debugging session

### T2-Q3 followup: Crash restart handler wiring (ponytail-debt)

**Status**: crash_backoff_ms + MAX_CRASH_RESTARTS constants exist as dead
code stubs. The Terminated event handler in boot_resin has a ponytail:
marker for wiring but is not connected.

**Fix**: wire the Terminated handler to call crash_backoff_ms(i) + re-spawn
until MAX_CRASH_RESTARTS threshold, then mark RunningMode::Terminated.

**Priority**: MEDIUM (sidecar crash currently = manual restart)
**Estimate**: ~20 lines + 1 mock test

### AGENTS section 11 stale debug_assert note

**Fact**: mihomo.rs L88-94 has runtime #[cfg(not(debug_assertions))] panic
guard for non-loopback base. AGENTS section 11 "debug_assert stripped;
controller NOT safe" is stale — the runtime guard covers release builds.

**Fix**: doc-only commit to trim/replace the stale note.
**Priority**: LOW (doc hygiene)
**Estimate**: 3 lines

### Release exe rebuild needed

After folder rename, the release exe at release/windows-gui/EgressAPIKEY.exe
may have stale path references baked in. Need to rebuild:
```
pnpm build && cargo build --release -p egressapikey-app --features custom-protocol
bash scripts/build-all.sh
```
Then verify Vite chunk hash embedded in exe.

**Priority**: HIGH (user tests with this exe)
**Estimate**: 1 build cycle

## Execution Order

1. T3-Q3 (1-line i18n fix + test) → commit → 
2. Release exe rebuild → stage → smoke → commit
3. T2-Q3 crash restart wiring → commit
4. T3-Q2 subscription error surfacing → commit
5. AGENTS section 11 doc cleanup → commit
6. T3-Q6 console flash investigation → commit if fixable

## Constraints

- Single-threaded (AGENTS section 8)
- ctx_* first for all file edits
- Ponytail full: minimal diffs, reuse existing
- Every code change: build + test + stage release exe + smoke
- Small step commits
- Do NOT tag, do NOT publish, do NOT trigger GitHub Actions

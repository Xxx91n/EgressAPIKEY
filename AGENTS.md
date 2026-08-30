# EgressAPIKEY - Project Operating Instructions

> **[!IMPORTANT] Folder rename complete**: the on-disk repository folder has been renamed from `D:\Aworker\ai-api-route` to `D:\Aworker\EgressAPIKEY`. All path-agnostic artifacts are aligned: `tauri.conf.json` productName = "EgressAPIKEY", Cargo bin name "egressapikey-app", npm name "EgressAPIKEY", bundle id "com.egressapikey.desktop".

---

## context-mode routing (MANDATORY)

- File edits (including patches) MUST go through ctx_batch_execute / ctx_execute_file, not apply_patch.
- ctx_* first; fallback to Codex builtins only when ctx_* can't do the same job. Read-to-analyze / search / large grep: ctx_batch_execute(commands, queries) or ctx_search(queries) - never Get-Content/Select-String into context. For data analysis use ctx_execute(code) and print only the answer.
- Web/HTTP: ctx_fetch_and_index(url, source) then ctx_search(queries). curl/wget/inline HTTP are forbidden.
- Shell OK for git, mkdir, rm, mv, cd, ls, npm install, dotnet build, cargo build, vitest, scripts/build-all.ps1 (execution, not analysis; output is bounded and acceptable).
- Windows paths in ctx sandbox: use forward-slash Windows form `D:/Aworker/EgressAPIKEY/...` for both the `cwd` argument and inline paths. The ctx `shell` language routes to `pwsh.exe` (PowerShell 7), NOT bash — Git-Bash form `/d/Aworker/...` either resolves to the wrong drive `D:\d\Aworker\...` or, when passed as `cwd`, kills the spawn with `pwsh.exe ENOENT`. PowerShell cmdlets need `pwsh -NoProfile -Command "..."`. `$`-using PowerShell logic must go in a `.ps1` and run with `-File` (inline `$` is stripped by the host transport).
- After resume: `ctx_search(sort:"timeline")` before asking the user anything. Search prior session memory before re-reading sources.
- Output artifacts as files + path + one-line summary; never inline large content. Descriptive source labels for `ctx_search(source:"label")`.
- Keep this block at the very top. Any later agent editing this file must keep the context-mode routing block intact and on top. Extended project spec follows.

---


## CodeGraph (MANDATORY for code exploration)

CodeGraph is the project's indexed code intelligence layer. The index lives at `.codegraph/` (gitignored). All agents and LLMs working on this project MUST use CodeGraph as the FIRST step for code exploration — it returns verbatim source of relevant symbols grouped by file in one capped call, far more efficient than manual Grep/Read loops.

**How to use**:
- Via MCP: call `codegraph_explore` with `projectPath: "D:\Aworker\EgressAPIKEY"` and a query (symbol names, file names, or natural-language question).
- Via CLI: `codegraph explore "<query>"` or `codegraph query "<symbol>"` or `codegraph node <symbol>` or `codegraph files`.
- After any code change: run `codegraph sync .` to incrementally update the index. For a full rebuild: `codegraph index .`.
- Check index status: `codegraph status .`.

**When to call FIRST (before reading files)**:
- "How does X work?" or "Where is X defined?"
- "What calls Y?" or "What is the blast radius of changing Z?"
- Surveying an area before an edit
- Finding the call path between symbols

**When NOT needed**: trivial one-file edits where you already know the exact line, or after CodeGraph has already returned the source in this session (treat returned source as already Read — do NOT re-open those files).

**Index sync is mandatory after code changes** (same commit that changes code must update the index). The index is gitignored and never committed.
## Project Overview

EgressAPIKEY is a Tauri 2 + React 19 desktop app: an L7 proxy gateway specialized for AI API keys. Each key maps to a Resin (Platform, Account) pair; the Resin Go sidecar guarantees a distinct sticky exit IP per pair and locks the lease until an SSE stream completes. The Rust crates/resin-core is the shell-side support crate (loopback REST client, whitebox config store, port forwarder/health, strategy engine, stream sensing, IP reputation, typed IPC errors) after the ADR-0050 dead-kernel-face deletion; it no longer re-implements the gateway kernel.

- Backend core: `crates/resin-core/` (Rust: tokio, reqwest, rusqlite; axum dep removed with ADR-0050)
- Desktop shell: `src-tauri/` (Tauri 2, sidecar lifecycle, system tray, Ghost safety net)
- Frontend: `src/` (React 19, TS, Vite, Tailwind, ReactFlow 12, Zustand 5, react-i18next)
- Docs: `docs/architecture/ARCHITECTURE.md`, `docs/history/phases/PROJECT_PLAN.md`, `docs/architecture/MEMORY_REUSE_DECISION.md`
- Release artifacts: `release/` (gitignored except tags)

---

## Storage locations (runtime)

The app stores user data in OS-standard dirs (resolved by Tauri's `app.path()`). On Windows these land under `%APPDATA%`; on macOS under `~/Library/Application Support`; on Linux under `~/.config` / `~/.local/share`. The authoritative three-layer model is legislated in `docs/architecture/ARCHITECTURE.md` §「配置权威」(Config Authority); the layers here are L1 GUI preferences / L2 whitebox user-editable config / L3 Resin runtime — an agent must not add a config key without classifying it into one of the three.

- **L1 GUI preferences (`settings.json`)**: `app_config_dir()` — written by `tauri-plugin-store` (webview `src/lib/settings.ts`) plus four Rust commands (`lightweight_get/set` in `src-tauri/src/commands/settings.rs`, `get/set_log_level` in `src-tauri/src/commands/settings.rs`) for keys the webview cannot own. Keys: `lang`, `theme`, `view`, `processRoutes`, WebDAV credentials (`webdavUrl`/`webdavUsername`/`webdavPassword`), IP-reputation credentials (`ipReputationProvider`/`ipQualityScoreApiKey`/`abuseIpDbApiKey`), `ipChannelPolicyMap`, `keyCandidates`, `nodeProbe`, `portAuthDefault`, `splitRatio`, `lightweightEnabled`/`lightweightDelayMinutes`, `topologyState`/`localSubOrder`, `view` (last-active tab; `effectiveConfig` is the one-level Effective Config view, architecture-recovery ticket 13). Only the Rust tray reads `lang` directly; no other Rust component consumes L1 keys (see §7.6). Dead pre-T3-A network keys (`gatewayBind`, `mihomoApi`) are purged once at startup (arch/02). L1 keys never influence proxy behavior — if a key changes what the proxy does, it belongs in L2.
- **L2 whitebox user-editable config (authoritative write entry)**: two files in `app_config_dir()`, each with exactly one writing module —
  - `egressapikey-strategy.json` (`resin_core::StrategyConfig`, version 1): shell strategy identity (`a_class`, `b_class`, `manual_nodes`, `subscriptions`, `top_n` per platform). Written only by `resin_core::StrategyService` (`strategy_config_put` + the deep `strategy_platform_regions_set` are its thin IPC facades; ADR-0052); applied to Resin only by `strategy_apply` (whose auto-clean of stale platform entries lives inside the Service). GUI edits and external file edits converge here (ADR-0036, read side per ADR-0039 SS2).
  - `egressapikey-ports.json` (`resin_core::WhiteboxConfig`: `entry_ports` + `network`): written only by `crates/resin-core/src/whitebox_config.rs` `WhiteboxConfigStore` (hotswap-config atomic write + validate-before-swap + file watch). `port_upsert`/`port_remove`/`port_toggle`/`whitebox_save_network` IPC all funnel into it; its `watch_apply` applies accepted files to SQLite + listeners as one transaction (invalid files never trigger the callback). `egressapikey.db` (`DbPool`, `port_mappings` table, hand-written `PRAGMA user_version`, currently v3) is the SQLite sync partner seeded from the whitebox JSON at boot (`main.rs` seeds `WhiteboxConfigStore` from `db.list_ports()`; on corrupt JSON it quarantines and reseeds from DB) — the whitebox file, not the DB, is the truth source (ADR-0042 S2/S6).
- **L3 Resin runtime (derived, rebuildable)**: the sidecar's own state under the per-user Resin state dir (`state.db`/`cache.db`/`request_logs*.db`) plus live leases/listeners. Execute-only authority: reachable exclusively through the ResinClient REST seam (`crates/resin-core/src/resin_client.rs`); rebuildable from L2 at any time (`strategy_apply` PATCHes `region_filters`, `restore_ports_from_whitebox` re-POSTs `/api/v1/endpoints` after a Resin restart, ADR-0042 S6). Former seam exception CLOSED by architecture-recovery ticket 11 (2026-08-30): `request_log_tail` now reads via ResinClient `GET /api/v1/request-logs`; Resin v1.2.0 does expose this endpoint. No shell code may read Resin's private `request_logs*.db` files — a direct read reintroduced anywhere is a review blocker.

Logs are storage, not config: `app_log_dir()` — `tauri-plugin-tracing` daily-rotating file appender for all `tracing::` output; Resin request logs are L3 runtime data, not a config layer.

The Settings > Storage card exposes `Open config directory` and `Open log directory` buttons (commands `get_config_dir` / `get_log_dir` in `src-tauri/src/commands/settings.rs`, scoped `opener:allow-open-path` capability) so a user can reach these paths from inside the GUI.

## /init conventions (enforced from P0)

Any agent or human landing on this repo MUST apply these conventions. Violating any is a blocking review comment.

### 1. Code exploration - CodeGraph MANDATORY
- Before reading source files to answer "how does X work / where is X / what calls Y", query CodeGraph first. See the CodeGraph block above for commands.
- After any source change in a commit, run `codegraph sync .` in the SAME session before committing so the index reflects the change. The index lives in `.codegraph/` (gitignored, never committed).
- Treat codegraph-returned source as already-Read; do NOT re-open those files in the same session.


### 2. Tool routing - context-mode MANDATORY
- See the context-mode routing block at the top. File edits, large grep, web fetch, data analysis all go through ctx_* first. `curl`/`wget`/inline HTTP are forbidden; use `ctx_fetch_and_index`. Shell is OK for bounded mutating commands (git, mkdir, cargo build, pnpm scripts).


### 3. i18n - decoupled, full-key coverage
- All user-visible strings in `src/` MUST come from the i18n catalog (`src/locales/<locale>/*.json`) via `react-i18next` `t()` / `Trans`. Never hard-code English (or any locale) in components.
- Base locales (18): `en`, `zh`, `ja`, `es`, `fr`, `de`, `ko`, `ru`, `pt`, `ar`, `hi`, `id`, `it`, `nl`, `pl`, `th`, `tr`, `vi`. Adding a string means adding the key to ALL base locales in the same commit. The `Locale` union in `src/store/appStore.ts` and the `ALL` list in `scripts/i18n-check.cjs` MUST stay in lockstep with `src/locales/` directories.
- `pnpm i18n:scan` runs `i18next-parser` (config `i18next-parser.config.js`) to extract t()/Trans keys into `src/locales/`; `pnpm i18n:check` fails the build if any base locale is missing a key (or has an extra one) vs the canonical `en` catalog. i18n.ts lazy-loads each (locale, namespace) chunk via `i18next-resources-to-backend` so Vite code-splits one chunk per locale.
- Locale files are the single source of truth for UI text; no inline substitutions of translated strings.


### 4. Tests - mandatory per behavior
- Rust: every public function in `crates/resin-core/` has a unit test in the same file (`#[cfg(test)] mod tests`) or an integration test under `crates/resin-core/tests/`. New behavior without a test is blocked.
- Frontend: Zustand stores and pure reducers have vitest unit tests under `src/**/*.test.ts(x)`. Component interactions have playwright e2e under `e2e/`.
- Evaluator for the whole repo: `bash scripts/verify-build.sh` - runs cargo build, cargo test, pnpm build, pnpm test; exits non-zero if any fail. CI calls this; so should pre-push hooks.


### 5. CI/CD - multi-platform packaging to release/

> **Hard close-loop (verified P23): every source commit MUST yield a freshly-staged exe.** This is non-negotiable. After ANY edit to `src/`, `src-tauri/`, `crates/`, `scripts/`, `tauri.conf.json`, or locale catalogs, you MUST, in the same turn:
> 1. Rebuild the bundle: `pnpm build` (Vite writes a NEW chunk hash to `dist/assets/*.js`).
> 2. Rebuild the exe: `cargo build --release -p egressapikey-app --features custom-protocol` (or `bash scripts/build-all.sh` which runs the two steps in order).
> 3. Stage it: copy `target/<host-triple>/release/EgressAPIKEY.exe` and the sidecar to `release/<os>-gui/`.
> 4. Prove the bundle is fresh, not just the shell: grep the new Vite chunk hash (from step 1) inside the staged exe bytes.` MainWindowTitle` alone is NOT sufficient - it proves the Tauri shell boots, not that the webview reflects your edits. A stale `dist/` embeds an old frontend and your UI fixes stay invisible to the user (this exact bug bit P22; the exe ran but showed pre-fix UI).
> 5. Only THEN claim complete. Claiming complete with a stale exe is a false claim.
>
> **Forbidden shortcut**: `cargo build --release` alone does NOT invoke `beforeBuildCommand` (only `tauri build` does). It reuses the on-disk `dist/`; shipping the resulting exe is shipping a stale-bundle binary.
>
> **Build-all hash guard (T10-audit)**: both `scripts/build-all.sh` and `scripts/build-all.ps1` now include an automatic chunk-hash verification step after staging the portable exe. They extract the latest Vite content hash from `dist/assets/index-*.js` and scan the staged exe bytes for it; if the hash is NOT found, the script FAILS with a clear "STALE BUNDLE" error message and exits non-zero. This means `build-all` is now self-guarding: even if the LLM forgets to run `pnpm build` first, the script will catch the stale bundle before it ships.

- GitHub Actions matrix builds five artifact groups into `release/`:
  1. windows-gui - Tauri MSI + NSIS `-setup.exe` installer (x86_64 msvc)
  2. linux-gui - Tauri `.deb` (Debian) + `.AppImage` (x86_64)
  3. macos-gui - Tauri `.dmg` arm64 AND x86_64 (split matrix jobs)
  4. gui-portable - one Tauri `--no-bundle` GUI executable per OS (no installer) for the convenient drop-and-run variant alongside the installer
  5. Backend-only headless target - `cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless` per OS, self-contained tar.gz per platform named `release/<os>-backend/` (headless exe + dist/ + resin sidecar) — see ADR-0043
- Local reproduction: `bash scripts/build-all.sh` runs the same pipeline on the host OS and stages artifacts at `release/<os>-backend.tar.gz`, `release/<os>-gui/<installers>`, and `release/<os>-gui/<os>-portable-gui`. Requires `@tauri-apps/cli` (devDependency).
- iOS GUI note: iPadOS cannot run a Tauri desktop shell; the Apple-silicon desktop sibling is the macOS `.dmg`. Documented in CI and README. Do not promise an iOS iPad build.
- Artifacts are produced by `tauri-action` (installer + portable) and the backend matrix job; `release/` is gitignored except for tagged release assets uploaded to the GitHub Release.
- Debug-vs-release caveat (verified P2): `cargo build` (debug) emits a binary whose `tauri::generate_context!()` honours `devUrl` (`http://localhost:1420`) when `debug_assertions` is on - the webview tries to load the Vite dev server and shows "ERR_CONNECTION_REFUSED" / "localhost 拒绝连接" if `pnpm dev` is not running. Double-clicking the debug exe with no dev server looks like a GUI that flashes and dies. The **release** binary (`cargo build --release` or `tauri build --no-bundle`) disables `debug_assertions`, so `generate_context!` falls back to `frontendDist: ../dist` and embeds the built assets - it runs standalone with no dev server. The user-facing drop-and-run binary MUST be the release portable (`release/<os>-gui/<os>-portable-gui`), never the debug exe. Verified P2: release exe (7.9MB) boots, `MainWindowTitle = "EgressAPIKEY"`, stays alive 14s+, stdout `tracing initialized`, no stderr/panic.
- **MinGW resource linker fix (windows-gnu target, verified P19)**:	auri-winres (build dependency of 	auri-build) finds the Windows SDK c.exe at C:\Program Files (x86)\Windows Kits\10\bin\...\x64\rc.exe and compiles esource.rc into MSVC .lib format that MinGW gcc.exe CANNOT link - error esource.lib: file not recognized: file format not recognized. Fix: after cargo build fails the link step (the resource.rc is already written), run windres --input <build-script-out>/resource.rc --output <build-script-out>/resource.lib --output-format coff to overwrite the MSVC .lib with a COFF object, then re-run cargo build --release (the linker will re-link with the fixed object). The previous successful builds produced libresource.a (the windres output) - the SDK rc.exe was likely not in PATH then. This is environmental, not a code bug.

- **Local release-rebuild order (verified P23)**: NEVER run `cargo build --release -p egressapikey-app --features custom-protocol` alone and call it a release. The `beforeBuildCommand: pnpm build` hook in `tauri.conf.json` is invoked ONLY by `tauri build` (or `build-all.sh` via its explicit `vite build` step). A bare `cargo build --release` reuses the on-disk `dist/` from the last `vite build`; if you edited frontend code since, the new exe carries the STALE frontend bundle and your UI fixes are invisible to the user. Smoke check `MainWindowTitle` only proves the Tauri shell boots - it does NOT prove the webview reflects the latest source. EITHER run `pnpm build && cargo build --release -p egressapikey-app --features custom-protocol` (manual two-step) OR run `bash scripts/build-all.sh` (which does `tsc -b && vite build` before cargo). Verify the embedded bundle is fresh by grepping the new chunk hash from `dist/assets/*.js` inside the exe bytes (e.g. ASCII-string match for the latest Vite chunk name like `C9su2AFf` in `[System.IO.File]::ReadAllBytes(exe)`).
- When handing a build to the user for testing, launch it once under `Start-Process` and confirm: `MainWindowHandle != 0`, `MainWindowTitle == "EgressAPIKEY"`, `WorkingSet ~20-40MB`, stdout has `tracing initialized` and no panic line. If `MainWindowTitle` is the exe path or stderr has `ERR_CONNECTION_REFUSED`, it is a debug build mistakenly handed out - rebuild release.


### 6. Git hygiene - push after every change
- Commit convention: `<Phase>: <area> - <summary>` e.g. `P1: core - lane hash + SSE lease`.
- Every phase/feature commit MUST be pushed (`git push`) so the project is always traceable. Do not accumulate local-only work across phases.
- `.gitattributes` LF policy is authoritative; `git diff --check` must be clean before each commit. Never commit with CRLF outside the allow set (`.bat`, `.ps1`, `.cmd`).
- `git config core.autocrlf false` at repo level (set at init). Do not re-enable autocrlf.
- **Build is part of the commit (verified P23)**: a code commit is unfinished until the freshly-built `release/<os>-gui/EgressAPIKEY.exe` exists, embeds the latest Vite chunk hash (see section 5 hard close-loop), and has been smoke-launched. A pushed source-only commit is traceable but the user cannot test the change. The `target/` dir and `release/` are gitignored so the exe itself is NOT committed - what you commit is the SOURCE; the freshly staged exe is the deliverable that lives outside git. Treat `release/windows-gui/EgressAPIKEY.exe` as the user-facing artifact; do not deliver a stale exe even if AGENTS.md was already updated.


### 7. File integrity (host protocol)
- New source files: UTF-8 no BOM, LF line endings unless extension is in the CRLF allow set.
- Edits to existing files: re-read the affected region before destructive change; after two failed apply_patch attempts on the same file, do one whole-file rewrite and verify bytes.
- Never inline `$` PowerShell logic; write a `.ps1` and run with `-File` (host transport strips inline `$`).


### 7.5. Tauri IPC input validation (from P-1 security audit)

- Every `#[tauri::command]` in `src-tauri/src/commands/` MUST validate its input BEFORE calling resin-core. The kernel is defense-in-depth, not the first line.
- Lane indices (`lane: usize`) must be rejected at the IPC layer with an explicit `Err` when `lane >= resin_core::MAX_LANES`. Any future lane-indexed API that reports success/failure per lane MUST return `bool` (or an equivalent status); callers MUST treat `false` as "no such lane" and never panic.
- Free-form string identifiers (e.g. `authority`) passed to a bounded table or future registry MUST be length-capped (≤253 chars per DNS host) and reject NUL/control characters. The bounded table itself MUST enforce a capacity ceiling (e.g. 256 entries) so a hostile or buggy caller cannot grow memory unbounded. (The TdEwma table this rule was written for was deleted in ADR-0050; the rule stays as the template for any successor.)
- Numeric inputs (`latency_ms`, etc.) that feed an EMA or accumulator MUST be capped to a plausible ceiling before entering the kernel (e.g. `LATENCY_CAP_MS = 24h`) so u64::MAX cannot poison the EMA.
- Do NOT lock the whole `SharedGateway` across an await; commands lock for one short critical section and return. (Current commands are sync; if a future command is async, keep the same invariant.)



### 7.6. Sidecar SSRF guard + IPC surface discipline (Re8 audit)
- mihomo status (ADR-0050): the `crates/resin-core/src/mihomo.rs` module (MihomoController + its loopback SSRF guard, MihomoConfig, subscription compile helpers) was DELETED with the rest of the dead kernel face (zero external references; it was never instantiated). If mihomo REST control is ever reintroduced, the controller MUST re-assert the Re8 rule before any IPC wiring: `api_base` validated as loopback-only (`http://127.0.0.1` / `http://localhost` / `http://[::1]`, plus https variants) at construction, with reject + accept-variant unit tests, and no frontend-controlled `String` may ever construct it (config stays server-side trust).
- Frontend `invoke()` surface (authoritative manifest, architecture-recovery ticket 03): the full command set lives in the `ipc-manifest` fenced block below. It is REGENERATED from the `#[tauri::command]` definitions under `src-tauri/src` by `node scripts/ipc-manifest-check.cjs --write`, and machine-checked on every build (`pnpm ipc:check`, `scripts/verify-build.sh`, and the CI verify job via verify-build.sh). The check fails the build when a command is added/removed/renamed, when the `generate_handler!` registry in `src-tauri/src/main.rs` drifts from the definition set, when this manifest drifts from either, or when a definition-file attribution goes stale (ticket 08 domain split will be caught automatically). Do not hand-edit entries: run the regenerator. The pre-ticket-03 list of 9 commands was a stale leftover from the removed SharedGateway path (ADR-0024); those four phantom names no longer appear anywhere in this file. Each TS wrapper in `src/lib/ipc.ts` validates input at the TS boundary (assertShortName/assertAuthority/assertIp, length caps, lane range `0..MAX_LANES=50`, latency cap, URL `http(s)://` prefix) BEFORE invoking, and treats the Rust response (`reason`, `lane`, `account`) as untrusted — never piped into another URL or command. The Rust side re-validates the same bounds in `commands/` domain modules (settings.rs for log level / lightweight, ports.rs for port and whitebox inputs). When wiring any new IPC command, the TS wrapper MUST follow this same validate-then-invoke contract.

```ipc-manifest
# Tauri IPC command manifest - REGENERATED by scripts/ipc-manifest-check.cjs --write.
# Machine-checked on every build (pnpm ipc:check / scripts/verify-build.sh / CI):
# entries must equal the #[tauri::command] set under src-tauri/src AND the
# generate_handler! registry in src-tauri/src/main.rs. Format: <command> = <file>.
# Do not hand-edit entries. Regenerated: 2026-08-31 (71 commands)
backup_create = src-tauri/src/commands/backup.rs
backup_upload = src-tauri/src/commands/backup.rs
backup_list = src-tauri/src/commands/backup.rs
config_export = src-tauri/src/commands/backup.rs
config_import = src-tauri/src/commands/backup.rs
get_sidecar_logs = src-tauri/src/commands/diagnostics.rs
get_sidecar_status = src-tauri/src/commands/diagnostics.rs
request_log_tail = src-tauri/src/commands/diagnostics.rs
check_firewall_status = src-tauri/src/commands/diagnostics.rs
probe_exit_ip = src-tauri/src/commands/diagnostics.rs
gateway_snapshot = src-tauri/src/commands/platform.rs
platform_add = src-tauri/src/commands/platform.rs
platform_remove = src-tauri/src/commands/platform.rs
platform_list = src-tauri/src/commands/platform.rs
platform_list_full = src-tauri/src/commands/platform.rs
platform_snapshot = src-tauri/src/commands/platform.rs
account_add = src-tauri/src/commands/platform.rs
account_bind_ip = src-tauri/src/commands/platform.rs
process_route_add = src-tauri/src/commands/platform.rs
process_route_remove = src-tauri/src/commands/platform.rs
process_route_list = src-tauri/src/commands/platform.rs
subscription_add = src-tauri/src/commands/platform.rs
subscription_remove = src-tauri/src/commands/platform.rs
subscription_refresh = src-tauri/src/commands/platform.rs
subscription_list = src-tauri/src/commands/platform.rs
node_pool_snapshot = src-tauri/src/commands/platform.rs
platform_update = src-tauri/src/commands/platform.rs
node_list = src-tauri/src/commands/platform.rs
node_probe = src-tauri/src/commands/platform.rs
platform_create_with_fields = src-tauri/src/commands/platform.rs
platform_leases = src-tauri/src/commands/platform.rs
lease_map = src-tauri/src/commands/platform.rs
ip_reputation_snapshot = src-tauri/src/commands/platform.rs
port_list = src-tauri/src/commands/ports.rs
port_suggest = src-tauri/src/commands/ports.rs
port_upsert = src-tauri/src/commands/ports.rs
port_remove = src-tauri/src/commands/ports.rs
port_toggle = src-tauri/src/commands/ports.rs
port_bind_platform = src-tauri/src/commands/ports.rs
port_running = src-tauri/src/commands/ports.rs
port_reload = src-tauri/src/commands/ports.rs
whitebox_save_network = src-tauri/src/commands/ports.rs
whitebox_path = src-tauri/src/commands/ports.rs
whitebox_get = src-tauri/src/commands/ports.rs
whitebox_reload = src-tauri/src/commands/ports.rs
stream_sensor_snapshot = src-tauri/src/commands/ports.rs
port_auth_info = src-tauri/src/commands/ports.rs
port_health_check = src-tauri/src/commands/ports.rs
watch_port_health = src-tauri/src/commands/ports.rs
system_config_get = src-tauri/src/commands/settings.rs
system_config_patch = src-tauri/src/commands/settings.rs
close_all_connections = src-tauri/src/commands/settings.rs
reset_kernel = src-tauri/src/commands/settings.rs
tray_refresh_labels = src-tauri/src/commands/settings.rs
get_config_dir = src-tauri/src/commands/settings.rs
get_log_dir = src-tauri/src/commands/settings.rs
lightweight_get = src-tauri/src/commands/settings.rs
lightweight_set = src-tauri/src/commands/settings.rs
set_log_level = src-tauri/src/commands/settings.rs
get_log_level = src-tauri/src/commands/settings.rs
strategy_verify = src-tauri/src/commands/strategy.rs
strategy_config_get = src-tauri/src/commands/strategy.rs
strategy_config_put = src-tauri/src/commands/strategy.rs
strategy_apply = src-tauri/src/commands/strategy.rs
strategy_platform_regions_set = src-tauri/src/commands/strategy.rs
authoritative_snapshot = src-tauri/src/commands/strategy.rs
strategy_backup_list = src-tauri/src/commands/strategy.rs
strategy_rollback = src-tauri/src/commands/strategy.rs
reconcile_now = src-tauri/src/commands/strategy.rs
```

- Never expose `MihomoController`, `CoreConfig.mihomo_api`, or `CoreConfig.mihomo_secret` through a `#[tauri::command]` that takes a raw `String` and constructs the controller from it. Config must come from `tauri-plugin-store` settings.json (server-side trust), not from the webview.
- The current `MihomoController` is NOT yet instantiated by the Tauri shell (only `resin-core` references it). Keeping it uninstantiated until the sidecar lifecycle wiring is an explicit safety boundary; do not wire it through a frontend-controlled constructor without revisiting this section.




### 8. Subagent policy for this repo
- This thread runs with subagents DISABLED (per user instruction). Do NOT spawn Codex native subagents or OMX team/worker lanes. Execute everything single-threaded in this agent.

### 9. Don't revert work you didn't make
- If uncommitted changes appear that this agent did not make, treat them as user/external and do not revert. Either ignore (unrelated) or build with them (affects the task).


### 10. Update this file in the same commit that changes the project
- AGENTS.md is the source of truth. When structure, conventions, module boundaries, tech stack, or release/push protocol changes, update this file in the SAME commit that introduces the change.



## Agent skills

### Issue tracker

Issues are tracked as local markdown under `.scratch/` (one directory per feature:
spec, tickets, handoffs, reports); remote `github.com/Xxx91n/EgressAPIKEY` is
available with a declared switch path to GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-label vocabulary (`needs-triage` / `needs-info` / `ready-for-agent` /
`ready-for-human` / `wontfix`), recorded as the `Status:` line of each issue file. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout: root `CONTEXT.md` glossary + `docs/adr/`. See `docs/agents/domain.md`.

### Architecture & Phase History (externalized)

> The following sections were moved to `docs/agents/` to keep this file under the 32 KiB `project_doc_max_bytes` limit.
> Content is unchanged (extraction only, progressive disclosure per writing-for-agents).

**Architecture implementation** (AGENTS.md former §11-17): [docs/agents/architecture-state.md](docs/agents/architecture-state.md)
- Runtime wiring state, Resin sidecar lifecycle (G1), Ghost safety net (G3), ResinClient (G2)
- IPC retarget to Resin sidecar, Resin webhook into desktop shell (G4), CI/CD release pipeline (G5)

**Phase history** (AGENTS.md former §18-66, P9-P26/T7-T22/R1-R2/C1-C2): [docs/agents/phase-history.md](docs/agents/phase-history.md)
- These are **process artifacts** (point-in-time phase completion records), not current truth.
- For current architecture overview: [docs/architecture/](docs/architecture/)
- For architectural decisions: [docs/adr/](docs/adr/)
- For what shipped (milestones): [CHANGELOG.md](CHANGELOG.md)


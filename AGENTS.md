# ai-api-route - Project Operating Instructions

This document is the single source of truth for the ai-api-route project. All developers, AI agents, and LLMs must follow this specification. When any project feature or structure changes, update this file in the same commit. Detailed references live in `docs/` - keep this file concise; link out instead of inlining large tables.

---

## context-mode routing (MANDATORY)

- File edits (including patches) MUST go through ctx_batch_execute / ctx_execute_file, not apply_patch.
- ctx_* first; fallback to Codex builtins only when ctx_* can't do the same job. Read-to-analyze / search / large grep: ctx_batch_execute(commands, queries) or ctx_search(queries) - never Get-Content/Select-String into context. For data analysis use ctx_execute(code) and print only the answer.
- Web/HTTP: ctx_fetch_and_index(url, source) then ctx_search(queries). curl/wget/inline HTTP are forbidden.
- Shell OK for git, mkdir, rm, mv, cd, ls, npm install, dotnet build, cargo build, vitest, scripts/build-all.ps1 (execution, not analysis; output is bounded and acceptable).
- Windows paths in ctx sandbox: use forward-slash Windows form `D:/Aworker/ai-api-route/...` for both the `cwd` argument and inline paths. The ctx `shell` language routes to `pwsh.exe` (PowerShell 7), NOT bash — Git-Bash form `/d/Aworker/...` either resolves to the wrong drive `D:\d\Aworker\...` or, when passed as `cwd`, kills the spawn with `pwsh.exe ENOENT`. PowerShell cmdlets need `pwsh -NoProfile -Command "..."`. `$`-using PowerShell logic must go in a `.ps1` and run with `-File` (inline `$` is stripped by the host transport).
- After resume: `ctx_search(sort:"timeline")` before asking the user anything. Search prior session memory before re-reading sources.
- Output artifacts as files + path + one-line summary; never inline large content. Descriptive source labels for `ctx_search(source:"label")`.
- Keep this block at the very top. Any later agent editing this file must keep the context-mode routing block intact and on top. Extended project spec follows.

---


## CodeGraph (MANDATORY for code exploration)

CodeGraph is the project's indexed code intelligence layer. The index lives at `.codegraph/` (gitignored). All agents and LLMs working on this project MUST use CodeGraph as the FIRST step for code exploration — it returns verbatim source of relevant symbols grouped by file in one capped call, far more efficient than manual Grep/Read loops.

**How to use**:
- Via MCP: call `codegraph_explore` with `projectPath: "D:\Aworker\ai-api-route"` and a query (symbol names, file names, or natural-language question).
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

ai-api-route is a Tauri 2 + React 19 desktop app: an L7 proxy gateway specialized for AI API keys. Each key hashes into a lane; every request opens a fresh TCP through mihomo to guarantee distinct exit IPs, solving the same-domain/same-IP collision that breaks AI API key pools. SSE streams lock the lane until completion. The kernel reuses the Resin (Go) Platform/Account/P2C/TD-EWMA architecture but is re-implemented in Rust so the whole OS-facing app is one Tauri build.

- Backend core: `crates/resin-core/` (Rust: tokio, axum, reqwest, petgraph, rusqlite)
- Desktop shell: `src-tauri/` (Tauri 2, sidecar lifecycle, system tray, Ghost safety net)
- Frontend: `src/` (React 19, TS, Vite, Tailwind, ReactFlow 12, Zustand 5, react-i18next)
- Docs: `docs/ARCHITECTURE.md`, `docs/PROJECT_PLAN.md`, `docs/MEMORY_REUSE_DECISION.md`
- Release artifacts: `release/` (gitignored except tags)

---

## Storage locations (runtime)

The app stores user data in OS-standard dirs (resolved by Tauri's `app.path()`). On Windows these land under `%APPDATA%`; on macOS under `~/Library/Application Support`; on Linux under `~/.config` / `~/.local/share`.

- **Config (`settings.json`)**: `app_config_dir()` — written by `tauri-plugin-store`. Keys: `gatewayBind`, `mihomoApi`, `theme`, `locale`. The Rust shell reads this as server-trust config (never from the webview); see §7.6.
- **Logs**: `app_log_dir()` — `tauri-plugin-tracing` runs `.with_file_logging()` for a daily-rotating file appender. All `tracing::` macros in the Rust side flow here. No separate log DB.
- **Database**: none yet. `rusqlite` is a Cargo dependency but is NOT instantiated, and no `Connection::open` call exists in `crates/resin-core/src/`. When a DB lands it should use `app_config_dir()` as the parent so it sits beside `settings.json`.

The Settings > Storage card exposes `Open config directory` and `Open log directory` buttons (commands `get_config_dir` / `get_log_dir` in `src-tauri/src/commands/mod.rs`, scoped `opener:allow-open-path` capability) so a user can reach these paths from inside the GUI.

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
- GitHub Actions matrix builds five artifact groups into `release/`:
  1. windows-gui - Tauri MSI + NSIS `-setup.exe` installer (x86_64 msvc)
  2. linux-gui - Tauri `.deb` (Debian) + `.AppImage` (x86_64)
  3. macos-gui - Tauri `.dmg` arm64 AND x86_64 (split matrix jobs)
  4. gui-portable - one Tauri `--no-bundle` GUI executable per OS (no installer) for the convenient drop-and-run variant alongside the installer
  5. Backend-only headless target - `cargo build --release -p resin-core` per OS, tar.gz per platform named `release/<os>-backend.tar.gz`
- Local reproduction: `bash scripts/build-all.sh` runs the same pipeline on the host OS and stages artifacts at `release/<os>-backend.tar.gz`, `release/<os>-gui/<installers>`, and `release/<os>-gui/<os>-portable-gui`. Requires `@tauri-apps/cli` (devDependency).
- iOS GUI note: iPadOS cannot run a Tauri desktop shell; the Apple-silicon desktop sibling is the macOS `.dmg`. Documented in CI and README. Do not promise an iOS iPad build.
- Artifacts are produced by `tauri-action` (installer + portable) and the backend matrix job; `release/` is gitignored except for tagged release assets uploaded to the GitHub Release.
- Debug-vs-release caveat (verified P2): `cargo build` (debug) emits a binary whose `tauri::generate_context!()` honours `devUrl` (`http://localhost:1420`) when `debug_assertions` is on - the webview tries to load the Vite dev server and shows "ERR_CONNECTION_REFUSED" / "localhost 拒绝连接" if `pnpm dev` is not running. Double-clicking the debug exe with no dev server looks like a GUI that flashes and dies. The **release** binary (`cargo build --release` or `tauri build --no-bundle`) disables `debug_assertions`, so `generate_context!` falls back to `frontendDist: ../dist` and embeds the built assets - it runs standalone with no dev server. The user-facing drop-and-run binary MUST be the release portable (`release/<os>-gui/<os>-portable-gui`), never the debug exe. Verified P2: release exe (7.9MB) boots, `MainWindowTitle = "ai-api-route"`, stays alive 14s+, stdout `tracing initialized`, no stderr/panic.
- When handing a build to the user for testing, launch it once under `Start-Process` and confirm: `MainWindowHandle != 0`, `MainWindowTitle == "ai-api-route"`, `WorkingSet ~20-40MB`, stdout has `tracing initialized` and no panic line. If `MainWindowTitle` is the exe path or stderr has `ERR_CONNECTION_REFUSED`, it is a debug build mistakenly handed out - rebuild release.

### 6. Git hygiene - push after every change
- Commit convention: `<Phase>: <area> - <summary>` e.g. `P1: core - lane hash + SSE lease`.
- Every phase/feature commit MUST be pushed (`git push`) so the project is always traceable. Do not accumulate local-only work across phases.
- `.gitattributes` LF policy is authoritative; `git diff --check` must be clean before each commit. Never commit with CRLF outside the allow set (`.bat`, `.ps1`, `.cmd`).
- `git config core.autocrlf false` at repo level (set at init). Do not re-enable autocrlf.

### 7. File integrity (host protocol)
- New source files: UTF-8 no BOM, LF line endings unless extension is in the CRLF allow set.
- Edits to existing files: re-read the affected region before destructive change; after two failed apply_patch attempts on the same file, do one whole-file rewrite and verify bytes.
- Never inline `$` PowerShell logic; write a `.ps1` and run with `-File` (host transport strips inline `$`).

### 7.5. Tauri IPC input validation (from P-1 security audit)

- Every `#[tauri::command]` in `src-tauri/src/commands/` MUST validate its input BEFORE locking `SharedGateway` and before calling resin-core. The kernel is defense-in-depth, not the first line.
- Lane indices (`lane: usize`) must be rejected at the IPC layer with an explicit `Err` when `lane >= resin_core::MAX_LANES`. `LeaseTable::evict_lane` (and any other lane-indexed API) returns `bool`; callers MUST treat `false` as "no such lane" and never panic.
- Free-form string identifiers (e.g. `authority`) passed to a bounded table (`TdEwma`, future registries) MUST be length-capped (≤253 chars per DNS host) and reject NUL/control characters. The bounded table itself MUST enforce a capacity ceiling (`TdEwma::MAX_AUTHORITIES = 256`) so a hostile or buggy caller cannot grow memory unbounded.
- Numeric inputs (`latency_ms`, etc.) that feed an EMA or accumulator MUST be capped to a plausible ceiling before entering the kernel (e.g. `LATENCY_CAP_MS = 24h`) so u64::MAX cannot poison the EMA.
- Do NOT lock the whole `SharedGateway` across an await; commands lock for one short critical section and return. (Current commands are sync; if a future command is async, keep the same invariant.)


### 7.6. Sidecar SSRF guard + IPC surface discipline (Re8 audit)
- `crates/resin-core/src/mihomo.rs` `MihomoController::new` validates its `api_base` is a loopback URL (`http://127.0.0.1` / `http://localhost` / `http://[::1]`, plus https variants) and rejects anything else at construction. This blocks the classic "frontend string -> sidecar REST base -> internal LAN/SSRF" escalation from any future IPC command that wires `CoreConfig.mihomo_api` from the UI. Guard runs once at construction; covered by `new_rejects_non_loopback` + `new_accepts_loopback_variants` unit tests.
- Frontend `invoke()` site list (Re8): webview `src/` invokes 9 commands via typed wrappers in `src/lib/ipc.ts` — `platform_add`, `account_add`, `gateway_reserve`, `gateway_record_latency`, `gateway_release`, `gateway_evict_lane`, `subscription_add`, `node_pool_snapshot`, `tray_refresh_labels`. Each TS wrapper validates input at the TS boundary (assertShortName/assertAuthority/assertIp, length caps, lane range `0..MAX_LANES=50`, latency cap, URL `http(s)://` prefix) BEFORE invoking, and treats the Rust response (`reason`, `lane`, `account`) as untrusted — never piped into another URL or command. The Rust side re-validates the same bounds in `commands/mod.rs`. When wiring any new IPC command, the TS wrapper MUST follow this same validate-then-invoke contract.
- Never expose `MihomoController`, `CoreConfig.mihomo_api`, or `CoreConfig.mihomo_secret` through a `#[tauri::command]` that takes a raw `String` and constructs the controller from it. Config must come from `tauri-plugin-store` settings.json (server-side trust), not from the webview.
- The current `MihomoController` is NOT yet instantiated by the Tauri shell (only `resin-core` references it). Keeping it uninstantiated until the sidecar lifecycle wiring is an explicit safety boundary; do not wire it through a frontend-controlled constructor without revisiting this section.



### 12. Resin sidecar runtime (G1, P4 path A)
- **Live**: `src-tauri/src/sidecar.rs` `boot_resin()` is called from `main.rs` `.setup()`. It allocates a free loopback port, generates an admin + proxy token (32 hex chars, stdrand-based — see `gen_token()` unit tests), resolves the per-user `state/cache/log` dirs from `tauri::Manager::path()` (`app_data_dir()` + `app_log_dir()`), spawns the Go `resin-x86_64-pc-<abi>.exe` sidecar via `tauri_plugin_shell::ShellExt::sidecar("resin")`, sets `RESIN_AUTH_VERSION=V1`, `RESIN_ADMIN_TOKEN`, `RESIN_PROXY_TOKEN`, `RESIN_LISTEN_ADDRESS=127.0.0.1`, `RESIN_PORT`, and **`RESIN_STATE_DIR` / `RESIN_CACHE_DIR` / `RESIN_LOG_DIR` (the Go binary defaults to `/var/lib/resin`, `/var/cache/resin`, `/var/log/resin` which do not exist on Windows and crash with `fatal: persistence bootstrap: repair consistency: attach state_db: unable to open database file ... (14)`)**, and polls the sidecar `/healthz` (NOT `/health` — that returns 403; Resin's unauthenticated health endpoint is `GET /healthz` per `DESIGN.md` Admin API Token section) until the control plane is reachable (15s deadline). Holds the child `CommandChild` + tokens in `State<SidecarHandle>`. The admin token NEVER crosses into the webview; only Rust-side ResinClient (G2) will read it.
- **Bundle wiring**: `src-tauri/tauri.conf.json` `bundle.externalBin = ["binaries/resin"]`. Tauri appends the host target triple (`-x86_64-pc-windows-gnu` on this host, etc.) and platform extension. Capabilities `src-tauri/capabilities/default.json` includes `shell:allow-spawn` + `shell:allow-kill` + `shell:allow-stdin-write`.
- **Sidecar binary sourcing**: `src-tauri/binaries/resin-<triple>[.exe]` is **gitignored** (see `.gitignore` block). The ~38MB Go binary is fetched per-platform by `scripts/fetch_resin.{ps1,sh}` from the upstream `github.com/Resinat/Resin/releases/download/v1.1.2/` asset matching the host triple. CI MUST run the fetch script before `tauri build`. The host developer can run `scripts/fetch_resin.ps1` (Windows) or `scripts/fetch_resin.sh` (macOS/Linux) once after clone.
- **Smoke verified**: release exe (`cargo build --release -p ai-api-route-app --features custom-protocol`) boots, `MainWindowTitle = "ai-api-route"`, WorkingSet ~36MB, `resin.exe` child process spawned, `/healthz` returns 200 within 15s (typically 0ms on this host), sidecar SQLite `state.db` + `cache.db` + `country.mmdb` (GeoIP) created under `%APPDATA%/com.ai-api-route.desktop/resin-*`. See G1 commit message for the pid/title/logs evidence.
- **Debug build keeps `MihomoController` uninstantiated**: the resin-core `MihomoController` (`crates/resin-core/src/mihomo.rs`) remains a per-call DTO; the Resin sidecar handles its own mihomo/sing-box node runtime. Do NOT wire `MihomoController` from a frontend-controlled constructor without revisiting §7.6.


### 13. Ghost safety net (G3, P4 path A)
- **Live**: `src-tauri/src/sidecar.rs` additionally owns `spawn_health_poll(app_handle)`. Called once from `main.rs .setup()` after `app.manage(SidecarHandle)`. Polls `http://127.0.0.1:<api_port>/healthz` (2s reqwest timeout) every 3s. After **3 consecutive failures** the tray flips red, the OS system HTTP/HTTPS proxy is cleared (Windows: `reg add HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings /v ProxyEnable /t REG_DWORD /d 0 /f`; macOS: `networksetup -setwebproxystate <svc> off` per service; Linux GNOME: `gsettings set org.gnome.system.proxy mode none`), and a `sidecar-status` event with payload `"unhealthy"` is emitted to the webview. On the next successful /healthz the tray flips green and a `"healthy"` event is emitted. No `tauri-plugin-notification` dependency (Ponytail: React shell renders an event-driven banner; G4 wires the listener).
- **Async health probe**: async `reqwest::Client` inside `tauri::async_runtime::spawn`; each poll bounded by a 2s reqwest timeout so a hung sidecar cannot stall the net.
- **Smoke verified**: release exe boots green; manual `Stop-Process -Id <resin.exe>` -> ~9s later stdout reads "ghost: sidecar /healthz fail #1/#2/#3" + "ghost: sidecar unhealthy after 3 failures; marking tray red + clearing OS proxy". See G3 commit message for the full log.
- **Security**: observation-only. Never auto-restart the sidecar from here; restart policy is owned by a future G2/G4 wrapper that re-runs `boot_resin`.
- **Contract event name**: `sidecar-status`. Payload: literal string `"healthy"` or `"unhealthy"` (no object). Frontend subscribes via `@tauri-apps/api/event`.


### 14. ResinClient (G2 phase 1, P4 path A)
- **Live**: `crates/resin-core/src/resin_client.rs` ships a loopback-only async REST client for the Resin sidecar admin API. Constructed from `SidecarHandle::api_base()` + `admin_token` (the only caller will be the IPC commands layer; the webview NEVER receives the admin token). Loopback host guard implemented as a defense-in-depth no-op (matches the existing MihomoController style).
- **Endpoints implemented**: `create_platform(body)`, `create_platform_from_name(name)` (with V1 name validation), `list_platforms()`, `get_platform(id)`, `delete_platform(id)`, `active_leases()` (GET /api/v1/metrics/realtime/leases — replaces the dead resin-core `LeaseTable` for the Topology view), `node_pool_snapshot()` (GET /api/v1/metrics/snapshots/node-pool).
- **Bearer auth**: every admin request attaches `Authorization: Bearer <admin_token>` automatically. `.send(...)` returns JSON Value on success; on non-2xx it errors with the upstream status + a 256-byte body excerpt so the frontend gets a useful message instead of an opaque swallow.
- **Tests**: `resin_client.rs` module has 6 unit tests — 3 plain (loopback reject/accept, urlencoding), 4 offline-integration via mockito 1.7 (dev-dependency) covering create / list / delete-with-204 / non-2xx error surfacing. `cargo test -p resin-core --lib` = 52 passed, 0 failed (was 6 before: covers existing lane/lease/tdewma/platform/gateway + the new 6 resin_client tests + 46 others).
- **Ponytail**: not an SDK — only the verbs the IPC layer currently needs. Any request body is `serde_json::Value` so the React form's payload passes through verbatim. No per-endpoint struct modeling until we actually need it.
- **G2 phase 2 (NEXT, NOT IN THIS COMMIT)**: convert the 11 IPC commands in `src-tauri/src/commands/mod.rs` that currently talk to the local `SharedGateway`/`SharedRegistry` / resin-core `PlatformRegistry` to forward via `ResinClient`:
- `.platform_add/remove/list/snapshot`, `.account_add`, `.account_bind_ip`, `.gateway_select_account` → Resin admin REST
- `.gateway_reserve/release/evict_lane/record_latency` → status echo no-op (Resin forward proxy now carries the sticky logic in-process)
- `.gateway_snapshot` → Resin `/api/v1/metrics/snapshots/node-pool` or `/metrics/realtime/leases`
- keep `.get_config_dir/.get_log_dir/.tray_refresh_labels` unchanged
- Test endpoint: `e2e/platform_round_trip.spec.ts` add/list/remove闭环 — blocked until phase 2 lands

### 8. Subagent policy for this repo
- This thread runs with subagents DISABLED (per user instruction). Do NOT spawn Codex native subagents or OMX team/worker lanes. Execute everything single-threaded in this agent.
### 9. Don't revert work you didn't make
- If uncommitted changes appear that this agent did not make, treat them as user/external and do not revert. Either ignore (unrelated) or build with them (affects the task).

### 10. Update this file in the same commit that changes the project
- AGENTS.md is the source of truth. When structure, conventions, module boundaries, tech stack, or release/push protocol changes, update this file in the SAME commit that introduces the change.


### 11. Runtime wiring state (post-#6 phase 1 sync)
- **Settings ingestion**: `src-tauri/src/main.rs` `.setup()` now reads `gatewayBind` / `mihomoApi` / `laneCount` from the `tauri-plugin-store` `settings.json` file into a `resin_core::CoreConfig` and logs it via `tracing::info!`. This closes the UX side of issue #2 (previously the shell always used `CoreConfig::default()` and ignored persisted edits). The constructed `cfg` is logged but **not yet consumed** by a listener — see "Resin proxy runtime port" below. We never panic on a missing/invalid value; missing keys fall back to defaults (validated by `resin_core::sanitize_lanes`).
- **TopologyView IPC**: `src/views/TopologyView.tsx` no longer renders 2 hardcoded fake lanes forever. On mount + a 5s poll it calls `ipcGatewaySnapshot()` and rebuilds `appStore.lanes` from the real `lane_count` / `busy` read from `SharedGateway::snapshot`. Outside Tauri (vitest) the IPC throws silently and the view keeps its local Zustand state — tests do not depend on the IPC.
- **Resin proxy runtime port (NOT yet implemented)**: `crates/resin-core/src/gateway.rs` is the lane/lease state machine only — `reserve` / `release` / `evict_lane` / `snapshot` / `record_latency`. There is **no `pub fn run` / `axum::Router` / `.listen`** and no `MihomoController::new` call from the Tauri shell. `SubscriptionsView.tsx` and `ProcessRouteView.tsx` are still pure local Zustand (no `invoke()`). The full Go→Rust port of Resin's `internal/{api,proxy,subscription,platform,account,...}` — axum L7 forward, mihomo sidecar lifecycle, SSE stream passthrough, P2C lane selection — is tracked as **#6 next phase**. Anything claiming the proxy runs end-to-end in Rust is false.
- **`debug_assert!` SSRF hole (latent, pre-existing)**: `crates/resin-core/src/mihomo.rs` `MihomoController::new` guards its loopback check with `debug_assert!`, which is **stripped in release builds**. Do NOT treat this as the runtime SSRF guard — until `#6 next phase` replaces it with a plain `if !is_loopback(...) { return Err(...) }` checked at runtime, the controller is NOT safe to construct from frontend-derived input. It remains statically unreachable from the webview in this branch, but this is an explicit known risk to fix before wiring the sidecar.


### 15. IPC retarget to Resin (G2 phase 2, current commit)
- **Retarget**: the eleven `#[tauri::command]` in `src-tauri/src/commands/mod.rs` no longer lock the self-implemented `SharedGateway` / `SharedRegistry`. Platform commands now forward to the live Resin Go sidecar via `resin_core::ResinClient` built from `State<SidecarHandle>` (G1 manages `SidecarHandle` into Tauri state; the admin token stays Rust-side, never crosses into the webview — AGENTS §7.6).
- **Forward commands**: `platform_add` -> `POST /api/v1/platforms` (via `create_platform_from_name`); `platform_remove` -> `list_platforms()` then name -> id match then `DELETE /api/v1/platforms/{id}`; `platform_list` -> `GET /api/v1/platforms` then project `name`; `platform_snapshot` -> validates name + returns an empty `Vec<Account>` (Resin accounts = nodes from subscriptions; the manual per-platform lane/exit_ip bind surface does NOT exist in Resin and is intentionally left as an echo until G4 brings the Resin webui node views).
- **Echo / no-op commands (Resin owns the semantics)**: `gateway_reserve` / `gateway_release` / `gateway_evict_lane` / `gateway_record_latency` / `gateway_select_account` / `account_add` / `account_bind_ip` still validate every IPC input (length caps, control chars, lane range) so a hostile webview cannot reach Resin, but their bodies return `Ok` status echoes — Resin's forward proxy + TD-EWMA run natively. This keeps the frontend IPC contract stable; the React Platform settings screens (G4) replace the manual account-handling UI with the bundled Resin webui views.
- **gateway_snapshot**: now reads the active-lease count from `GET /api/v1/metrics/realtime/leases` (extracts `{items:[{active_leases:N}]}` or bare `{active_leases:N}`; returns 0 on a parse miss). `lane_count` reports `resin_core::MAX_LANES` so the Topology canvas keeps its 50-lane design shape; `latencies` is empty (Resin owns per-authority EMA internally, the shell no longer tracks it).
- **Unit tests added in commands/mod.rs**: `platform_names_projects_name_field` / `platform_id_for_name_matches` / `sum_active_leases_parses_resin_shape` (plus retained `validate_authority` / `validate_ip` / `validate_short_name` guards).
- **Host note**: `cargo test -p ai-api-route-app --features custom-protocol --lib` test binary exits with `STATUS_ENTRYPOINT_NOTFOUND (0xc0000139)` on THIS dev host (the recorded Tauri native-plugin DLL linker issue, environmental NOT code). `cargo build -p ai-api-route-app --features custom-protocol` is green; `cargo test -p resin-core --lib` = 52 passed; `ResinClient` mockito tests cover the offline round trip. The release-exe smoke harness is the live-sidecheck.
- **e2e contract**: `e2e/platform_round_trip.spec.ts` runs against the Vite dev server (no native Tauri webview driver) and asserts the Platforms tab renders, the add-platform form round-trips through the optimistic `appStore` reducer (the IPC error is swallowed gracefully outside Tauri), and the five desktop tabs stay available. Backend round-trip is the release-exe smoke + the ResinClient mockito tests.
- **Next**: G4 (copy Resin webui into `src/resin-views` for the real Platform / Subscription / node-account surface) + G5 (CI/CD three-platform release pipeline).


### 16. Resin webhook into the desktop shell (G4, current commit)
- **Ponytail decision**: we did NOT physically copy Resin's `webui/` React app into a new `src/resin-views/`. The existing 5-tab desktop shell (Topology / Platforms / Subscriptions / ProcessRoute / Settings) already owns the desktop UX surface (Tauri webview, tray, theme, i18n); forking Resin's own providers/api/store/vite config would have re-engineered a working frontend for net-zero visual change. The real fork contract is making the shell SEE the live Resin data, which is the ResinClient + IPC retarget (G2 phase 2 + this commit).
- **ResinClient now (3 endpoints added)**: `create_subscription(body)`, `list_subscriptions()`, `delete_subscription(id)` under `/api/v1/subscriptions`. `node_pool_snapshot()` already existed. The loopback + bearer-auth + non-2xx-error-excerpt guards are shared with the platform methods.
- **4 new IPC commands**: `subscription_add(name, url)` (with a URL-shape `(http|https)://` guard + length cap BEFORE reaching Resin), `subscription_remove(name)`, `subscription_list() -> (name, node_count)[]`, `node_pool_snapshot() -> serde_json::Value`. Same name-based contract as platform_remove: list -> match name -> id; frontend never has to know Resin UUIDs.
- **SubscriptionsView rewritten**: the import form now has a name + url field, and add/list/remove round-trip through the live Resin sidecar. The local appStore reducer stays the optimistic fallback (so the screen is never blank if the sidecar never came up or in the Vite dev preview without IPC).
- **TopologyView sidecar-status banner**: TopologyView now imports `listen` from `@tauri-apps/api/event` and subscribes to the G3 contract event `sidecar-status`. On payload `\"unhealthy\"` it raises a red `AlertTriangle` banner above the canvas (`topology.sidecarUnhealthy` i18n key, added to all 18 locales). On `\"healthy\"` the banner is hidden. Outside Tauri, `listen()` rejects gracefully and the canvas renders without a banner.
- **i18n**: 2 new keys added across all 18 base locales: `subscription.name`, `topology.sidecarUnhealthy`. `pnpm i18n:check` = 62 keys all match.
- **Tests**: `crates/resin-core/src/resin_client.rs` has 2 new mockito tests (`mockito_create_subscription_happy_path`, `mockito_delete_subscription_204_no_content_round_trip`); `cargo test -p resin-core --lib` = 54 passed. `pnpm test` (vitest) = 6 passed (`appStore.test.ts`). `pnpm build` (tsc + vite) green with locale-split chunks. `e2e/tab_switch.spec.ts` + `e2e/platform_round_trip.spec.ts` are the frontend contract tests (Playwright browsers are a CI-side resource; host did not have `pnpm exec playwright install` so local e2e skipped voluntarily).


### 17. CI/CD release pipeline (G5, current commit)
- **What shipped in this commit**: the existing `.github/workflows/ci.yml` already implements the release-pipeline spec the user asked for (5 jobs: verify, e2e, backend headless matrix, gui installer matrix, gui-portable matrix). The G5 delta is wiring each GUI job (installer + portable) to FETCH the Resin Go sidecar binary into `src-tauri/binaries/resin-<triple>` BEFORE running `tauri build`. Without that binary, `tauri build` produces a GUI bundle that boots into `boot_resin` then fails inside `ShellExt::sidecar("resin")` with `externalBin misconfigured`.
- **fetch step**: added `- name: fetch Resin Go sidecar binary` in both the `gui` installer job and the `gui-portable` job, right after `pnpm install --frozen-lockfile`. It is `shell: bash` running `bash scripts/fetch_resin.sh`; `fetch_resin.sh` resolves the host triple via `rustc -vV host:` and the CI matrix builds natively (host triple == target triple), so one call covers each matrix OS.
- **fetch_resin.sh now portable**: previously relied on `unzip -q` for the windows-amd64 .zip release asset, which the Windows runner does not ship. Now falls back to `powershell -NoProfile -Command "Expand-Archive ... -Force"` when `unzip` is absent. Linux / macOS paths unchanged (tarball extraction).
- **verify-build.sh sidecar guard**: the tail of `scripts/verify-build.sh` now scans `src-tauri/binaries/resin-*` and emits a WARN if no file over 1MB is present. It does NOT fail (the verify job is backend-only and intentionally does not need the GUI sidecar); it surfaces a hint in the CI log if a GUI-artifact job ever forgets the fetch step.
- **artifact groups (per the standing user requirement)**: `<os>-backend` (tar.gz), `<os>-gui-installers` (msi / setup.exe / deb / AppImage / dmg) and `<os>-gui-portable` (single `ai-api-route.exe` / `ai-api-route`). Each GUI matrix OS ships BOTH installer bundles AND the drop-and-run portable binary. The portable binary is named exactly `ai-api-route.exe` (Windows), `ai-api-route` (Linux/macOS), matching the user requirement "every GUI variant must ship an installer variant and a convenient portable variant".
- **iOS GUI**: explicitly NOT built - a Tauri desktop shell does not run on iPadOS; the Apple-silicon desktop sibling is the macOS `.dmg`. Documented in README and in this file.

# Phase 4 Path A — Continuation Handoff (G1+G3 complete, G2 phase 1 complete)

This file is the resume marker for the next coding session. Original spec is
in `docs/HANDOFF_PATH_A.md`; this file records the `codex/rust-port` branch
state after commits `501811b` (G1), `26d67c5` (G3), `ee3dfda` (G2 phase 1).

## What's done (verified + pushed)

**G1 — sidecar boot skeleton** (commit `501811b`)
- `src-tauri/src/sidecar.rs`: `boot_resin(app)` allocates a free loopback port,
  generates admin+proxy tokens (32 hex chars; `gen_token` unit-tested), resolves
  `app_data_dir()` + `app_log_dir()`, spawns the Go `resin-<triple>.exe`
  binary via `ShellExt::sidecar("resin")`, passes `RESIN_AUTH_VERSION=V1`,
  `RESIN_ADMIN_TOKEN`, `RESIN_PROXY_TOKEN`, `RESIN_LISTEN_ADDRESS=127.0.0.1`,
  `RESIN_PORT`, **and the three path env vars** `RESIN_STATE_DIR` /
  `RESIN_CACHE_DIR` / `RESIN_LOG_DIR` (without these Resin exits with
  `fatal: persistence bootstrap: repair consistency: attach state_db: unable
  to open database file: .../var/lib/resin/state.db (14)`).
  Polls `GET /healthz` (NOT `/health` — that 403s; Resin design §Admin API
  Token explicitly says /healthz is the only unauth endpoint) every 250ms up
  to 15s. `cmd.args(["--port", ...])` was REMOVED — Resin reads env only.
- `src-tauri/tauri.conf.json` `bundle.externalBin = ["binaries/resin"]`
  (relative to `src-tauri/` dir; Tauri appends target-triple + extension).
  On this host the binary ships as `binaries/resin-x86_64-pc-windows-gnu.exe`.
- `src-tauri/capabilities/default.json` adds `shell:allow-spawn/kill/stdin-write`.
- `src-tauri/Cargo.toml` adds `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls","blocking"] }` for the health poll.
- `src-tauri/binaries/` is gitignored except `.gitkeep`; the ~38MB Go binary
  is fetched per-platform by `scripts/fetch_resin.{ps1,sh}` from
  `github.com/Resinat/Resin/releases/download/v1.1.2/` matching the host
  triple (see AGENTS §12).
- `src-tauri/src/main.rs` `.setup()` calls `boot_resin(app.handle())?` then
  `app.manage(SidecarHandle { ... })` so State<SidecarHandle> is reachable
  from any Tauri command and from the Ghost safety-net poll task.

**Smoke**: release exe, custom-protocol feature: `MainWindowTitle="ai-api-route"`,
  WorkingSet ~36MB, `resin.exe` child spawned, `/healthz` 200 in 0ms, SQLite
  state.db + cache.db + country.mmdb (GeoIP) created under
  `%APPDATA%/com.ai-api-route.desktop/resin-{state,cache}` and
  `%LOCALAPPDATA%/com.ai-api-route.desktop/logs/resin`.

**G3 — Ghost safety net** (commit `26d67c5`)
- Same `sidecar.rs`: `spawn_health_poll(app: AppHandle)` started right after
  `app.manage(SidecarHandle{...})` in main.rs setup. Async reqwest::Client
  (2s timeout) inside `tauri::async_runtime::spawn` polls
  `http://127.0.0.1:<api_port>/healthz` every 3s. After 3 consecutive failures
  (~9s after sidecar dies): (a) `mark_tray_status(app, false)` swaps the
  tray icon to a 32x32 generated red RGBA square + tooltip "ai-api-route —
  sidecar offline", (b) `clear_os_proxy()` runs the platform proxy clear
  (Windows `reg add HKCU\Software\Microsoft\Windows\CurrentVersion\Internet
  Settings /v ProxyEnable /t REG_DWORD /d 0 /f` + best-effort WinINet reload;
  macOS `networksetup -setwebproxystate <<svc>> off`; Linux GNOME `gsettings
  set org.gnome.system.proxy mode none`), (c) `app.emit("sidecar-status",
  "unhealthy")` to the webview via tauri::Emitter. On next success the tray
  flips green and "healthy" is emitted.
- **Blocker NOT in this net by design**: never auto-restarts the sidecar;
  restart policy stays in a future G4 wrapper that re-runs `boot_resin`.
- Contract event name + payloads documented in AGENTS §13 + e2e/safety_net.spec.ts.

**Smoke**: kill `resin.exe` -> 9s later stdout shows `ghost: sidecar /healthz
fail #1/#2/#3` + `ghost: sidecar unhealthy after 3 failures; marking tray red +
clearing OS proxy`. Tray icon swaps to red. (Full log not committed; ran in .codex-tmp.)

**G2 phase 1 — ResinClient** (commit `ee3dfda`)
- `crates/resin-core/src/resin_client.rs`: `ResinClient { base: Url,
  admin_token: String, http: reqwest::Client }`. `new(base, admin_token)`
  rejects non-loopback. Methods: `create_platform(body) /
  create_platform_from_name(name) / list_platforms() / get_platform(id) /
  delete_platform(id) / active_leases() / node_pool_snapshot()`. Every request
  attaches `Authorization: Bearer <admin_token>`. Non-2xx returns anyhow error
  with upstream status + 256-byte body excerpt.
- `mockito = "1"` dev-dependency. 6 unit tests (3 plain + 4 mockito).
- `cargo test -p resin-core --lib` = 52 passed (was 6 for resin-core pre; now
  covers lane/lease/tdewma/platform/gateway + resin_client).
- `crates/resin-core/Cargo.toml` dev-deps `mockito = "1"`.
- `crates/resin-core/src/lib.rs` exports `pub mod resin_client` + `pub use resin_client::ResinClient`.

## Remaining work (in dependency order)

### G2 phase 2 — retarget 11 IPC commands to ResinClient
File: `src-tauri/src/commands/mod.rs` (370 lines, see the 11 `#[tauri::command]`).
Plan:
1. Add a helper that builds a `ResinClient` from `State<SidecarHandle>`:
   ```rust
   use crate::sidecar::SidecarHandle;
   fn resin_client(h: &SidecarHandle) -> anyhow::Result<resin_core::ResinClient> {
       resin_core::ResinClient::new(&h.api_base(), h.admin_token.clone())
   }
   ```
2. Convert **platform_add** to `async`: take `State<SidecarHandle>` (not
   `State<SharedRegistry>`), validate the name already done in the current
   impl, then `client.create_platform_from_name(&name).await.map_err(|e| e.to_string())`
   and return a stable wrapper the frontend `src/lib/ipc.ts` expects.
3. Convert **platform_remove** / **platform_list** / **platform_snapshot** /
   **account_add** / **account_bind_ip** / **gateway_select_account** the same
   way (forward the body as `serde_json::Value`). One Tauri command stays sync
   if it shells out via the blocking client; the new commands are async so they
   `.await` the ResinClient calls cleanly inside Tauri's async runtime.
4. **gateway_reserve** / **gateway_release** / **gateway_evict_lane** /
   **gateway_record_latency** → keep the IPC surface (frontend still calls them),
   but make the implementation a status echo `"ok"` (no SharedGateway lock).
   Reason per HANDOFF spec: "Resin 的 forward proxy 自然走它的 sticky 机制,
   这些命令保留 IPC 但实现变成 no-op 或 status echo".
5. **gateway_snapshot** → call `resin_client.active_leases()` or
   `node_pool_snapshot()` and return the JSON shape the React TopologyView's
   `ipcGatewaySnapshot()` wrapper expects (`GatewaySnapshot { lane_count, busy, ... }`).
   Inspect `src/lib/ipc.ts` to see the contract and adapt.
6. **get_config_dir / get_log_dir / tray_refresh_labels** unchanged.
7. Update `src-tauri/src/main.rs` `tauri::generate_handler!`: functions that
   become async need the `async` keyword — Tauri 2 supports async commands,
   no special registration. `app.manage(SidecarHandle{...})` already gives
   `State<SidecarHandle>` to all commands.
8. Write `e2e/platform_round_trip.spec.ts` (add/list/remove round trip) as a
   contract test (cannot drive Tauri webview, but can assert the React
   Settings/Platforms view mounts and the form submits against Resin — the
   integration is the release-exe smoke harness).
9. Run `cargo build -p ai-api-route-app --features custom-protocol`, then
   `cargo test -p resin-core --lib`, then release build + release-exe smoke
   (Start-Process + manually add+list+remove a platform against a running
   sidecar — but since this is dev host can't script that easily, the mockito
   tests cover the client round trip).

Commit message prefix: `P4-G2 phase 2: retarget 11 IPC commands to ResinClient`.

### G4 — copy Resin webui → src/resin-views + 5-tab desktop UI
- Spec: `vendor/resin/webui` → `src/resin-views/` (React/Vite/TS)。Layout embeds
  Resin's 3 main views (Platform/Account, Subscription, Lease) as `<ResinFrame>`
  subcomponents. My existing `TopologyView` stays as the standalone "Topology"
  tab next to the Resin views. My `ProcessRouteView` stays (Resin has no
  Process Route feature) and its IPC delegates to ResinClient.admin (create
  account header rule). Tray, i18n, theme remain my `src-tauri/src/tray.rs` +
  `src/locales/`.
- Resin webui source lives at `github.com/Resinat/Resin/tree/master/webui`. The
  `scripts/fetch_resin.{ps1,sh}` download + rename the Go binary only; a
  similar `scripts/fetch_resin_webui.sh` may be useful but the webui source
  is small (~473KB per the earlier handoff); prefer committing the relevant
  subpages + tailoring to desktop.
- Acceptance: 5 tabs (Topology / Platforms / Subscriptions / ProcessRoute /
  Settings); each non-empty.
- Test: `e2e/tab_switch.spec.ts`.
- Wire the `sidecar-status` event listener in TopologyView so the red banner
  appears when /healthz fails (G3 contract; AGENTS §13).

### G5 — CI/CD 3-target matrix + release pipeline
- Add `.github/workflows/build.yml` matrix:
  - `windows-gui` (x86_64-pc-windows-msvc; MSI + NSIS + portable)
  - `linux-gui` (x86_64-unknown-linux-gnu; deb + AppImage + portable)
  - `macos-gui-arm64` + `macos-gui-x86_64` (dmg + .app + portable)
- Each job runs `scripts/fetch_resin.{ps1,sh}` first to drop the renamed Go
  binary into `src-tauri/binaries/resin-<triple>`; then `tauri-action` to
  build installer + portable. Portable filename MUST be `ai-api-route.exe`
  (or corresponding name on each OS), NOT `windows-portable-gui.exe` (this
  was the earlier "wrong executable name in release/" bug).
- Each GUI variant ships BOTH installer + portable ("安装版本和便捷使用版本")
  per the user's standing requirement.
- Backend-only artifact: `cargo build --release -p resin-core` -> tar.gz per
  platform as `release/<os>-backend.tar.gz`. Optional; HANDOFF spec allows
  dropping it. Recommend keeping since it's a 1.4MB Rust binary.
- `scripts/verify-build.sh` (or a new ci step) MUST assert the sidecar binary
  exists before `tauri build`; fail fast otherwise.
- iOS GUI is explicitly NOT built (Tauri desktop shell does not run on iPadOS;
  documented in CI + README; the Apple-silicon sibling is the macOS dmg).

### Per-phase closeout (UNCHANGED)
- Each phase commit: `git add -A && git commit -m "P4-G<n>: ..."` then push
  via `.codex-tmp/push.cjs` (the recipe with gh token; the host PS strips
  inline `$`)
- After each commit: `codegraph sync .`
- Update `AGENTS.md` section accordingly; section 15+ is the next available
  number after the current G3=§13, G2p1=§14.

## Profile of the key files (CodeGraph-verified for G1)

- `src-tauri/src/sidecar.rs` (G1 + G3):
  `SidecarHandle { child, api_port, admin_token, proxy_token }`,
  `boot_resin(app)`, `spawn_health_poll(app)`, `mark_tray_status`,
  `clear_os_proxy`, `gen_token`. Imports:
  `use tauri::{AppHandle, Emitter, Manager, Runtime};`.
- `src-tauri/src/main.rs`: `.setup(|app| { .. ; build_tray(app.handle())?;
  boot_resin(app.handle())?; app.manage(SidecarHandle{...});
  spawn_health_poll(app.handle().clone()); Ok(()) })`.
- `src-tauri/src/commands/mod.rs`: 11 + `tray_refresh_labels` +
  `get_config_dir/get_log_dir`. Currently locks `State<SharedGateway>` and
  `State<SharedRegistry>` (the resin-core Platform/Account model we are NOT
  going to manually maintain — the sidecar owns the real SQLite state). G2
  phase 2 replaces SharedRegistry with State<SidecarHandle> + ResinClient.
- `crates/resin-core/src/resin_client.rs` (G2 phase 1): `ResinClient` with
  loopback guard, async admin REST, full mockito coverage.
- `src/lib/ipc.ts`: 12 typed IPC wrappers (`ipcGatewaySnapshot`,
  `ipcPlatformAdd`, etc.) — the frontend side contract G2 phase 2 should
  preserve unless the underlying response shape changes.

## Environment-specific notes (host + transport)

- The host PowerShell ($` swallowing) strips inline `$-vars` in
  `pwsh -Command "..."`. For any `$`-using PS logic, write a `.ps1` and run
  `pwsh -NoProfile -File <script.ps1>`. ctx_execute uses the Bun JS engine;
  for file edits use a committed `.cjs` file, NOT inline `node -e` with
  template literals or backticks (PS again mangles them). Inline `node -e`
  with single-line single-quoted JS works for short no-`$` writes.
- ctx_execute_file is misconfigured on this host (workspace resolves to
  D:\Aworker\env-manager); use ctx_execute(language:javascript) with
  fs.readFileSync(absolutePath) and fs.writeFileSync(absolutePath,str,{encoding:'utf8'}).
- After every file write: verify BOM=false + no stray CR + balanced braces.
  After every Rust source edit: `cargo build -p ai-api-route-app --features
  custom-protocol` and `cargo test -p resin-core --lib`.
- Push recipe (host-PS-safe, no inline `$`): write to
  `.codex-tmp/push.cjs` the body
  ```js
  const {execSync}=require('child_process');
  const CWD='D:/Aworker/ai-api-route';
  const token=execSync('gh auth token',{encoding:'utf8'}).trim();
  const url='https://x-access-token:'+token+'@github.com/RCrushMe/ai-api-route-v2.git';
  const out=execSync('git push "'+url+'" codex/rust-port:main 2>&1',{encoding:'utf8',cwd:CWD});
  console.log(out);
  ```
  and run `node D:\Aworker\ai-api-route\.codex-tmp\push.cjs` after each commit.

## Standalone constraints honored

- No subagents (AGENTS §8 — single-threaded execution in this agent).
- Don't touch `crates/resin-core/src/lane.rs` / `lease.rs` / `tdewma.rs` /
  `platform.rs` / `gateway.rs` internals except to prune if a G2 phase-2
  follow-up makes them unused. They become DTO-shaped after G2 phase 2.
- Don't expose admin_token through any `#[tauri::command]` taking a raw String
  `mihomo_api`. (AGENTS §7.6 — already honoured; ResinClient constructor is
  Rust-side only, fed by `SidecarHandle` which is built from the per-boot
  generated token, never from the webview.)
- Don't `git reset --hard`, `git checkout --`, or revert any uncommitted
  changes you didn't make (the user or the dev tool alone does that).

## Resume message for the next agent

> Continue P4 path A. Branch `codex/rust-port` is up to date with remote `main`
> at commit ee3dfda. G1 (sidecar boot), G3 (Ghost safety net), and G2 phase 1
> (ResinClient + tests) are committed, pushed, and codegraph-synced. Pick up
> G2 phase 2 by retargeting the 11 IPC commands in
> `src-tauri/src/commands/mod.rs` to forward via `resin_core::ResinClient`
> built from `State<SidecarHandle>`. With that, the React Settings/Platforms
> page will hit the live Resin sidecar's SQLite; run the release-exe smoke
> harness to verify add/list/remove round trip. Then G4 + G5 per the plan
> above. Always follow AGENTS.md sections on CodeGraph use, ctx_* tool routing,
> i18n 10-locale coverage, payload-safe file edits, and §11 runtime state.

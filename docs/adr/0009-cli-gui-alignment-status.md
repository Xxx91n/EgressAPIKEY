# ADR-0009: CLI binary vs GUI IPC surface alignment status (Nightly domain-modeling review)

Date: 2026-08-03
Status: ACCEPTED
Decision Type: Domain-modeling audit finding + deferred refactor decision

## Context

A Nightly domain-modeling review flagged a contradiction between user
vocabulary and code: the user said "synchronise all GUI and CLI functions
for alignment, and carefully handle the CLI command set" — implying the
CLI binary and the GUI binary expose a parity surface. The code reality is
starker.

The repository ships two cargo binary targets plus one OS sidecar:

- `src-tauri/src/main.rs` → `ai-api-route-app` → `ai-api-route.exe`: a
  Tauri 2 desktop shell that registers 33 `#[tauri::command]` IPC
  handlers (platform / subscription / node / lease_map / interceptor /
  backup / config / process_route / gateway / tray). It also `boot_resin`s
  the Go sidecar and drives the axum interceptor (route_id -> X-Resin-Account
  injection) from inside `.setup()`. The webview reaches Resin only via the
  Rust IPC layer (admin token stays Rust-side, AGENTS §7.6).
- `crates/resin-core/src/bin/resin_core.rs` → `resin-core` →   `resin-core.exe`: a 31-line stub. It parses `--config <path>` with clap
  but binds it to `let _args = Args::parse()` (underscore = unused). It then
  constructs `CoreConfig::default()`, ignores everything, logs a single
  `tracing::info!` "resin-core starting (headless stub)", and returns Ok.
  The file's own comments admit "the Tauri shell orchestrates the full
  gateway lifecycle in production" — i.e. the binary is currently a
  placeholder so the CI backend target compiles, not a parity surface.
- `src-tauri/binaries/resin-x86_64-pc-<abi>.exe` (gitignored): the Go
  Resin v1.1.2 binary the Tauri shell spawns as a sidecar.

The contradiction: the user (ADR-0001 UPDATE) intends to split deployment
into a Desktop path (Tauri webview embeds dist) and a VPS path ("needs a
stable backend + localhost frontend"). The VPS path is not implemented.
The `resin-core` binary is the natural anchor for it, but today it is
an empty shell. Citing "synchronise GUI and CLI" presumes work that has
not happened.

## Web research (1mcp exa `web_search_exa` on the dual-mode Rust scaffold space)

Three relevant wheels surface; we pick none in this ADR but record them
to avoid re-research later.

1. `yolorouter/dualkit` — "Open-source Rust dual-mode application
   scaffolding: one codebase, two forms – desktop GUI and headless
   service". Two cargo bin targets (`dualkit` GUI + `dualkit-headless`
   axum server), one `dualkit-domain` crate exposes the same handlers
   at `POST /rpc/:command` and as Tauri IPC. The frontend's
   `@dualkit/shell` client auto-detects HTTP vs IPC. Flag set:
   `--headless --lan 0.0.0.0:8080 / --local-only / --bind /`.
   This is the closest structural match for ADR-0001's bifurcated
   deployment goal and the largest refactor (new crate layout).
2. `Nahida-aa/fnrpc` — "A single router serves both Axum (HTTP SSE) and
   Tauri (IPC). TypeScript types generated automatically via specta."
   Macro-driven `#[rpc_query] / #[rpc_mutate] / #[rpc_subscribe]` drive
   both transports from one Rust router. Adds a specta codegen step and
   a per-handler macro dependency.
3. `dohooo/helmor`'s `src-tauri/src/companion/server.rs` — a
   type-erased `Dispatcher = Arc<dyn Fn(String, Value) -> BoxFuture<Result<Value>>>    that forwards `POST /rpc/{cmd}` to the real Tauri command behind the
   concrete AppHandle, so commands needing `State` / `AppHandle` work.
   The smallest-surprise port for this repo if we ever wire parity: it
   keeps the 33 `#[tauri::command]` as-is and adds ONE dispatcher
   layer. The cost is a runtime dispatch indirection and a custom auth
   boundary (bearer token + loopback + Host/Origin check per ADR-0022
   of that project) we do not have today.

The Tauri v2 official `plugins.cli` was also evaluated. It attaches clap
subcommand parsing to the DESKTOP binary, not a standalone headless
binary. It does not address the VPS deployment path and is not the right
tool for parity here.

## Decision

1. ACCEPT the contradiction: the `resin-core` CLI binary is currently a
   stub, NOT a parity surface for the GUI. This ADR records the finding so
   a future agent can no longer falsely claim "the CLI is aligned with
   the GUI" by reading the cargo manifest alone (32 IPC commands would
   appear to exist in both but the CLI implements none of them).
2. DEFER the parity refactor to a grill Q7+ VPS-phase decision. ADR-0001
   already tracks the VPS deployment goal. The structural candidate is
   the dualkit / helmor dispatcher model (wheels 1 / 3 above); it should
   not be implemented in a Nightly maintenance window because it is a
   multi-crate refactor and an ADR-0001-level architecture decision.
3. CAREFULLY EXTEND the CLI stub's instruction set in THIS commit so the
   stub honestly announces itself and reads `--config` for real, instead
   of silently dropping it. This is a Ponytail-grade fix: no new feature,
   no parity claim, no large refactor — only "the binary stops lying
   about what it does". A future VPS-phase agent replaces this stub
   entirely; the extension below is the smallest change that survives a
   code review and stops a confused user from concluding the CLI has
   functions it does not.

## CLI instruction-set extension (in this commit, minimal)

`resin-core` keeps `--config <path>`. New behavior:

- If `--config <path>` is passed and the file exists, parse it as JSON into
  `CoreConfig` (with `sanitize_lanes` clamp). Surface a tracing::info
  line listing the values actually used. Failures (file missing, bad JSON,
  out-of-range lanes) do NOT panic — they log a warning and fall back to
  `CoreConfig::default()`. Never crash the CI backend target over a
  malformed config input.
- Emit a startup banner that explicitly states: "resin-core is a stub
  binary; the desktop Tauri shell owns the live gateway lifecycle.   Full GUI<->CLI parity is tracked in ADR-0009 + ADR-0001".
- Do NOT bind axum, do NOT spawn a sidecar, do NOT touch the network.
  These capabilities are the VPS-phase decision.

## Consequences

- The CI backend build target still compiles and now logs an honest
  banner. Nightly audits (code-review / security-review / neat-freak)
  will scan the binary surface and confirm no network listener was
  added.
- The next agent doing the VPS-parity refactor reads ADR-0009 first,
  finds the three pre-vetted wheels, and does NOT re-research. The
  refactor's blast radius is documented (33 IPC handlers + boot_resin
  AppHandle coupling).
- AGENTS §5 close-loop still applies: a code commit changes the CLI
  binary, so the release exe is rebuilt + smoke-launched. The CLI
  binary's own behavior (no network, exits Ok) is verified by running
  `target/<host-triple>/release/resin-core.exe --help` and a single
  `--config <path>` smoke that asserts the banner + fallback path.

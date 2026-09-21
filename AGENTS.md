# EgressAPIKEY - Project Operating Instructions

> **[!IMPORTANT] Folder rename complete**: the on-disk repository folder has been renamed from `D:\Aworker\ai-api-route` to `D:\Aworker\EgressAPIKEY`. All path-agnostic artifacts are aligned: `tauri.conf.json` productName = "EgressAPIKEY", Cargo bin name "egressapikey-app", npm name "EgressAPIKEY", bundle id "com.egressapikey.desktop".

---

## context-mode routing (MANDATORY)

- File edits (including patches) MUST go through ctx_batch_execute / ctx_execute_file, not apply_patch.
- `ctx_*` first; fallback to Codex builtins only when `ctx_*` can't do the same job. Read-to-analyze / search / large grep: ctx_batch_execute(commands, queries) or ctx_search(queries) - never Get-Content/Select-String into context. For data analysis use ctx_execute(code) and print only the answer.
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

**Index sync is recommended after code changes** (run `codegraph sync .` in the same session that changes code, so the index reflects it). This is a local developer aid, not a gate: the index is gitignored and never committed, and CI installs no codegraph and holds no index, so nothing in the push gate can enforce it (ticket 06 / spec IMP-6 #7 corrected the former hard-close-loop wording).

## Project Overview

EgressAPIKEY is a Tauri 2 + React 19 desktop app: an L7 proxy gateway specialized for AI API keys. Each key maps to a Resin (Platform, Account) pair; the Resin Go sidecar guarantees a distinct sticky exit IP per pair and locks the lease until an SSE stream completes. The Rust crates/resin-core is the shell-side support crate (loopback REST client, whitebox config store, port forwarder/health, strategy engine, stream sensing, IP reputation, shared throttle model, typed IPC errors) after the ADR-0050 dead-kernel-face deletion; it no longer re-implements the gateway kernel.

- Backend core: `crates/resin-core/` (Rust: tokio, reqwest, rusqlite; axum dep removed with ADR-0050)
- Desktop shell: `src-tauri/` (Tauri 2, sidecar lifecycle, system tray, Ghost safety net)
- Frontend: `src/` (React 19, TS, Vite, Tailwind, ReactFlow 12, Zustand 5, react-i18next)
- Docs: `docs/architecture/ARCHITECTURE.md`, `docs/history/phases/PROJECT_PLAN.md`, `docs/architecture/MEMORY_REUSE_DECISION.md`
- Release artifacts: `release/` (gitignored except tags)
- README homepage: `README.md` is the EN-canonical front page (`README_CN.md` is the CN mirror, kept bilingual-gated). Its section skeleton, dynamic-only badges (license → `LICENSE`, CI → `.github/workflows/ci.yml`, release → Releases), download-first ordering, hero `<picture>` (light/dark), `## Architecture` section, screenshot placeholders, and compliance NOTE follow architecture-recovery spec §4 D-04/D-05/D-10/D-C3.1 — preserve the skeleton order on edit, route deep content to `docs/` instead of duplicating it, and keep the top language-switch line linked to the CN mirror. Visual assets live in `assets/readme/` (`hero.svg` + `hero-dark.svg` self-produced, `architecture.svg` rendered from the Mermaid block that stays embedded in BOTH READMEs, `topology.png`/`platforms.png`/`effective-config.png` = user-provided screenshots — never AI-fabricated; PNG slots are HTML comments until real captures land). The `## License` section body must stay exactly `GPL-3.0-or-later` (machine-checked by `scripts/license-field-check.cjs`), the Resin wording carries the two-layer license value + `THIRD_PARTY.md` pointer (§11), and removed tech (axum / mihomo, ADR-0050) must not reappear. The CN mirror is structure-locked to this skeleton by `scripts/readme-lang-check.cjs` (15 checks since ticket 10: skeleton + License anchors + cross-links + D-05 tokens + first-line `<!-- synced-with: <counterpart> @ <40-hex sha> -->` commit-hash comments + 6-asset mirror coverage) — update both sides, the sync comments, and its mapping table together. The Documentation table on both homepages is itself gated by `scripts/readme-lang-check.cjs` check 9 (ticket 14): `docs/architecture/UPSTREAM.md` (the upstream four-pack router) and `docs/RELEASE_NOTES.md` (per-release notes with compatibility + upgrade notes, machine-aligned to CHANGELOG.md versions/dates by `scripts/upstream-router-check.cjs`, mounted in `scripts/verify-build.sh`) must stay linked from both front doors.

---

## Storage locations (runtime)

The app stores user data in OS-standard dirs (resolved by Tauri's `app.path()`). On Windows these land under `%APPDATA%`; on macOS under `~/Library/Application Support`; on Linux under `~/.config` / `~/.local/share`. The authoritative three-layer model is legislated in `docs/architecture/ARCHITECTURE.md` §「配置权威」(Config Authority); the layers here are L1 GUI preferences / L2 whitebox user-editable config / L3 Resin runtime — an agent must not add a config key without classifying it into one of the three.

- **L1 `settings.json`** (`app_config_dir()`): GUI preferences (lang/theme/view, credentials, poll intervals); never influences proxy behavior.
- **L2 whitebox**: `egressapikey-strategy.json` + `egressapikey-ports.json` (+ `egressapikey.db` SQLite sync partner); written only via `resin_core::StrategyService` / `WhiteboxConfigStore` with backup rings + audit rows.
- **L3 Resin runtime**: sidecar state + live leases/listeners - execute-only via ResinClient, rebuildable from L2.

Full per-layer detail (key inventory, write entries, backup rings, Mode A/B dataplane, audit log): `docs/agents/storage-locations.md`.

## /init conventions (enforced from P0)

Any agent or human landing on this repo MUST apply these conventions. Violating any is a blocking review comment.

### 1. Code exploration - CodeGraph MANDATORY

- Before reading source files to answer "how does X work / where is X / what calls Y", query CodeGraph first. See the CodeGraph block above for commands.
- After any source change in a commit, run `codegraph sync .` in the SAME session before committing so the index reflects the change. Recommended, not enforced: the index lives in `.codegraph/` (gitignored, never committed) and CI has no codegraph install, so the push gate cannot check it.
- Treat codegraph-returned source as already-Read; do NOT re-open those files in the same session.

### 2. Tool routing - context-mode MANDATORY

- See the context-mode routing block at the top. File edits, large grep, web fetch, data analysis all go through ctx_* first. `curl`/`wget`/inline HTTP are forbidden; use `ctx_fetch_and_index`. Shell is OK for bounded mutating commands (git, mkdir, cargo build, pnpm scripts).

### 3. i18n - decoupled, full-key coverage

- All user-visible strings in `src/` MUST come from the i18n catalog (`src/locales/<locale>/*.json`) via `react-i18next` `t()` / `Trans`. Never hard-code English (or any locale) in components.
- Base locales (18): `en`, `zh`, `ja`, `es`, `fr`, `de`, `ko`, `ru`, `pt`, `ar`, `hi`, `id`, `it`, `nl`, `pl`, `th`, `tr`, `vi`. Adding a string means adding the key to ALL base locales in the same commit. The `Locale` union in `src/store/appStore.ts` and the `ALL` list in `scripts/i18n-check.cjs` MUST stay in lockstep with `src/locales/` directories.
- `pnpm i18n:scan` runs `i18next-parser` (config `i18next-parser.config.js`) to extract t()/Trans keys into `src/locales/`; `pnpm i18n:check` fails the build if any base locale is missing a key (or has an extra one) vs the canonical `en` catalog. i18n.ts lazy-loads each (locale, namespace) chunk via `i18next-resources-to-backend` so Vite code-splits one chunk per locale.
- Locale files are the single source of truth for UI text; no inline substitutions of translated strings.
- The bilingual README pair (`README.md` / `README_CN.md`) is itself gated: `scripts/readme-lang-check.cjs` (mounted in `verify-build.sh`) locks the CN mirror to the EN-final section skeleton via an explicit EN<->CN heading mapping (same order, no extras on either side), requires the `GPL-3.0-or-later` License anchor in BOTH files plus working language cross-links, and bans the D-05 drift tokens (mihomo / axum / the unpublished `npm install -g @egressapikey/server`) from both homepages. Editing either homepage means updating both sides and the script mapping table in the same commit.

### 4. Tests - mandatory per behavior

- Rust: every public function in `crates/resin-core/` has a unit test in the same file (`#[cfg(test)] mod tests`) or an integration test under `crates/resin-core/tests/`. New behavior without a test is blocked.
- Frontend: Zustand stores and pure reducers have vitest unit tests under `src/**/*.test.ts(x)`. Component interactions have playwright e2e under `e2e/`.
- Evaluator for the whole repo: `bash scripts/verify-build.sh` - runs cargo build, cargo test, pnpm build, pnpm test, the six guard scripts and the mode-a contract gate (ADR-0068 D4); exits non-zero if any fail. CI calls this; so should pre-push hooks.
- Frontend vitest isolation guard (architecture-recovery 18): `node scripts/vitest-isolation-guard.cjs` (also wired into verify-build.sh) fails the build when a view test clicks a collapsible sub-header directly instead of the settled-state helpers (`clickSubHeaderExpand`/`clickSubHeaderCollapse`) - a raw click can land before the default-collapse seeding effect and invert the toggle (full-suite-only flake).

### 5. CI/CD - multi-platform packaging to release/

> **CI-only close-loop (2026-09-19 arbitration of the former P23 hard close-loop - ADR-0072, round11 D-006#1): every source commit MUST yield CI-built delivery evidence.** Local builds are FORBIDDEN (the 2026-09-04 CI-only build policy stands: no local compile/test/packaging). After ANY edit to `src/`, `src-tauri/`, `crates/`, `scripts/`, `tauri.conf.json`, or locale catalogs, delivery evidence = a CI run covering the change - the verify job (verify-build.sh) at minimum, plus a release-matrix dispatch when a testable exe is needed. The chunk-hash freshness check (grep the latest Vite chunk hash inside the staged exe bytes) and the `MainWindowTitle` smoke check apply to the CI-built artifact: `MainWindowTitle` alone proves the shell boots, not that the webview reflects your edits - a stale `dist/` embeds an old frontend and your UI fixes stay invisible (this exact bug bit P22).
>
> **Forbidden shortcut (now a hard pipeline rule)**: `cargo build --release` alone does NOT invoke `beforeBuildCommand` (only `tauri build` does) - it reuses the on-disk `dist/` and ships a stale-bundle binary. Any pipeline producing a deliverable exe MUST run `tsc -b && vite build` before cargo; `scripts/build-all.sh`/`.ps1` embody that order and fail fast with "STALE BUNDLE" when the staged exe lacks the latest chunk hash (T10-audit guard). Under CI-only these scripts are the canonical pipeline reference - read them, do not run them locally.

- GitHub Actions matrix builds five artifact groups into `release/`:
  1. windows-gui - Tauri MSI + NSIS `-setup.exe` installer (x86_64 msvc)
  2. linux-gui - Tauri `.deb` (Debian) + `.AppImage` (x86_64)
  3. macos-gui - Tauri `.dmg` arm64 AND x86_64 (split matrix jobs)
  4. gui-portable - one Tauri `--no-bundle` GUI executable per OS (no installer) for the convenient drop-and-run variant alongside the installer
  5. Backend-only headless target - `cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless` per OS, self-contained tar.gz per platform named `release/<os>-backend/` (headless exe + dist/ + resin sidecar) — see ADR-0043
- Pipeline reference: `scripts/build-all.sh`/`.ps1` encode the same pipeline order as the CI release jobs (`tsc -b && vite build` -> cargo -> stage, with the chunk-hash guard). Running them locally is FORBIDDEN by the CI-only build policy (ADR-0072); read them to understand the pipeline, do not execute them.
- iOS GUI note: iPadOS cannot run a Tauri desktop shell; the Apple-silicon desktop sibling is the macOS `.dmg`. Documented in CI and README. Do not promise an iOS iPad build.
- Artifacts are produced by `tauri-action` (installer + portable) and the backend matrix job; `release/` is gitignored except for tagged release assets uploaded to the GitHub Release.
- Debug-vs-release caveat (verified P2): `cargo build` (debug) emits a binary whose `tauri::generate_context!()` honours `devUrl` (`http://localhost:1420`) when `debug_assertions` is on - the webview tries to load the Vite dev server and shows "ERR_CONNECTION_REFUSED" / "localhost 拒绝连接" if `pnpm dev` is not running. Double-clicking the debug exe with no dev server looks like a GUI that flashes and dies. The **release** binary (`cargo build --release` or `tauri build --no-bundle`) disables `debug_assertions`, so `generate_context!` falls back to `frontendDist: ../dist` and embeds the built assets - it runs standalone with no dev server. The user-facing drop-and-run binary MUST be the release portable (`release/<os>-gui/<os>-portable-gui`), never the debug exe. Verified P2: release exe (7.9MB) boots, `MainWindowTitle = "EgressAPIKEY"`, stays alive 14s+, stdout `tracing initialized`, no stderr/panic.
- **MinGW resource linker fix (windows-gnu target, verified P19)**:    auri-winres (build dependency of     auri-build) finds the Windows SDK c.exe at C:\Program Files (x86)\Windows Kits\10\bin\...\x64\rc.exe and compiles esource.rc into MSVC .lib format that MinGW gcc.exe CANNOT link - error esource.lib: file not recognized: file format not recognized. Fix: after cargo build fails the link step (the resource.rc is already written), run windres --input <build-script-out>/resource.rc --output <build-script-out>/resource.lib --output-format coff to overwrite the MSVC .lib with a COFF object, then re-run cargo build --release (the linker will re-link with the fixed object). The previous successful builds produced libresource.a (the windres output) - the SDK rc.exe was likely not in PATH then. This is environmental, not a code bug.

- **No local release rebuilds (CI-only, ADR-0072)**: the former P23 local two-step guidance is retired - never build release artifacts locally. Release artifacts come from the CI release matrix (`workflow_dispatch`); their freshness is guaranteed by pipeline order (frontend build before cargo) plus the build-all hash guard.
- When handing a build to the user for testing, launch the CI-built portable exe once under `Start-Process` and confirm: `MainWindowHandle != 0`, `MainWindowTitle == "EgressAPIKEY"`, `WorkingSet ~20-40MB`, stdout has `tracing initialized` and no panic line. If `MainWindowTitle` is the exe path or stderr has `ERR_CONNECTION_REFUSED`, it is a debug build mistakenly handed out - re-dispatch the CI release build.

### 6. Git hygiene - push after every change

- Commit convention: `<Phase>: <area> - <summary>` e.g. `P1: core - lane hash + SSE lease`.
- Every phase/feature commit MUST be pushed (`git push`) so the project is always traceable. Do not accumulate local-only work across phases.
- `.gitattributes` LF policy is authoritative; `git diff --check` must be clean before each commit. Never commit with CRLF outside the allow set (`.bat`, `.ps1`, `.cmd`).
- `git config core.autocrlf false` at repo level (set at init). Do not re-enable autocrlf.
- **Session-artifact hygiene (untrack recipe)**: session-local dirs (`.zcode/`, `.codex-tmp/`, `.omx/`, `context-mode/`, `.scratch/`) must never enter git tracking; each gets a single literal-line `.gitignore` entry (no wildcard broadening). A tracked leftover is untracked with `git rm -r --cached <path>` — the one permitted index operation outside but's write set; disk files are preserved (never delete). Re-verify with `git status` and `git ls-files <path>`: both must come back empty for the path.
- **Delivery evidence is part of the commit (CI-only, ADR-0072; supersedes the former P23 clause)**: a source commit is finished when CI proves it - the verify job runs verify-build.sh on every push, and a release-matrix dispatch produces the user-facing portable exe (latest Vite chunk hash embedded) when the change needs a testable binary. `target/` and `release/` are gitignored - what you commit is the SOURCE; CI-built artifacts are the deliverable and live outside git. Never hand the user a locally-built exe; do not claim a UI change is testable until its CI-built artifact exists.

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
- `subscription_add`'s optional `pipeline` parameter (Round 7 ticket 01) accepts exactly `"establish"` or `null`; any other value is rejected at the IPC layer (Rust) and at the TS wrapper before invoking. The parameter only opts the ALREADY-validated subscription import into the backend establish cascade — it never relaxes the name/url/update_interval validation above, and the pipeline queue is bounded (64 entries) with per-entry attempt caps (5) so repeated enqueues cannot grow memory or hammer Resin.
- `subscription_add`'s optional `default_port` parameter (Round 7 ticket 03, D-C1.3) is a plain `u16` validated at both boundaries against `MIN_USER_PORT` (privileged ports rejected; `u16` needs no upper-bound check). Providing it skips the suggest probe in the cascade's optional default-port tail (`resin_core::subscription_pipeline::ensure_default_port`, invoked by the shell right after a GREEN drain pass — the 5-step `run_pipeline`/report shape is untouched). Every conflict (a whitebox row already bound to the platform, the port held by another platform, a foreign Resin listener, a 409 create) is a WARNING + skip, never an overwrite and never an error; unbind goes through the existing `port_remove` path — no new IPC command was added (manifest count unchanged at 78). `port_suggest` now delegates to `resin_core::subscription_pipeline::suggest_free_entry_port` so the GUI and the pipeline share one ADR-0031 suggestion implementation.
- A failed establish cascade compensates only what THAT pass created (Round 7 ticket 04, D-C1.4): `resin_core::subscription_pipeline::compensate_failed_cascade` deletes the Resin-side platform row only when this pass created it AND `apply_strategy`'s failure-owner probe attributes the failure to it; the subscription (user data) and the strategy whitebox (L2) are NEVER touched — whitebox deletion belongs to another path. The ordered sub/plat/port/apply marking persists as schema-locked `last_cascade_error {stage, reason, rollback_actions[]}` on the subscription's status row via `StrategyService::record_cascade_failure` (status subresource — generation does NOT move), cleared by the next Converged write; `validate()` caps the record (8 actions × 256 chars, NUL rejected). No new IPC command (manifest stays 78); the UI surface is read-only projection (snapshot → TS sanitizer → SubscriptionsView hover/expand).
- subscription_remove now runs a best-effort reverse tail (architecture-recovery ticket 05, A-008 / Round 9 D-007 option B): after the Resin DELETE succeeds, `resin_core::subscription_pipeline::remove_default_port_if_orphaned` releases every whitebox entry-port row bound to the deleted subscription's platform through the same three-layer order as `port_remove` (L2 whitebox retain→apply first, then the L3 Resin endpoint delete — Mode B only; Mode A's shell listener comes down with the whitebox apply and Resin is never contacted). The read-only id="default" endpoint is never a deletion candidate; a failed release is a `tracing::warn!` (the leftover surfaces as drift), never an IPC error — the removal itself succeeded. The (e)→(f) create-then-write window in `ensure_default_port` is likewise closed: a whitebox-write failure compensates by DELETE-ing exactly the endpoint THIS step created (fresh live read; idempotent no-op when already gone), and step (a)'s same-platform binding check already provides the idempotence the ticket demands. No new IPC command (manifest stays 79).

### 7.6. Sidecar SSRF guard + IPC surface discipline (Re8 audit)

- mihomo status (ADR-0050): the `crates/resin-core/src/mihomo.rs` module (MihomoController + its loopback SSRF guard, MihomoConfig, subscription compile helpers) was DELETED with the rest of the dead kernel face (zero external references; it was never instantiated). If mihomo REST control is ever reintroduced, the controller MUST re-assert the Re8 rule before any IPC wiring: `api_base` validated as loopback-only (`http://127.0.0.1` / `http://localhost` / `http://[::1]`, plus https variants) at construction, with reject + accept-variant unit tests, and no frontend-controlled `String` may ever construct it (config stays server-side trust).
- Headless control surface (architecture-recovery round 8, ticket 03 / A-007): the
  `egressapikey-headless` router MUST keep every route - including the static
  fallback - behind the `security_guard` layer in `src-tauri/src/headless_main.rs`,
  which enforces a shared `--auth-token` on `/api/v1/*` + `/metrics/*` and a
  `Host` / `Origin` allowlist on every request. The token is CSPRNG-generated
  (`getrandom`); time + PID entropy is forbidden. Never register a route above the
  `.layer(...)` call, and never add a route that bypasses the guard. Primitives +
  unit tests: `src-tauri/src/headless_security.rs`. Operator threat model:
  `docs/how-to/HEADLESS_DEPLOYMENT.md`.
- Frontend `invoke()` surface (authoritative manifest, architecture-recovery ticket 03): the command list lives in `docs/agents/ipc-manifest.md` (the `ipc-manifest` fence below is a pointer only). It is REGENERATED by `node scripts/ipc-manifest-check.cjs --write` and machine-checked on every build (`pnpm ipc:check`, `scripts/verify-build.sh`, CI); the check fails on command add/remove/rename, `generate_handler!` registry drift, manifest drift, or stale file attribution. Do not hand-edit: run the regenerator. Each TS wrapper in `src/lib/ipc.ts` validates input at the TS boundary (assertShortName/assertAuthority/assertIp, length caps, lane range `0..MAX_LANES=50`, latency cap, URL `http(s)://` prefix) BEFORE invoking, and treats the Rust response (`reason`, `lane`, `account`) as untrusted — never piped into another URL or command. The Rust side re-validates the same bounds in `commands/` domain modules. When wiring any new IPC command, the TS wrapper MUST follow this same validate-then-invoke contract.

**Echo command list (kept for IPC contract compatibility; the shell does not implement their semantics — ADR-0050 deleted the kernel face)**:

- `account_add` (src-tauri/src/commands/platform.rs:64): validates inputs (incl. the lane range) then returns `Ok(())` — Resin owns account semantics since ADR-0050. Emits `tracing::warn!(target: "ipc.account_deprecated")` per call.
- `account_bind_ip` (src-tauri/src/commands/platform.rs:85): validates inputs then returns `Ok(true)` — same echo semantics. Emits the same deprecation warn. TS wrapper `ipcAccountBindIp` carries the matching `@deprecated` + dev-only `console.warn`.

Removal condition: drop these from the manifest + registry only when Resin's account REST surface changes shape (T16 owns the account header-rules integration; do not delete in a docs round).

```ipc-manifest
# Moved to docs/agents/ipc-manifest.md - regenerate with
# node scripts/ipc-manifest-check.cjs --write. Commands: 85
```

- If mihomo REST control is ever reintroduced (ADR-0050 is the authoritative record of the `mihomo.rs` deletion): never expose `MihomoController`, `CoreConfig.mihomo_api`, or `CoreConfig.mihomo_secret` through a `#[tauri::command]` that takes a raw `String` and constructs the controller from it; `api_base` must pass the loopback-only validation described above (non-loopback targets are rejected); and config must come from `tauri-plugin-store` settings.json (server-side trust), not from the webview.
- If `MihomoController` is ever reintroduced, it must not be wired through a frontend-controlled constructor without revisiting this section; keeping it uninstantiated by the Tauri shell was the pre-deletion safety boundary and remains the default stance until such a reintroduction is explicitly designed.

### 8. Subagent policy for this repo

- This thread runs with subagents DISABLED (per user instruction). Do NOT spawn Codex native subagents or OMX team/worker lanes. Execute everything single-threaded in this agent.

### 9. Don't revert work you didn't make

- If uncommitted changes appear that this agent did not make, treat them as user/external and do not revert. Either ignore (unrelated) or build with them (affects the task).

### 10. Update this file in the same commit that changes the project

- AGENTS.md is the source of truth. When structure, conventions, module boundaries, tech stack, or release/push protocol changes, update this file in the SAME commit that introduces the change.

### 11. License layering & third-party provenance (ADR-0067)

- The repo declares GPL-3.0-or-later via the root `LICENSE` (official verbatim GNU text — never reword it) and the three fields `README.md` License section / root `Cargo.toml` `[workspace.package].license` / `package.json` `license`, kept equal by `scripts/license-field-check.cjs` (mounting into verify-build belongs to the CI-gate ticket; run `node scripts/license-field-check.cjs` before committing license-touching changes). Member crates inherit via `license.workspace = true`, never hardcode.
- `THIRD_PARTY.md` is the single registry for third-party components and records the two-layer value per component: declared license vs dependency-tree truth. Resin v1.2.0 = declared MIT + dependency-tree GPL-3.0-or-later via sing-box (`resin/go.mod` pins `github.com/sagernet/sing-box v1.12.21`; sing-box's upstream LICENSE is GPLv3-or-later).
- Mere-aggregation invariant (ADR-0067 D3): shell ↔ sidecar interact only over the loopback REST seam (`ResinClient`). No FFI, no in-process linking, no source embedding in either direction — breaking this voids the license layering and requires revisiting ADR-0067.
- On any `docs/RESIN_UPSTREAM_MANIFEST.yaml` version bump: re-verify the license facts against upstream originals at the exact new tag (go.mod + LICENSE URL quotes, never memory) and update `THIRD_PARTY.md` + the manifest two-layer `license` field in the same commit (ADR-0017 amendment).

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

### Community & contribution files

Moved to [docs/agents/community-files.md](docs/agents/community-files.md) (CONTRIBUTING / ISSUE_TEMPLATE / PR_TEMPLATE / SECURITY / CODE_OF_CONDUCT inventory).

### Architecture & Phase History (externalized)

> The following sections were moved to `docs/agents/` to keep this file under the 32 KiB `project_doc_max_bytes` limit.
> Content is unchanged (extraction only, progressive disclosure per writing-for-agents).

**Architecture implementation** (AGENTS.md former §11-17): [docs/agents/architecture-state.md](docs/agents/architecture-state.md)

- Runtime wiring state, Resin sidecar lifecycle (G1), Ghost safety net (G3), ResinClient (G2)
- IPC retarget to Resin sidecar, Resin → shell signal channel (G4 = IPC retarget + sidecar-status event subscription, no HTTP webhook), CI/CD release pipeline (G5)

**Phase history** (AGENTS.md former §18-66, P9-P26/T7-T22/R1-R2/C1-C2): [docs/agents/phase-history.md](docs/agents/phase-history.md)

- These are **process artifacts** (point-in-time phase completion records), not current truth.
- For current architecture overview: [docs/architecture/](docs/architecture/)
- For architectural decisions: [docs/adr/](docs/adr/)
- For what shipped (milestones): [CHANGELOG.md](CHANGELOG.md)

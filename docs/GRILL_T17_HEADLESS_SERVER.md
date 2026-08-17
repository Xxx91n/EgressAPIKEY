# GRILL T17 — Headless NPM Server Fix Branch

> Date: 2026-08-17
> Status: PLANNED (awaiting user execution signal)
> Branch: codex/rust-port (T17 headless fix branch, parallel with other fix branches)
> Related ADRs: [ADR-0043](adr/0043-headless-server-build-separation.md)
> Sources: atomcode research T17-Q1 (12 searches, 11 web_fetch, dual-binary architecture), T17-Q2 (20 sources, dual-runtime frontend adaptation), T17-Q3 (16 sources, build/packaging separation), T17-Q4 (18 sources, enterprise documentation structure)

## Context

The project ships a Tauri 2 desktop GUI (EgressAPIKEY.exe) and a headless HTTP server binary (egressapikey-headless.exe) from the same codebase. The headless binary serves the prebuilt React dist/ via axum ServeDir and reverse-proxies /api/v1/* and /metrics/* to the Resin Go sidecar with admin bearer token injection. This is the "npm install" path for users who want localhost access without the Tauri desktop shell.

### Current state (verified)

- `src-tauri/src/bin/headless.rs` — ~230 lines, fully functional: axum build_router + ServeDir + proxy_to_resin + boot_resin_standalone + two-phase shutdown + tracing daily rotation + dirs crate OS-standard paths
- `Cargo.toml` [[bin]] name="egressapikey-headless" path="src/bin/headless.rs" — **NO required-features gating** (Tauri bundler auto-discovery hazard, rssh issue + tauri #15325)
- `release/windows-gui/npm-server/egressapikey-headless.exe` — headless artifact nested inside GUI channel directory (should be separate release/windows-backend/)
- `release/windows-backend/` — only has egressapikey-headless.d (1591 bytes debug symbol), NO release exe staged
- `build-all.ps1` L33-35 — headless build failure marked as "non-fatal, GUI-only"
- `npm/launcher.cjs` — ~80 lines, CommonJS, EGRESSAPIKEY_HEADLESS_BIN + EGRESSAPIKEY_DIST path resolution (codeg CODEG_STATIC_DIR/CODEG_MCP_BIN isomorphic)
- `npm/package.json` — @egressapikey/server, bin: egressapikey-server, engines node>=18
- AGENTS.md S5 — stale: "Backend-only headless target = cargo build --release -p resin-core" (actual headless is src-tauri crate bin)
- CI ci.yml backend job — builds resin-core per-OS, NOT egressapikey-headless binary
- No headless/deployment/npm/server/docker documentation in docs/

## Grill decisions (4 questions, all resolved)

| Q | Decision | Industry template | Key evidence |
|---|---|---|---|
| Q1 | **B** — verify end-to-end first, store mental model to wiki, then add docs + VPS closure | headless.rs already has VPS-ready axum + Go sidecar + ServeDir + reverse proxy | atomcode T17-Q1: codeg-server, rssh-server, emem systemd unit |
| Q2 | **A** — frontend ipc.ts dual-mode adapter (isTauri() + fetch fallback) | codeg port-adapter + ChatGPT-Next-Web TransformStream for SSE | atomcode T17-Q2: 20 sources, Wails v3 maintainer confirms WebView IPC is security-disabled for remote URLs |
| Q3 | **A** — full separation (source move + required-features + release dir migrate + CI split job) | codeg build-tauri + build-server separate jobs; rssh source relocation | atomcode T17-Q3: 16 sources, tauri issue #15325 stage-2 disk scan bug, PR #15427 fix |
| Q4 | **A** — add maintenance docs (Diataxis 4 quadrants) + sync all grill docs/README/comments | Diataxis framework + codeg docs + mihomo wiki + code-server npm.md | atomcode T17-Q4: 18 sources |

## Execution plan (3 phases, 10 steps)

### Phase 1: End-to-end verification (Q1=B)

| Step | Action | Acceptance criteria |
|---|---|---|
| P1 | Launch egressapikey-headless.exe + dist/ + resin sidecar. Verify (a) browser loads React SPA at http://127.0.0.1:14200, (b) /api/v1/* proxies to Resin sidecar returns real data, (c) Ctrl+C two-phase shutdown kills resin child | All three verified with evidence. Mental model stored to wiki. |

### Phase 2: Build separation (Q3=A)

| Step | Action | Acceptance criteria |
|---|---|---|
| P2 | Move headless.rs from src-tauri/src/bin/headless.rs to src-tauri/src/headless_main.rs. Cargo.toml [[bin]] path="src/headless_main.rs" + required-features=["headless"]. Add [features] headless = [] | cargo build --bin egressapikey-headless --features headless succeeds. cargo build (no features) does NOT compile headless. |
| P3 | Migrate release artifact from release/windows-gui/npm-server/ to release/windows-backend/. Self-contained: headless exe + dist/ + resin.exe sidecar | release/windows-backend/ contains egressapikey-headless.exe + dist/ + resin.exe |
| P4 | build-all.ps1 + build-all.sh: headless build failure changes from non-fatal warning to fatal exit 1 | Script exits non-zero if headless build fails |
| P5 | CI ci.yml: backend job changes from resin-core per-OS binary to egressapikey-headless binary (cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless). Independent rust-cache key. Artifact: <os>-backend.tar.gz | CI backend job produces egressapikey-headless binary per platform |

### Phase 3: Frontend adaptation + documentation (Q2=A, Q4=A)

| Step | Action | Acceptance criteria |
|---|---|---|
| P6 | src/lib/ipc.ts: each wrapper detects isTauri(). Tauri: invoke(). Browser: fetch("/api/v1/...") direct to headless axum reverse proxy. SSE: TransformStream reassembly (ChatGPT-Next-Web pattern) | vitest: isTauri()=true -> invoke, isTauri()=false -> fetch. All wrappers dual-mode. |
| P7 | New docs: npm/README.md (rewrite per code-server template: install->URL->credentials 3-line closure + CLI flags table + env overrides + security model). docs/HEADLESS_DEPLOYMENT.md (systemd unit template + env table + port config + dist path + sidecar placement, per mihomo/vaultwarden). docs/HEADLESS_RUNBOOK.md (log location + /healthz + graceful shutdown + restart policy + troubleshooting) | 3 docs files created with industry-standard content per atomcode Q4 research |
| P8 | AGENTS.md sync: S5 CI/CD backend job description, S17 G5 backend matrix, all headless references aligned to new path release/<os>-backend/ and egressapikey-headless bin name | grep headless/backend/npm-server in AGENTS.md shows no stale references |
| P9 | README.md + README_CN.md sync: add headless server install/usage section (dual-target product explanation + npm install one-liner + CLI flags link) | Both READMEs have headless section |
| P10 | Test closed-loop: vitest verifies ipc.ts dual-mode (isTauri branch + fetch branch). cargo build --bin egressapikey-headless --features headless verifies required-features gating. launcher.cjs --dry-run verifies path resolution | 3 test categories green. No false positives. |

## Industry templates referenced (from atomcode research)

- **codeg** (docs.codeg.app): one Cargo workspace, shared codeg_lib, 3 binaries (codeg/codeg-server/codeg-mcp), CI build-tauri + build-server separate jobs, CODEG_STATIC_DIR/CODEG_MCP_BIN env pattern
- **rssh** (github.com/shihuili1218/rssh): one crate, 3 binaries (rssh/rssh-cli/rssh-server), required-features gating + source relocation (avoid src/bin/ auto-discovery), include_dir embedded dist for headless
- **clash-verge-rev**: Tauri GUI + Go sidecar (mihomo), prebuild.mjs sidecar download, externalBin tauri.conf.json
- **ChatGPT-Next-Web**: stream_fetch command + TransformStream reassembly — SSE/fetchEventSource zero-change dual-runtime
- **code-server** (npm.md): OS dependency table + install->URL->credentials 3-line closure + PATH troubleshooting
- **mihomo** (wiki.metacubex.one): systemd unit template (Restart=always, LimitNOFILE, ExecReload=kill -HUP, journalctl)
- **Diataxis** (diataxis.fr): tutorials/how-to/reference/explanation 4 quadrants — the industry-standard doc structure

---

## Audit follow-up (T17-audit-fix)

The original T17 marked P6 and P10 as checked but a deeper code-level diff audit (against the GRILL plan acceptance criteria no false positives) found three real defects that the original 4 vitest assertions did NOT cover:

1. **platform_remove / subscription_remove / platform_update fetch path broken (high severity).** The SPA's invokeHttp fallback sent DELETE /api/v1/platforms with body {name:"X"} and PATCH with a full camelCase body. The headless axum reverse-proxy (proxy_to_resin) was a pure forwarder — it forwarded the body-bearing collection DELETE verbatim to Resin, which expects DELETE /api/v1/platforms/{id} (path-param, no body). Resin returned 404/405 and the SPA's mutate calls silently failed in headless mode. The Tauri mode worked because the Rust IPC commands (platform_remove, platform_update, subscription_remove) do a list+match name-to-id translation in Rust before forwarding — but the headless mode bypassed those commands entirely.
2. **patch-body key shape not equivalent (medium).** The SPA TS wrapper ipcPlatformUpdate sends camelCase IPC keys (allocationPolicy, regexFilters, ...) and the Tauri Rust command rewrites them to Resin's snake_case form. The headless path forwarded camelCase — Resin would have rejected the PATCH even with a working id.
3. **Vitest P10 false positive.** The 4 T17 dual-mode assertions only exercised platform_add (POST collection) + platform_list (GET collection) — the two trivially-correct routes. Zero coverage for the three broken DELETE/PATCH routes. The no-false-positives P10 criterion was a false claim on the mutation surface that actually mattered.

**Fix (industrial pattern B — BFF/agent-side translation, per atomcode research).** The headless axum proxy now does the same list+match name-to-id resolution the Tauri Rust commands do, BEFORE forwarding, for the three known translation routes. The SPA frontend contract stays name-based in both modes; resolve+mutate are atomic in one process (no client-side TOCTOU). Industry prior art surfaced by atomcode: Kong request-transformer (transform before upstream), AWS API Gateway mapping templates, LiteLLM (admin uses UUID, alias resolved server-side), Vaultwarden (read by name, mutate by UUID). Pattern C (name-based upstream) was rejected because Resin v1.2.0 has no name endpoints and we don't fork the upstream Go binary.

**Changes.**
- src-tauri/src/headless_main.rs: proxy_to_resin now aggregates the body (64 MiB cap) and a new translate_request dispatcher rewrites (method, path, body) into the path-param form when needed. New pure helpers items_arr, id_for_name, url_encode_segment, rewrite_patch_body_snake_case — extracted as pure functions so unit tests can lock the contract without spinning up reqwest.
- src/lib/ipc.test.ts: 3 new T17-audit vitest assertions pin the SPA-side contract (frontend continues to send the business name in the JSON body to the collection URL; the server-side BFF resolves name->id after this fetch).
- src-tauri/src/headless_main.rs cfg(test) module bff_translate_tests: 12 cargo unit tests covering items-wrapper vs bare-array resolution, empty-id rejection, URL encode behaviour, PATCH name stripping + camelCase to snake_case translation, null-field drop, enum/length validation, and empty/non-object body rejection.

**Verification.**
- cargo test -p egressapikey-app --bin egressapikey-headless --features headless -> 12 passed.
- npx vitest run -> 255 passed (was 252; +3 audit assertions).
- npx tsc --noEmit clean.
- npm run i18n:check -> 363 keys / 18 locales match.
- cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless -> 4.7MB exe staged at release/windows-backend/egressapikey-headless.exe.
- Smoke: launched the binary, process ALIVE, GET / -> 200, GET /api/v1/platforms (proxied to Resin sidecar) -> 200.

**Audit trail preservation.** The original Phase 3 table (P6/P10 marked as done) is left intact above for the per-phase status snapshot; this section is the authoritative current truth. Following AGENTS.md section 6/10 directive: do NOT rewrite historical status rows into the new truth — leave them and add the follow-up.

**Ponytail.** No new crates. The BFF layer is ~140 LoC added to headless_main.rs (a single match on three routes + a pure rewriter), and the existing axum/reqwest/serde_json/bytes deps already present in the headless Cargo.toml cover it. The frontend (src/lib/ipc.ts invokeHttp, CMD_TO_HTTP) is unchanged — the translation lives where the Rust IPC commands already live.

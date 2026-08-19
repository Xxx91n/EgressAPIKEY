# GRILL T22 — Headless Server Rendering Fix + WebView2 Architecture Alignment Audit

> Status: PLANNED (post-grill, awaiting user instruction to build)
> Date: 2026-08-19
> Branch: codex/rust-port (fix branch, parallel)
> Predecessor: T21 (headless/WebView2 architecture audit — closed)

## Problem

The headless npm server (egressapikey-headless.exe) starts, binds port 14200,
opens the browser to http://127.0.0.1:14200/, and the user sees a flash of
the SideRail nav + "Loading..." text before the entire React tree unmounts,
leaving a blank white page.

## Root Cause (verified via code + atomcode research)

src/lib/ipc.ts ipcWatchPortHealth() unconditionally constructs
new Channel<PortHealthSnapshot>() from @tauri-apps/api/core and calls
_invoke("watch_port_health", ...) without the isTauri() guard. In a
plain browser (headless mode), Channel constructor touches
window.__TAURI_INTERNALS__ which does not exist, synchronously throws
inside the React useEffect in TopologyView.tsx L916, React unmounts
the entire component tree, white screen.

Secondary issues:
1. headless_main.rs ServeDir has no not_found_service, client-side
   routes (refresh on /platforms) return 404, blank page on deep-link/refresh.
2. No React Error Boundary, any uncaught throw in any view unmounts
   the entire app, no recovery, no diagnostics.

## Grill Decisions

| Q | Decision | Rationale |
|---|----------|-----------|
| Q1 | A | Root-path flash to white screen: React mount succeeds briefly then crashes |
| Q2 | A | headless exe starts, browser auto-opens, SideRail + Loading flash then white |
| Q3 | A | 3-layer fix: isTauri guard + ServeDir SPA fallback + Error Boundary |
| Q4 | C | agent-browser open localhost:14200, read DOM (no screenshots) |
| Q5 | A | Hand-made bash script in ctx_batch_execute, ~20 lines |
| Q6 | B | Full architecture alignment audit after white-screen fix |

## Execution Plan

### Phase 1 — White-Screen Fix (3 layers)

| Step | File | Change |
|------|------|--------|
| L1 | src/lib/ipc.ts | ipcWatchPortHealth: add isTauri() guard before new Channel() |
| L2 | src-tauri/src/headless_main.rs | ServeDir not_found_service(ServeFile::new(dist/index.html)) + import ServeFile |
| L3 | src/App.tsx | ErrorBoundary class component, wrap main render |

### Phase 2 — Build + Smoke

| Step | Command | Gate |
|------|---------|------|
| 2a | pnpm build (tsc + vite) | tsc green, new chunk hash |
| 2b | cargo build --release --features headless | green, ~4.6MB |
| 2c | cargo build --release --features custom-protocol | green, ~14MB |
| 2d | Stage to release/windows-backend/ + release/windows-gui/ | exe + dist + resin.exe |
| 2e | Chunk hash verify in exe bytes | hash found |
| 2f | Smoke: headless exe starts, port 14200 listening | process alive |

### Phase 3 — agent-browser Closed-Loop Verification

| Step | Script | Assertion |
|------|--------|-----------|
| 3a | npx agent-browser open http://127.0.0.1:14200/ | page opens |
| 3b | npx agent-browser snapshot -i | SideRail nav buttons present (7 buttons) |
| 3c | npx agent-browser read | rendered DOM includes app title, #root non-empty |
| 3d | npx agent-browser snapshot -i after clicking each nav | each view renders visible content |

### Phase 4 — Architecture Alignment Audit (headless vs webview2)

| Step | Scope | Method |
|------|-------|--------|
| 4a | IPC command coverage | Diff CMD_TO_HTTP map (20 entries) vs all invoke() call sites. List unmapped. |
| 4b | Per-view headless compat | For each of 7 views, trace all Tauri API calls. Confirm isTauri guard or try/catch. |
| 4c | Settings persistence | settings.ts LazyStore calls: confirm fallback when not in Tauri. |
| 4d | Audit report + fix | Write findings, fix uncovered issues, delete audit doc after fix. |

### Phase 5 — Commit + Push + Docs

| Step | Action |
|------|--------|
| 5a | Run vitest + cargo test — green |
| 5b | Update AGENTS.md with T22 section |
| 5c | Commit: T22: headless white-screen fix |
| 5d | Push |

## Industry References

- tower-http 0.7.0 docs.rs: ServeDir::not_found_service + ServeFile for SPA fallback
- axum official static-file-server example: SetStatus(ServeFile, NOT_FOUND) pattern
- axum Discussion #2486: fallback(ServeFile(index.html)) community canonical answer
- React docs: Error Boundaries (getDerivedStateFromError + componentDidCatch)
- atomcode research: 14 source articles (Tavily + AnySearch), confirmed root cause

## Acceptance Criteria

1. egressapikey-headless.exe starts, browser opens localhost:14200, no white screen
2. SideRail visible, Topology canvas renders, all 7 tabs switchable with content
3. Page refresh on any client route returns index.html (not 404)
4. agent-browser DOM read confirms: #root non-empty, nav buttons present, app title visible
5. Error Boundary catches unexpected throws, renders fallback with reload, not white screen
6. Architecture audit: all IPC commands either mapped to HTTP route or isTauri-guarded
7. vitest + cargo test green; pnpm build + cargo build green; chunk hash verified
# ADR-0049: Headless Server White-Screen Root Cause + SPA Fallback + Error Boundary

> Status: ACCEPTED
> Date: 2026-08-19
> Supersedes: none
> Related: ADR-0043 (headless build separation), GRILL T22

## Context

The headless npm server serves the same Vite-built React SPA as the Tauri
WebView2 desktop shell, but without Tauri IPC injection. The SPA dual-mode
isTauri() guard routes IPC calls to HTTP fetch in headless mode. However,
one streaming IPC wrapper (ipcWatchPortHealth) bypassed the guard and
directly constructed a Channel object from @tauri-apps/api/core, which
throws synchronously in a plain browser because window.__TAURI_INTERNALS__
does not exist. The throw happens inside a React useEffect in TopologyView
(the default view), causing React to unmount the entire component tree.

Secondary issue: ServeDir in headless_main.rs lacked SPA fallback
(not_found_service), so client-side routes returned 404 on refresh.

## Decision

Three-layer fix:

1. isTauri guard on streaming IPC (ipcWatchPortHealth): return no-op
   unsubscribe when !isTauri(). Same pattern as the existing invoke() wrapper.

2. ServeDir SPA fallback (headless_main.rs): add
   ServeDir::not_found_service(ServeFile::new(dist.join("index.html"))).
   Industry canonical pattern (axum Discussion #2486, tower-http docs.rs).

3. React Error Boundary (App.tsx): class component with
   getDerivedStateFromError + componentDidCatch + fallback UI with reload.
   Prevents future uncaught throws from silently unmounting the app.

## Alternatives Considered

- Only fix layer 1: would fix immediate white-screen but no defense against
  future issues. No SPA fallback means refresh still 404s.
- Playwright spec to reproduce then fix: adds framework overhead, slower.
  Root cause was already clear from code analysis + atomcode research.
- Mock __TAURI_INTERNALS__ globally: fragile, does not address the real
  issue that streaming IPC has no headless equivalent.

## Consequences

- ipcWatchPortHealth in headless mode is a no-op (no port health streaming).
  Acceptable: port health is a desktop-only real-time feature.
- SPA fallback returns 200 for unmatched paths (ServeFile default status).
  The SPA router handles client routes; 404 semantics not needed.
- Error Boundary adds ~25 lines to App.tsx: standard React pattern.
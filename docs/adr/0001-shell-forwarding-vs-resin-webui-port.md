# ADR-0001: Keep shell-forwarding architecture, deepen IPC coverage

Date: 2026-08-02
Status: PROPOSED (pending user confirmation in grill-with-docs session)
Decision Type: Architectural

## Context

The project forks Resin (github.com/Resinat/Resin) as a Go sidecar inside a
Tauri 2 + React 19 desktop shell. The original G4 spec called for physically
copying Resin/webui/ React components into src/resin-views/. The P16 Ponytail
decision skipped that physical port and instead built a self-contained 5-tab
desktop shell with ResinClient + IPC forwarding to the live sidecar.

## Decision

Keep the shell-forwarding architecture (self-built 5-tab + ResinClient IPC).
Do NOT physically port Resin/webui/ React components.

Deepen IPC coverage for 4 specific gaps identified in the Q1 gap audit:
1. Subscription enable/disable + interval + ephemeral toggle + LastError display
2. Reverse-proxy header-rule editor (empty_account_behavior + fixed_account_header)
3. Structured request-log viewer tab (per platform/account/target filtering)
4. Passive circuit-breaker toggle exposed via platform PATCH

## Rationale

Physical port conflicts:
- Resin's webui has its own React/Vite/TS stack, state management, routing,
  and i18n directory. Embedding it as sub-views introduces two parallel React
  apps, two routing models, two i18n catalogs. An iframe wrapper breaks IPC.
- The desktop shell already owns: Tauri webview, system tray, theme switching,
  i18n (18 locales via react-i18next), topology canvas (ReactFlow), Ghost safety
  net. A full React app piggyback would either bypass all of those or need
  rewriting each Resin component to fit.

The architectural contract is sound: shell sees live Resin data via
ResinClient REST forwarding. The gap is IPC coverage depth, not structure.

## Consequences

- 39 Resin admin endpoints remain the only way the webview reaches Resin.
  The admin token stays Rust-side. No new domain crossing.
- 4 new IPC commands needed (subscription_update, request_log_query,
  reverse_proxy_config_get/set, platform circuit-breaker toggle).
- Each new view tab remains a native desktop view, not a foreign React app.

## UPDATE (2026-08-02, after user Q1 reply)

User answered Q1 = (C): "internally audit and decide between (a) bringing the
Resin mature webui IN and merging our shell into it, vs (b) keeping our 5-tab
shell and only back-porting the missing views from the Resin webui." User
added a pivotal requirement:

> 我以后可能不只是考虑本机的 Tauri 壳子，因为我以后是想要把这个
> 搬进一些 VPS 里面的，这种一般都需要稳定的后端和 localhost 的前端

This bifurcates the deployment surface:
1. Desktop shell path: Tauri webview embeds the React app built to ../dist (no
   separate frontend server, no localhost:80 port).
2. VPS path: needs a standalone backend (the Resin Go binary itself is the
   backend) + a localhost-served frontend. The current Resin Go binary already
   serves its own webui at http://127.0.0.1:2260 (per README) inside the same
   port that carries the control-plane API + the proxy — i.e. for VPS use you
   can just deploy the Resin Go binary alone and point your browser at its
   webui. The Tauri shell is the DESKTOP convenience layer, not the only
   frontend.

### Internal deliberation

(a) Merge Resin webui into the desktop shell — physically port
    vendor/resin/webui React components into src/, rewrite their API client to
    use Tauri IPC instead of fetch(), rewire their state mgmt, port their
    routing, port their i18n strings, and abandon our existing Topology canvas
    / ProcessRoute / Settings / tray / theme work. Pros: we get Resin's
    Platform/Subscription/Lease/Metrics/Log views verbatim. Cons:
    - Two parallel React stacks must be merged; their i18n catalog and ours
      must be reconciled across 18 locales × ~150 keys, per-key.
    - Their API client hits fetch(:2260) — we MUST rewrite every call site to
      invoke() or a flag-day dual-mode shim; the desktop path needs IPC for
      the admin token to stay Rust-side (AGENTS §7.6).
    - The VPS path doesn't need any of this — Resin's own webui is already
      served standalone on the sidecar port. Merging webui into src/ helps
      only the desktop path, and even there the cost is a large rewrite that
      loses our Topology / ProcessRoute / tray work.
    - Loses our USPs: topology canvas, ProcessRoute, Ghost safety net,
      backup-WebDAV, single-instance, tray i18n, locale memory — Resin's own
      webui has none of these.

(b) Keep our 5-tab shell + selectively back-port missing Resin webui views.
    Pros:
    - Desktop path keeps its UX USPs.
    - VPS path: just ship the Resin Go binary standalone (it serves its own
      webui at :2260) + our optional Rust shell as a desktop add-on. No merge
      cost, no dual-mode shim, no i18n reconciliation across two catalogs.
    - Our IPC forwarder is the SINGLE source of truth for what the desktop
      user can do; we choose which Resin features to surface.
    Cons:
    - We must hand-write the vewing ports one by one (subscription_update,
      request_log_query, node-pool depth, reverse-proxy header rule editor,
      circuit-breaker toggle). Each is ~one new ResinClient method + one new
      IPC command + one view extension, matching our existing 36-command
      shape. Not zero work, but bounded and additive.

### Decision

Path (b): keep our 5-tab shell; selectively back-port the missing views via
new IPC commands. The VPS path delegates the frontend to the Resin Go
binary's own bundled webui (no extra work — Resin already serves it).

This is the cleanest split:
- Desktop: one React app, one IPC surface, admin token never in webview.
- VPS: deploy the Resin Go binary alone (or via our release pipeline as a
  standalone tarball). The Resin webui on :2260 IS the VPS frontend.

### What this ADR now blocks

- Physical port of vendor/resin/webui into src/ — REJECTED for the desktop
  path (we keep the shell-forwarding contract). The Resin webui remains the
  authoritative VPS deployment frontend served by the sidecar itself.
- Any dual-mode shim or shared React component library ambitions between our
  shell and Resin's webui. The back-port is selective and bespoke, not a
  shared library.

Status: ACCEPTED (pending user confirmation of this internal decision).

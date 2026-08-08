# ADR-0021: SOCKS5 Auth Exposure + Port Health Check

**Status**: Accepted
**Date**: 2026-08-08
**Supersedes**: None (complements ADR-0012)

## Context

Users report SOCKS5 entry ports are unreachable. Root cause traced through
Resin v1.2.0 source code (`resin/internal/proxy/socks5.go`):

1. Resin's SOCKS5 handler **always requires username/password auth** when
   `RESIN_PROXY_TOKEN` is non-empty (which our shell sets every boot).
2. Username = `Platform.Account` string (e.g. `Default.port-17990`).
3. Password = the `RESIN_PROXY_TOKEN` value (32-hex random string).
4. The shell never exposes this token to the GUI (AGENTS §7.6 security rule),
   so users/AI-gateways cannot authenticate → connection refused.
5. No port health check IPC exists — no way to verify a listener is actually
   bound and accepting connections.

### No-auth mode research

Resin's SOCKS5 auth is **globally token-driven**, not per-port:
- `negotiateMethod` (socks5.go L261): `if s.token != "" → MUST UserPass`
- Per-endpoint `require_proxy_auth_info` field is **inverse**: only forces
  username when global token is empty.
- Cannot support per-port auth/no-auth coexistence without forking Resin's
  `negotiateMethod` logic or upstream adding a per-endpoint `allow_no_auth`
  flag.

## Decision

1. **Expose SOCKS5 auth credentials to GUI** via new `port_auth_info` IPC.
   Returns `{ username: "Platform.Account", password: proxy_token }` per port.
   GUI displays credentials with copy-to-clipboard. Token is loopback-only;
   no increased attack surface (webview already local).

2. **Add `port_health_check` IPC** — TCP connect to port + optional SOCKS5
   handshake. Returns `{ reachable: bool, auth_required: bool, latency_ms: u64 }`.
   GUI shows green/red status per port. Auto-check after port create/update.

3. **No-auth mode**: not supported per-port (Resin architecture limit).
   Documented as known limitation. If user wants no-auth, they must use HTTP
   forward proxy mode (Resin allows no-auth HTTP when token check is on the
   Proxy-Authorization header, which HTTP clients can omit).

## Consequences

- `SidecarHandle.proxy_token` becomes readable via IPC (but only the auth
  info helper, not a raw token dump).
- Port cards in PlatformsView show auth credentials + health status.
- Integration test: create port → health check → SOCKS5 connect → verify
  tunneling. Closes the "toy" gap for entry ports.
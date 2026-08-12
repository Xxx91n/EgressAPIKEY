# ADR-0027: Empty proxy_token enables no-auth on all entry ports

## Status
ACCEPTED (T7-fix, 2026-08-13)

## Context

Resin v1.2.0 SOCKS5 and HTTP forward proxy auth negotiation:

- `socks5.go:261`: `if s.token != "" || requireAuthInfo { ... } else { no-auth path }`
- `forward.go:103`: `if p.token == "" { no-auth path } else { 407 ErrAuthRequired }`

The shell previously generated a non-empty `RESIN_PROXY_TOKEN` (32 hex chars via
`gen_token()`). This forced Resin to reject no-auth connections on ALL ports,
regardless of the per-endpoint `require_proxy_auth_info` DB flag (0 = no-auth).

Effect: omniroute (and any no-auth client) could not connect to any entry port:
- SOCKS5 greeting `05 01 00` got `05 FF` (No Acceptable Methods)
- HTTP CONNECT got `407 Proxy Authentication Required` with `X-Resin-Error: AUTH_REQUIRED`

The GUI showed "无需认证" (no auth needed) because `egressapikey-ports.json` had
`auth_required: false`, but this flag only controls the `requireAuthInfo` parameter
which is ignored when `s.token != ""` (the OR condition).

## Decision

Set `proxy_token = String::new()` (empty) in `sidecar.rs:273`.

This makes Resin accept no-auth on all ports where `require_proxy_auth_info=0`.
When `require_proxy_auth_info=1` (auth_required=true), the username is
`<Platform>.<Account>` and the password can be anything (empty is fine)
because `socks5.go:307` short-circuits the password check when `s.token == ""`.

## Source verification

### SOCKS5 (`resin/internal/proxy/socks5.go`)

- Line 261: `if s.token != "" || requireAuthInfo {` -> empty token makes the OR false
- Line 265-271: `else` branch accepts both `socks5MethodNoAuth(0x00)` and `socks5MethodUserPass(0x02)`
- Line 307: `if s.token != "" && string(password) != s.token` -> short-circuits when token is empty
- Line 311: `if requireAuthInfo && (len(username) == 0 || len(password) == 0)` -> only blocks empty when auth required

### HTTP Forward (`resin/internal/proxy/forward.go`)

- Line 103: `if p.token == "" {` -> no-auth path
- Line 105-109: no Proxy-Authorization + `!requireProxyAuthInfo(r)` -> returns `nil` (success)

## Consequences

- Ports with `auth_required=false`: no-auth works (omniroute connects directly)
- Ports with `auth_required=true`: user=Platform.Account, pass=anything (empty is fine)
- `port_auth_info` IPC returns `password: ""` (empty) -- GUI shows "无需认证" correctly
- `port_health_check` SOCKS5 greeting now sends `05 02 00 02` (NoAuth + UserPass)
  and accepts `0x00` or `0x02` in the response match

## E2E verification (T7-fix commit 97848a6)

- HTTP CONNECT via 1790: `200 Connection Established` (was 407)
- SOCKS5 NoAuth via 1791: `0x05 0x00` (was 0xFF)
- SOCKS5 NoAuth via 1799: `0x05 0x00` (was 0xFF)
- Full SOCKS5 CONNECT 1.1.1.1:80 via 1791: rep=0, tunnel open, CF response received

## Previous (wrong) conclusion that this ADR supersedes

The code comment at `sidecar.rs:269` (pre-T7) said:
  "ADR-0027 SUPERSEDED: empty proxy_token breaks SOCKS5 for any endpoint
   with require_proxy_auth_info=true"

This was wrong. Source verification proves empty token is safe for both auth paths.
The original "breaks SOCKS5" was likely a misdiagnosis of a different issue.


## Related
- ADR-0021: SOCKS5 auth and port-health (original port_auth_info design)
- ADR-0012: Thin-shell multi-port forwarding (architecture)
- ADR-0028: Whitebox network DNS struct
- `docs/T7-TEST-NETWORK-DIAGNOSIS.md`: live port diagnosis with raw wire evidence
- `resin/internal/proxy/socks5.go:261`, `resin/internal/proxy/forward.go:103`
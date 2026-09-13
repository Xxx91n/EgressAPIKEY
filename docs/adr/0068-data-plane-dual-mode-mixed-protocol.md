# ADR-0068: data-plane dual mode (shell forwarder + engine direct) and mixed protocol entry ports

> Status: ACCEPTED (2026-09-13, round8-grill decisions D-001/D-007)
> Extends (reopens none): ADR-0012 (thin-shell multi-port forwarder, port = identity), ADR-0015 (port_forwarder thin-shell downgrade).
> Research basis: atomcode Q3/Q7 (IETF BFF BCP draft-27; sing-box mixed inbound + mihomo mixed-port first-byte peek; golang/go#18508, psf/requests#3516).

## Context

Round 8's grill verified against source that ADR-0012's forwarder never
materialised: port_forwarder.rs carries the identity/auth building blocks
(resin_identity, detect_protocol, basic_proxy_auth) but no TcpListener,
and main.rs constructs the forwarder without a listen loop; Resin
endpoints are created without platform/account in the body, so the
de-facto data plane is engine-direct with client-supplied credentials
(the sharp critique's "25% data-plane completion" finding, confirmed).
The grill chose dual mode over an immediate fork (D-001) and
mixed-protocol entry over single-protocol (D-007): HTTP proxy is the
client-ecosystem common denominator, SOCKS5 support is a dependency tax
across ecosystems, and SOCKS5's unique advantages (UDP, arbitrary TCP,
remote DNS) are irrelevant to the HTTPS+SSE workload.

## Decision

### D1. Dual data-plane mode (D-001)

Mode A — shell forwarder: the Rust shell listens on entry ports, clients
connect credential-free, and the forwarder injects the port's
(platform, account) as the proxy credential toward Resin. Mode B —
engine direct: Resin listens natively; the client supplies the
Platform.Account credential once. Defaults: Windows desktop = A;
VPS/headless = B. README claims "zero adaptation" only for mode A.

### D2. Fork fallback (D-001)

If A+B proves unmaintainable or misses the round-8 performance targets
(decision D-004), maintain a Resin fork and offer the work upstream as
PRs. Not scheduled now.

### D3. Mixed protocol entry ports (D-007)

Port protocol enum becomes http | socks5 | mixed, mixed the default.
Engine flag mapping tightens: mixed opens both allow_socks5 and
allow_http_forward; http opens only allow_http_forward; socks5 opens
only allow_socks5 (the current implicit "socks5 also opens
http_forward" is removed; existing ports migrate once). The A-mode
forwarder sniffs the connection's first byte (0x05 = SOCKS5, else HTTP)
and injects the port's credential in the declared dialect
(Proxy-Authorization basic / RFC 1929 username-password).

### D4. Pre-implementation verification gate

Resin ships no first-party documentation for the dual-flag behaviour:
before implementation lands, one live port must answer both
`curl -x http://` and `curl -x socks5h://` to confirm same-port
auto-detection. HTTPS/TLS fronting of entry ports is explicitly out of
scope for this decision.

## Consequences

- The shell gains real per-port listeners (tokio): the round-8
  acceptance table applies, including the four SSE behavioural
  assertions (decision D-004) as this path's hard acceptance.
- B-mode remains the VPS/headless default; the forwarder is an optional
  layer, not a dependency of headless deployments (D-003 keeps the BFF
  architecture separate from the data plane).
- Existing whitebox rows migrate from the two-value protocol enum to the
  three-value enum in the same change that tightens the flag mapping.

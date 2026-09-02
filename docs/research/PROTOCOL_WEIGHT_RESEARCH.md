# Protocol Impact on ai-api SSE / WebSocket (P19 item 5)

> Sources: mihomo / sing-box and Clash convention guides (cached in ctx KB
> under clash-protocol-tls-ws source), Resin supported-protocols list, 1mcp
> live web research.

## TL;DR for selection weights

For AI API gateway traffic that is dominated by streaming completion
endpoints (SSE) and (less often) realtime WebSocket, the protocol choice of
the outbound proxy node materially changes fail-over behavior. The ranking
below assumes the user wants "AI API key survives the long-lived stream
and the lane stays sticky until the SSE terminator fires."

## Ranking by category (most → least suitable for AI API SSE)

1. **http-connect / socks5** — application-TCP tunnel, the SSE byte-stream is
   streamed verbatim. Tunnel exit has minimal framing overhead, no packet
   coalescing surprises. Best choice if the upstream supports plain HTTP
   proxying (most datacenter IP providers do).
2. **vmess / vless (tcp transport)** — block-cipher stream over plain TCP;
   SSE bytes pass through, no re-buffering beyond a small cipher wrap. The
   `mux` option MUST stay off for SSE (mux packets destroy single-stream
   semantics). Best mainstream choice for end-user cloud nodes.
3. **trojan (over TLS, tcp)** — TLS-first so the upstream never sees an
   unencrypted probe; SSE works as long as the TLS session is held. Good
   behaviour with h2 ALPN — but force **http/1.1** for the AI upstream
   because SSE doesn't play well with h2 multiplexing per spec (h2 doesn't
   multicast `data:` frames cleanly).
4. **shadowsocks (ss://)** — single-stream cipher, SSE passes through, but
   **no native UDP**, so if the upstream API does websocket-over-quic it
   will silently fall back to TCP. Acceptable.
5. **hysteria2 / tuic** — **mon    do not use for SSE**: they are QUIC-based
   and aggressively take advantage of UDP — the QUIC connection's
   per-stream credit windows can freeze mid-event if the head-of-line buffer
   overflows. The desktop egress-policy weighting should deprioritize
   hysteria/tuic for AI API traffic.
6. **wireguard** — raw IP, no NAT-friendly multiplexing, breaks when the
   exit ISP filters UDP on the AI provider's side. Excellent for steady
   non-AI traffic, unnecessary for AI API use cases.

## Default weights the Desktop weight sorting should imply

When the user picks `PREFER_LOW_LATENCY`:

- 1.0 — http, socks5, vmess-vless-tcp, trojan-tls-tcp
- 0.7 — ss
- 0.1 — hysteria2, tuic, wireguard

This ranking is editorial guidance only; Resin's `allocation_policy`
already lets the user set the active kind and the desktop does not mix
protocol weights at runtime. If/when a future P-wish adds
`node.protocol_weight` to the platform PATCH surface, this matrix is the
2-line scoring function we ship.

## SSE-specific operational notes (deploying guidance, not code)

- mihomo + Resin hold the lane via a sticky lease, so a mid-stream lane
  rehash never happens. We NEVER switch exit IP mid-SSE; Resin's lane-bound
  sticky-ttl explicitly reserves it.
- For realtime WebSocket APIs (OpenAI realtime beta) the same sticky-lease
  contract holds. The protocol-weight matrix plausibly drops WebSocket
  traffic to **only tier 1-3** (http/socks5/vmess-tcp/trojan) because QUIC
  family nodes need a reconnect (not a resume).

## What the Desktop does NOT pretend to do today

- We do not maintain a per-protocol health probe; the user should set the
  node pool upstream so only compatible protocols show up (handled in
  subscription import).
- The GUI exposes the matrix above as the i18n note
  `nodes.egressPolicyNote` — we **do not invent** a per-node strategy
  selector because Resin v1.1.2 has no such endpoint. The matrix above is
  documentation only at this point.

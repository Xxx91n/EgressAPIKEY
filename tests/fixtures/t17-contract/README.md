# t17 contract - data-plane Mode A byte-level baseline

Authoritative sources: ADR-0068 D4 (four-scenario gate table) and round8
ticket 17 (A-001, "credential is identity"). The automated gate living in
`scripts/mode-a-contract-check.cjs` re-establishes everything below on every
CI run; this document records the hand recipe and the byte-level baseline so
a failing gate can be reproduced and diagnosed without archaeology.

Original live evidence (raw transcripts, gitignored scratch):
`.scratch/architecture-recovery-closed-2026-09-16/repro/d4-mixed/` and
`.scratch/architecture-recovery-closed-2026-09-16/repro/t17-contract/`.

## Byte-level baseline (Resin v1.2.0, custom endpoints)

| endpoint flags | HTTP probe (`curl -x http://` / raw CONNECT) | SOCKS5 probe (`curl -x socks5h://` / raw greeting) |
|---|---|---|
| mixed (`allow_socks5` + `allow_http_forward`) | `HTTP/1.1 503` + `X-Resin-Error: NO_AVAILABLE_NODES` (no nodes provisioned -> failure-path reply, deterministic) | method reply `05 00` (no-auth accepted); CONNECT to unreachable target -> in-band `05 01` general failure |
| http only (`allow_http_forward` only) | (HTTP served normally) | refused: `05 ff` (no acceptable methods) |
| socks5 only (`allow_socks5` only) | refused: `HTTP/1.1 403` + `X-Resin-Error: ENDPOINT_CAPABILITY_DISABLED`, body `Proxy capability is disabled on this endpoint` | (SOCKS5 served normally) |

Note the underscore in `NO_AVAILABLE_NODES` - the error token is carried
verbatim in the `X-Resin-Error` header; hyphenated variants are wrong.

## Identity attribution ("credential is identity")

Boot with `RESIN_AUTH_VERSION=V1` and `RESIN_PROXY_TOKEN=""` (explicitly
empty = no-auth on endpoints with `require_proxy_auth_info=0`). The read-only
default (consolidated) endpoint then PUBLISHES whatever credential the client
supplies into routing metadata - the request log attributes it even though no
auth check happens, and even on the 503 failure path (attribution happens
before route resolution, `forward.go` `lifecycle.setAccount` precedes
`resolveRoutedOutbound`):

- HTTP absolute-form with `Proxy-Authorization: Basic base64("T17P.ac1:")` ->
  503 `NO_AVAILABLE_NODES`; request-log row `account=ac1`.
- SOCKS5 greeting offering `0x02` (UserPass), RFC1929 identity
  `T17P.ac2` with zero-length password -> in-band `05 01` on unroutable
  target; request-log row `account=ac2` (`proxy_type=3`).
- No credential at all -> 503; request-log row with empty `account`.

Provisioning a platform is NOT required for attribution (empty `platform` on
rows); it only changes routing behaviour.

## Boot recipe (live probing by hand)

Env (mirrors `src-tauri/src/sidecar.rs`):

    RESIN_AUTH_VERSION=V1
    RESIN_ADMIN_TOKEN=<random hex>
    RESIN_PROXY_TOKEN=            # empty on purpose
    RESIN_LISTEN_ADDRESS=127.0.0.1
    RESIN_PORT=<admin port>
    RESIN_STATE_DIR/RESIN_CACHE_DIR/RESIN_LOG_DIR=<fresh temp dirs>
    RESIN_REQUEST_LOG_QUEUE_FLUSH_INTERVAL=2s   # default is 5 MINUTES; without
                                                # this the log assertion stalls

Wait for `GET /healthz` (unauthenticated) -> 200 `{"status":"ok"}`. Admin
calls carry `Authorization: Bearer <RESIN_ADMIN_TOKEN>`.

Provision three endpoints (all `allow_proxy=true`, `allow_http_reverse=false`,
`allow_management=false`):

    POST /api/v1/endpoints  mixed:    {port, allow_socks5:true, allow_http_forward:true}
    POST /api/v1/endpoints  http-only:{port, allow_socks5:false, allow_http_forward:true}
    POST /api/v1/endpoints  socks5-only:{port, allow_socks5:true, allow_http_forward:false}

(All flag fields must be explicit - omitted bools default to the engines
fallback, not to false.) Optionally `POST /api/v1/platforms {"name":"T17P"}`.

Probe (curl equivalents of the four scenarios):

    curl -si -x http://127.0.0.1:<mixed> http://1.1.1.1/     -> 503 + X-Resin-Error: NO_AVAILABLE_NODES
    curl -sv -x socks5h://T17P.ac2:@127.0.0.1:<mixed> http://1.1.1.1/  -> method 05 00, then exit 97 (general failure)
    curl -sv -x socks5h://127.0.0.1:<http-only> http://1.1.1.1/        -> 05 ff, exit 97/1
    curl -si -x http://127.0.0.1:<socks5-only> http://1.1.1.1/         -> 403 + X-Resin-Error: ENDPOINT_CAPABILITY_DISABLED

Then read attribution back:

    GET /api/v1/request-logs?limit=100

rows carry `account`, `http_status`, `resin_error`, `proxy_type`,
`platform_name` (paged as `{"items":[...]}`).

A practical curl-vs-node note: curl draws `05 ff` refusals as exit 97 (or 1)
with no distinguishing stdout - byte-faithful assertions are far easier over a
raw socket, which is what the automated gate does.

Automated re-establishment: `node scripts/mode-a-contract-check.cjs` (mounted
as the `contracts` sub-step of `scripts/verify-build.sh`; CI = live probes
against the fetched sidecar, local without a binary = static self-checks +
skip).

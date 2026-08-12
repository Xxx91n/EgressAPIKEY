# T7-TEST: Network Layer Port Connectivity Diagnosis

## Date
2026-08-12 22:10 SGT

## Test Environment
- EgressAPIKEY.exe running (PID 16824, WS 40MB)
- Resin sidecar running (PID 5392, WS 82MB)
- Sidecar API: 127.0.0.1:47154 (/healthz = 200 OK)
- Entry ports: 1790 (HTTP), 1791 (SOCKS5), 1799 (SOCKS5) -- all TCP OPEN
- Platforms: Default (allocation: PREFER_LOW_LATENCY, region_filters: ["jp"])
- Subscription: "1" (local content, ~53 vless + http nodes)
- egressapikey-ports.json: all 3 ports have auth_required: false

## Test Results

### Port 1790 (HTTP Forward Proxy)

| Test | Result |
|------|--------|
| TCP connect | OPEN |
| GET / (direct) | 404 Not Found (expected for proxy port) |
| CONNECT 1.1.1.1:80 (no auth) | **407 Proxy Authentication Required** |
| GET http://1.1.1.1/cdn-cgi/trace (no auth) | **407 Proxy Authentication Required** |
| .NET HttpClient via WebProxy | **407 Proxy Authentication Required** |

HTTP response headers: Proxy-Authenticate: Basic realm="Resin", X-Resin-Error: AUTH_REQUIRED

### Port 1791 (SOCKS5)

| Test | Result |
|------|--------|
| TCP connect | OPEN |
| SOCKS5 greeting (method 0x00 no-auth) | **0xFF (No Acceptable Methods)** |
| SOCKS5 greeting (methods 0x00+0x02) | **0x02 (UserPass selected)** |
| SOCKS5 CONNECT 1.1.1.1:80 (no-auth) | **Rejected at greeting** |

### Port 1799 (SOCKS5)

| Test | Result |
|------|--------|
| TCP connect | OPEN |
| SOCKS5 greeting (method 0x00 no-auth) | **0xFF (No Acceptable Methods)** |
| SOCKS5 greeting (methods 0x00+0x02) | **0x02 (UserPass selected)** |

## Root Cause

**All entry ports force proxy authentication despite auth_required=false in the port config.**

Resin source code evidence:

### SOCKS5 (resin/internal/proxy/socks5.go line 261)

    if s.token != "" || requireAuthInfo {
        if containsSocks5Method(methods, socks5MethodUserPass) {
            selected = socks5MethodUserPass
        }
        // else: 0xFF NoAcceptable -- rejects no-auth!
    } else {
        // no-auth accepted here
    }

### HTTP Forward (resin/internal/proxy/forward.go line 103-124)

    if p.token == "" {
        // no-auth OK, credential optional
    } else {
        // MUST have Proxy-Authorization matching token -> 407 ErrAuthRequired
    }

### Shell sidecar.rs (line 273)

    let proxy_token = gen_token();  // 32 hex chars, NON-EMPTY
    cmd.env("RESIN_PROXY_TOKEN", &proxy_token)

**The proxy_token is always non-empty -> Resin forces auth on ALL ports.**
The GUI shows "无需认证" because egressapikey-ports.json.auth_required=false,
but this flag only controls the requireAuthInfo parameter -- it is ignored
when s.token != "" (the OR condition in socks5.go:261 and the else branch
in forward.go:103).

## Impact

1. omniroute cannot connect -- it sends no-auth SOCKS5/HTTP, gets 0xFF/407
2. GUI dashboard is misleading -- shows port online + "无需认证" but connections fail
3. The user entire network test fails -- no port can pass traffic without credentials
4. The credential format is non-obvious -- SOCKS5 user must be <Platform>.<account>
   (e.g. Default.port-1791) and password must be the proxy_token (not visible in GUI)

## Fix Options

### Option A: Empty proxy_token (enables no-auth on all ports)
Set RESIN_PROXY_TOKEN="" in sidecar.rs. Resin SOCKS5 + HTTP forward paths
will accept no-auth when s.token == "". The require_proxy_auth_info DB
flag becomes the sole auth gate -- when false, ports are open; when true,
basic auth with platform.account + empty token is used.

Risk: ADR-0027 was originally created because empty token broke SOCKS5.
That was likely a different bug. Need to verify Resin source handles
empty token correctly in UserPass path (socks5.go:authenticateUserPass).

### Option B: Wire credentials into GUI (correct but heavy)
- Expose proxy_token in port_auth_info IPC (already exists)
- GUI shows credentials per port (user: Default.port-1791, pass: <proxy_token>)
- omniroute configured with SOCKS5 auth: user=Default.port-1791 pass=<token>
- Still requires user to copy-paste credentials into omniroute config

### Option C: Hybrid (Ponytail)
- Set proxy_token="" for ports where auth_required=false
- Keep non-empty proxy_token only for ports where auth_required=true
- Requires per-endpoint token override in Resin (may not exist in v1.2.0 API)

## Verification Commands

After fix, run these to verify the tunnel is open:

    # HTTP CONNECT test (no auth, should get 200 OK)
    $sock = [System.Net.Sockets.TcpClient]::new("127.0.0.1", 1790)
    $stream = $sock.GetStream()
    $request = "CONNECT 1.1.1.1:80 HTTP/1.1" + [char]13 + [char]10
    $request += "Host: 1.1.1.1:80" + [char]13 + [char]10 + [char]13 + [char]10
    $bytes = [Text.Encoding]::ASCII.GetBytes($request)
    $stream.Write($bytes, 0, $bytes.Length); $stream.Flush()
    Start-Sleep -Milliseconds 3000
    $buf = New-Object byte[] 4096; $read = $stream.Read($buf, 0, 4096)
    [System.Text.Encoding]::ASCII.GetString($buf, 0, $read)

    # SOCKS5 no-auth greeting (should get 0x05 0x00)
    $sock = [System.Net.Sockets.TcpClient]::new("127.0.0.1", 1791)
    $stream = $sock.GetStream()
    $greeting = [byte[]](5, 1, 0)
    $stream.Write($greeting, 0, 3); $stream.Flush()
    Start-Sleep -Milliseconds 500
    $buf = New-Object byte[] 2; $read = $stream.Read($buf, 0, 2)
    # Expected: read=2, bytes=5 0 (version=5, method=NoAuth)

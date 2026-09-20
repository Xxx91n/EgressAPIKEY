//! Mode A data-plane integration tests (round-8 ticket 17; A-001 + A-015).
//!
//! Chain under test: client -> shell forwarder (entry port, Mode A)
//! -> mock consolidated port (fake Resin proxy listener: SOCKS5 RFC 1929
//! UserPass + HTTP Basic CONNECT on one port) -> mock origin (fake AI
//! endpoint). All legs are real loopback TCP sockets - per the spec's
//! testing decision the behaviour is measured end to end, not through
//! stubbed traits. The mock consolidated port replicates exactly what the
//! ADR-0068 D4 gate and the ticket-17 contract probes observed live
//! (.scratch/architecture-recovery/repro/d4-mixed/ + t17-contract/): one
//! port, both dialects, presented credentials parsed and relayed raw.
//!
//! Coverage map to the issue's acceptance boxes:
//!   - per-entry-port TcpListener + dual-protocol state machine:
//!     mode_a_socks5_..., mode_a_http_connect_..., dialect_gate_...
//!   - credential injection per dialect: rfc1929_... + basic_... asserts
//!   - SSE four behavioural assertions: sse_flushes_per_event...,
//!     client_abort_cascades..., slow_client_backpressure...,
//!     sse_upstream_reset_gets_in_band_error_frame
//!   - p95 <= 5ms added latency (mock upstream full chain):
//!     paired_request_added_latency_p95_under_5ms
//!   - the ticket-04 SSE flush-defect fix (forward path never used):
//!     absolute_form_never_reaches_the_engine_forward_path

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use resin_core::port_forwarder::basic_proxy_auth;
use resin_core::{DbPool, PortForwarder, PortMapping};

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

fn mapping(port: u16, protocol: &str, enabled: bool) -> PortMapping {
    PortMapping {
        port,
        protocol: protocol.to_string(),
        platform_name: "OpenAI".into(),
        account: format!("port-{port}"),
        label: String::new(),
        enabled,
        auth_required: true,
    }
}

/// Boot a Mode A forwarder bound on a fresh entry port, relaying to
/// engine_port with the given declared protocol. Waits for the bind.
async fn boot_shell_forwarder(entry_port: u16, engine_port: u16, protocol: &str) -> PortForwarder {
    let db = DbPool::open_in_memory().expect("mem db");
    let f = PortForwarder::shell(db, "127.0.0.1", engine_port, "t0k");
    f.reload(&[mapping(entry_port, protocol, true)])
        .await
        .expect("reload");
    for _ in 0..150 {
        if f.running_ports().contains(&entry_port) {
            return f;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("entry port {entry_port} never bound");
}

/// The identity the forwarder must inject for a port: OpenAI.port-<n> (the
/// account default in mapping()).
fn expected_identity(entry_port: u16) -> String {
    format!("OpenAI.port-{entry_port}")
}

// ---------------------------------------------------------------------------
// Mock origin: the fake AI endpoint behind the engine
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct OriginStats {
    accepted: Arc<AtomicUsize>,
    bytes_written: Arc<AtomicUsize>,
    /// Instant the origin finished writing its whole payload (proof of
    /// unbounded buffering if this lands while the client idles).
    done_at: Arc<Mutex<Option<Instant>>>,
    /// Instant the origin observed its peer going away mid-stream (proof of
    /// the cancel cascade).
    peer_gone_at: Arc<Mutex<Option<Instant>>>,
    /// First request head bytes read on the latest connection.
    last_request: Arc<Mutex<Vec<u8>>>,
}

impl OriginStats {
    fn new() -> Self {
        Self {
            accepted: Arc::new(AtomicUsize::new(0)),
            bytes_written: Arc::new(AtomicUsize::new(0)),
            done_at: Arc::new(Mutex::new(None)),
            peer_gone_at: Arc::new(Mutex::new(None)),
            last_request: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum OriginMode {
    /// Echo every byte until EOF (raw tunnel ping tests).
    Echo,
    /// One HTTP/1.1 response then close (absolute-form replay tests).
    HttpOnce,
    /// 5 SSE events spaced ~40 ms, then clean close.
    SsePaced,
    /// Stream SSE events forever until the peer goes away (cancel tests).
    StreamForever,
    /// Write a 16 MiB body as fast as possible (backpressure tests).
    Flood,
    /// Head + one event, brief pause, then hard reset (in-band error tests).
    SseThenReset,
}

async fn spawn_origin(mode: OriginMode) -> (u16, OriginStats) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let stats = OriginStats::new();
    let st = stats.clone();
    tokio::spawn(async move {
        loop {
            let Ok((s, _)) = l.accept().await else { return };
            let st = st.clone();
            tokio::spawn(async move {
                st.accepted.fetch_add(1, Ordering::SeqCst);
                handle_origin_conn(s, mode, st).await;
            });
        }
    });
    (port, stats)
}

async fn handle_origin_conn(mut s: TcpStream, mode: OriginMode, st: OriginStats) {
    let mut buf = vec![0u8; 8192];
    match mode {
        OriginMode::Echo => loop {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if s.write_all(&buf[..n]).await.is_err() || s.flush().await.is_err() {
                        break;
                    }
                }
            }
        },
        OriginMode::HttpOnce => {
            if let Ok(n) = s.read(&mut buf).await {
                *st.last_request.lock() = buf[..n].to_vec();
            }
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPONG")
                .await;
            let _ = s.flush().await;
        }
        OriginMode::SsePaced => {
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
                .await;
            let _ = s.flush().await;
            for i in 0..5 {
                let ev = format!("data: e{i}\n\n");
                if s.write_all(ev.as_bytes()).await.is_err() || s.flush().await.is_err() {
                    *st.peer_gone_at.lock() = Some(Instant::now());
                    return;
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        }
        OriginMode::StreamForever => {
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
                .await;
            let _ = s.flush().await;
            for i in 0..5000 {
                let ev = format!("data: e{i}\n\n");
                if s.write_all(ev.as_bytes()).await.is_err() || s.flush().await.is_err() {
                    *st.peer_gone_at.lock() = Some(Instant::now());
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        OriginMode::Flood => {
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\r\n")
                .await;
            let _ = s.flush().await;
            let chunk = vec![0xA5u8; 4096];
            let start = Instant::now();
            loop {
                if start.elapsed() > Duration::from_secs(10) {
                    return;
                }
                if s.write_all(&chunk).await.is_err() {
                    *st.peer_gone_at.lock() = Some(Instant::now());
                    return;
                }
                let _ = s.flush().await;
                let w = st.bytes_written.fetch_add(4096, Ordering::SeqCst) + 4096;
                if w >= 16 * 1024 * 1024 {
                    *st.done_at.lock() = Some(Instant::now());
                    return;
                }
            }
        }
        OriginMode::SseThenReset => {
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: e0\n\n",
                )
                .await;
            let _ = s.flush().await;
            // Let the bytes be absorbed by the relay before the reset, so
            // the client must see event 0 and the injected error frame.
            tokio::time::sleep(Duration::from_millis(150)).await;
            // linger 0 + close => TCP RST delivered to the relay (not a
            // clean FIN), which is what the in-band error path must react to.
            let _ = s.set_linger(Some(Duration::ZERO));
            drop(s);
        }
    }
}

// ---------------------------------------------------------------------------
// Mock consolidated port: the fake Resin proxy listener (both dialects)
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct EngineState {
    socks_users: Arc<Mutex<Vec<String>>>,
    socks_passes: Arc<Mutex<Vec<String>>>,
    auth_headers: Arc<Mutex<Vec<String>>>,
    request_lines: Arc<Mutex<Vec<String>>>,
    tunnels: Arc<AtomicUsize>,
}

async fn spawn_engine(origin_port: u16) -> (u16, EngineState) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let st = EngineState::default();
    let conn_st = st.clone();
    tokio::spawn(async move {
        loop {
            let Ok((s, _)) = l.accept().await else { return };
            let st = conn_st.clone();
            tokio::spawn(async move {
                let _ = engine_session(s, origin_port, st).await;
            });
        }
    });
    (port, st)
}

async fn engine_session(mut s: TcpStream, origin_port: u16, st: EngineState) {
    let mut first = [0u8; 1];
    if s.read_exact(&mut first).await.is_err() {
        return;
    }
    if first[0] == 0x05 {
        // SOCKS5: the forwarder offers UserPass only, like a strict client.
        let mut n = [0u8; 1];
        if s.read_exact(&mut n).await.is_err() {
            return;
        }
        let mut methods = vec![0u8; n[0] as usize];
        if s.read_exact(&mut methods).await.is_err() {
            return;
        }
        if !methods.contains(&0x02) {
            let _ = s.write_all(&[0x05, 0xFF]).await;
            return;
        }
        let _ = s.write_all(&[0x05, 0x02]).await;
        // RFC 1929 subnegotiation.
        let mut ver = [0u8; 1];
        if s.read_exact(&mut ver).await.is_err() || ver[0] != 0x01 {
            return;
        }
        let mut ul = [0u8; 1];
        if s.read_exact(&mut ul).await.is_err() {
            return;
        }
        let mut user = vec![0u8; ul[0] as usize];
        if s.read_exact(&mut user).await.is_err() {
            return;
        }
        let mut pl = [0u8; 1];
        if s.read_exact(&mut pl).await.is_err() {
            return;
        }
        let mut pass = vec![0u8; pl[0] as usize];
        if s.read_exact(&mut pass).await.is_err() {
            return;
        }
        st.socks_users
            .lock()
            .push(String::from_utf8_lossy(&user).to_string());
        st.socks_passes
            .lock()
            .push(String::from_utf8_lossy(&pass).to_string());
        let _ = s.write_all(&[0x01, 0x00]).await;
        // CONNECT request
        let mut hdr = [0u8; 4];
        if s.read_exact(&mut hdr).await.is_err() || hdr[0] != 0x05 || hdr[1] != 0x01 {
            return;
        }
        let (host, p) = match read_addr(&mut s, hdr[3]).await {
            Some(v) => v,
            None => return,
        };
        st.request_lines
            .lock()
            .push(format!("SOCKS CONNECT {host}:{p}"));
        let _ = p; // (test-only shortcut: every CONNECT targets the loopback origin)
        match TcpStream::connect(("127.0.0.1", origin_port)).await {
            Ok(o) => {
                let _ = s
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await;
                st.tunnels.fetch_add(1, Ordering::SeqCst);
                tunnel_copy(s, o).await;
            }
            Err(_) => {
                let _ = s
                    .write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await;
            }
        }
        return;
    }
    // HTTP side
    let mut buf = vec![first[0]];
    loop {
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        let mut tmp = [0u8; 512];
        match s.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
        if buf.len() > 64 * 1024 {
            return;
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let line = head.lines().next().unwrap_or("").to_string();
    st.request_lines.lock().push(line.clone());
    for l in head.lines() {
        if l.to_ascii_lowercase().starts_with("proxy-authorization:") {
            st.auth_headers.lock().push(l.to_string());
        }
    }
    if !line.starts_with("CONNECT ") {
        // The forwarder must never leave a forward-form request on this
        // path (ticket-04 defect); record it for assertion 3.
        let _ = s
            .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
            .await;
        return;
    }
    let target = line.split_whitespace().nth(1).unwrap_or("");
    let (host, p) = match target.rsplit_once(':') {
        Some((h, pp)) => (h.to_string(), pp.parse::<u16>().unwrap_or(0)),
        None => (target.to_string(), 443),
    };
    let _ = host;
    match TcpStream::connect(("127.0.0.1", p)).await {
        Ok(o) => {
            let _ = s
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await;
            st.tunnels.fetch_add(1, Ordering::SeqCst);
            tunnel_copy(s, o).await;
        }
        Err(_) => {
            let _ = s
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await;
        }
    }
}

/// Relay like the engine's tunnel does: an ABNORMAL break on either side
/// resets the other (that is how the in-band-error test makes the forwarder
/// see a reset rather than a clean EOF), a clean end closes normally.
async fn tunnel_copy(mut tunnel: TcpStream, mut peer: TcpStream) {
    let r = tokio::io::copy_bidirectional(&mut tunnel, &mut peer).await;
    if r.is_err() {
        let _ = tunnel.set_linger(Some(Duration::ZERO));
        let _ = peer.set_linger(Some(Duration::ZERO));
    }
}

async fn read_addr(s: &mut TcpStream, atyp: u8) -> Option<(String, u16)> {
    match atyp {
        0x03 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len).await.ok()?;
            let mut name = vec![0u8; len[0] as usize];
            s.read_exact(&mut name).await.ok()?;
            let mut p = [0u8; 2];
            s.read_exact(&mut p).await.ok()?;
            Some((
                String::from_utf8_lossy(&name).to_string(),
                u16::from_be_bytes(p),
            ))
        }
        0x01 => {
            let mut ip = [0u8; 4];
            s.read_exact(&mut ip).await.ok()?;
            let mut p = [0u8; 2];
            s.read_exact(&mut p).await.ok()?;
            Some((
                format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
                u16::from_be_bytes(p),
            ))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Client-side test helpers
// ---------------------------------------------------------------------------

/// Raw SOCKS5 client: credential-free offer, CONNECT, verify success.
/// Returns the stream ready for tunnel bytes.
async fn socks_open(fwd_port: u16, host: &str, port: u16) -> TcpStream {
    let mut s = TcpStream::connect(("127.0.0.1", fwd_port)).await.unwrap();
    s.write_all(&[0x05, 0x01, 0x02]).await.unwrap(); // offer UserPass only
    let mut rep = [0u8; 2];
    s.read_exact(&mut rep).await.unwrap();
    assert_eq!(
        rep,
        [0x05, 0x00],
        "Mode A must select NoAuth credential-free"
    );
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut rh = [0u8; 4];
    s.read_exact(&mut rh).await.unwrap();
    assert_eq!(rh[1], 0x00, "socks CONNECT reply: {:02x?}", rh);
    // Forwarder always answers with the fixed 10-byte IPv4 success form;
    // the 4 header bytes above leave exactly 6 to consume.
    let mut rest = [0u8; 6];
    s.read_exact(&mut rest).await.unwrap();
    s
}

// ---------------------------------------------------------------------------
// Acceptance: listener + state machine + per-dialect credential injection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_a_socks5_relays_with_rfc1929_injected_identity() {
    let (origin_port, _os) = spawn_origin(OriginMode::Echo).await;
    let (engine_port, es) = spawn_engine(origin_port).await;
    let entry = free_port().await;
    let _f = boot_shell_forwarder(entry, engine_port, "mixed").await;

    let mut s = socks_open(entry, "127.0.0.1", origin_port).await;
    s.write_all(b"hello-mode-a").await.unwrap();
    let mut got = [0u8; 12];
    s.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello-mode-a");

    // The engine saw exactly the port's identity, in the RFC 1929 dialect.
    let users = es.socks_users.lock();
    let passes = es.socks_passes.lock();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0], expected_identity(entry));
    assert_eq!(passes[0], "t0k");
    assert_eq!(es.tunnels.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mode_a_http_connect_injects_basic_and_shields_client_credential() {
    let (origin_port, _os) = spawn_origin(OriginMode::Echo).await;
    let (engine_port, es) = spawn_engine(origin_port).await;
    let entry = free_port().await;
    let _f = boot_shell_forwarder(entry, engine_port, "mixed").await;

    let mut s = TcpStream::connect(("127.0.0.1", entry)).await.unwrap();
    // Client presents its OWN credential - it must never reach the engine.
    let req = "CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\nHost: 127.0.0.1:{origin_port}\r\nProxy-Authorization: Basic c3VwZXJWaXNpb24=\r\n\r\n"
        .replace("{origin_port}", &origin_port.to_string());
    s.write_all(req.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    loop {
        let mut t = [0u8; 256];
        let n = s.read(&mut t).await.unwrap();
        if n == 0 {
            break;
        }
        head.extend_from_slice(&t[..n]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head).to_string();
    assert!(head.starts_with("HTTP/1.1 200"), "got {head}");

    let want = format!(
        "Proxy-Authorization: {}",
        basic_proxy_auth(&expected_identity(entry), "t0k")
    );
    let seen = es.auth_headers.lock();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0], want,
        "injected Basic identity, not the client credential"
    );
    assert!(!seen[0].contains("c3VwZXJWaXNpb24"));
    assert_eq!(es.tunnels.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn absolute_form_never_reaches_the_engine_forward_path() {
    // The ticket-04 defect: Resin's forward-GET copies without flush, so a
    // proxied SSE arrives in 2-4 KB batches. Mode A neutralises it by
    // re-tunnelling absolute-form through CONNECT: the engine's forward path
    // is never entered, and the origin sees an origin-form request.
    let (origin_port, os) = spawn_origin(OriginMode::HttpOnce).await;
    let (engine_port, es) = spawn_engine(origin_port).await;
    let entry = free_port().await;
    let _f = boot_shell_forwarder(entry, engine_port, "mixed").await;

    let mut s = TcpStream::connect(("127.0.0.1", entry)).await.unwrap();
    let req = "GET http://127.0.0.1:{origin_port}/ping HTTP/1.1\r\nHost: 127.0.0.1:{origin_port}\r\nAccept: text/event-stream\r\nProxy-Authorization: Basic ZGVhZGJlZWY=\r\n\r\n"
        .replace("{origin_port}", &origin_port.to_string());
    s.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    for _ in 0..20 {
        let mut t = [0u8; 512];
        match tokio::time::timeout(Duration::from_secs(3), s.read(&mut t)).await {
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => resp.extend_from_slice(&t[..n]),
            Err(_) => break,
        }
        if resp.windows(4).any(|w| w == b"PONG") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&resp).to_string();
    assert!(text.contains("200"), "client got: {text}");
    assert!(text.contains("PONG"), "client got: {text}");

    let lines = es.request_lines.lock();
    assert!(
        lines.iter().all(|l| l.starts_with("CONNECT ")),
        "engine must only ever see CONNECT from the forwarder, got {lines:?}"
    );
    let lr = os.last_request.lock();
    let req_text = String::from_utf8_lossy(&lr).to_string();
    assert!(
        req_text.starts_with("GET /ping HTTP/1.1"),
        "origin got {req_text}"
    );
    assert!(req_text.contains("Accept: text/event-stream"));
    assert!(
        !req_text.contains("Proxy-Authorization"),
        "origin got {req_text}"
    );
}

#[tokio::test]
async fn dialect_gate_mirrors_the_engine_refusals() {
    // http-only entry port: a SOCKS5 greeting is refused with 05 FF, exactly
    // like a tightened http-only Resin endpoint did at the D4 gate.
    let (origin_port, _os) = spawn_origin(OriginMode::Echo).await;
    let (engine_port, _es) = spawn_engine(origin_port).await;
    let entry_http = free_port().await;
    let _f1 = boot_shell_forwarder(entry_http, engine_port, "http").await;
    let mut s = TcpStream::connect(("127.0.0.1", entry_http)).await.unwrap();
    s.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
    let mut rep = [0u8; 2];
    s.read_exact(&mut rep).await.unwrap();
    assert_eq!(rep, [0x05, 0xFF], "http-only port must refuse SOCKS5");

    // socks5-only entry port: an HTTP request gets the 403 + capability
    // header the D4 gate observed from Resin's socks5-only endpoint.
    let entry_socks = free_port().await;
    let _f2 = boot_shell_forwarder(entry_socks, engine_port, "socks5").await;
    let mut s = TcpStream::connect(("127.0.0.1", entry_socks))
        .await
        .unwrap();
    s.write_all(b"GET http://example.invalid/ HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut head = Vec::new();
    for _ in 0..10 {
        let mut t = [0u8; 256];
        match tokio::time::timeout(Duration::from_secs(2), s.read(&mut t)).await {
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => head.extend_from_slice(&t[..n]),
            Err(_) => break,
        }
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head).to_string();
    assert!(head.contains("403"), "socks5-only port got {head}");
    assert!(
        head.contains("ENDPOINT_CAPABILITY_DISABLED"),
        "socks5-only port got {head}"
    );
}

#[tokio::test]
async fn socks5_upstream_dial_failure_answers_in_band() {
    // Engine port closed: the client must get a protocol-level failure
    // reply, not a hang and not a bare close.
    let dead_engine = free_port().await;
    let entry = free_port().await;
    let _f = boot_shell_forwarder(entry, dead_engine, "mixed").await;
    let mut s = TcpStream::connect(("127.0.0.1", entry)).await.unwrap();
    s.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
    let mut rep = [0u8; 2];
    s.read_exact(&mut rep).await.unwrap();
    assert_eq!(rep, [0x05, 0x00]);
    let mut req = vec![0x05, 0x01, 0x00, 0x03, 9];
    req.extend_from_slice(b"127.0.0.1");
    req.extend_from_slice(&80u16.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut rh = [0u8; 2];
    tokio::time::timeout(Duration::from_secs(6), s.read_exact(&mut rh))
        .await
        .expect("must answer, not hang")
        .unwrap();
    assert_eq!(rh[0], 0x05);
    assert_eq!(rh[1], 0x01, "general-failure reply, in-band");
}

// ---------------------------------------------------------------------------
// D-004 SSE: the four hard behavioural assertions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_flushes_per_event_through_the_forwarder() {
    // Assertion 1 (per-event flush): origin writes 5 events 40 ms apart;
    // the client must see them arrive spread over time, never batched.
    let (origin_port, _os) = spawn_origin(OriginMode::SsePaced).await;
    let (engine_port, _es) = spawn_engine(origin_port).await;
    let entry = free_port().await;
    let _f = boot_shell_forwarder(entry, engine_port, "mixed").await;

    let mut s = TcpStream::connect(("127.0.0.1", entry)).await.unwrap();
    let req = "GET http://127.0.0.1:{origin_port}/v1/stream HTTP/1.1\r\nAccept: text/event-stream\r\n\r\n"
        .replace("{origin_port}", &origin_port.to_string());
    s.write_all(req.as_bytes()).await.unwrap();

    let mut arrivals: Vec<Instant> = Vec::new();
    let mut acc = Vec::new();
    let mut seen = 0usize;
    let t0 = Instant::now();
    while seen < 5 && t0.elapsed() < Duration::from_secs(5) {
        let mut t = [0u8; 1024];
        let n = match tokio::time::timeout(Duration::from_millis(1500), s.read(&mut t)).await {
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => n,
            Err(_) => break,
        };
        acc.extend_from_slice(&t[..n]);
        let complete = count_complete_events(&acc);
        while seen < complete {
            arrivals.push(Instant::now());
            seen += 1;
        }
    }
    assert_eq!(
        arrivals.len(),
        5,
        "five events must arrive individually, saw {}",
        arrivals.len()
    );
    let spread = arrivals[4] - arrivals[0];
    // 4 gaps x 40 ms = 160 ms written; a batched relay would show ~0.
    assert!(
        spread >= Duration::from_millis(80),
        "arrival spread {spread:?} proves batching"
    );
}

/// A complete event is the 10-byte frame "data: e<N>\n\n".
fn count_complete_events(buf: &[u8]) -> usize {
    let mut c = 0;
    let mut i = 0;
    while i + 10 <= buf.len() {
        if &buf[i..i + 7] == b"data: e"
            && buf[i + 7].is_ascii_digit()
            && &buf[i + 8..i + 10] == b"\n\n"
        {
            c += 1;
            i += 10;
        } else {
            i += 1;
        }
    }
    c
}

#[tokio::test]
async fn client_abort_cascades_to_upstream_cancel() {
    // Assertion 2: dropping the client mid-SSE must tear the upstream down
    // so the engine cancels the origin request instead of burning tokens.
    let (origin_port, os) = spawn_origin(OriginMode::StreamForever).await;
    let (engine_port, _es) = spawn_engine(origin_port).await;
    let entry = free_port().await;
    let _f = boot_shell_forwarder(entry, engine_port, "mixed").await;

    let s = socks_open(entry, "127.0.0.1", origin_port).await;
    // Let the stream run briefly, then abort hard.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        os.accepted.load(Ordering::SeqCst),
        1,
        "origin connected through the tunnel"
    );
    drop(s);

    for _ in 0..150 {
        if os.peer_gone_at.lock().is_some() {
            return; // cancelled within 3 s
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("origin never observed the disconnect - the cancel cascade did not propagate");
}

#[tokio::test]
async fn slow_client_backpressure_is_bounded() {
    // Assertion 3: with an idle client the relay must NOT absorb the whole
    // payload - the origin's write progress must stall quickly, proving the
    // only buffering in the chain is socket-sized, not payload-sized.
    let (origin_port, os) = spawn_origin(OriginMode::Flood).await;
    let (engine_port, _es) = spawn_engine(origin_port).await;
    let entry = free_port().await;
    let _f = boot_shell_forwarder(entry, engine_port, "mixed").await;

    let s = socks_open(entry, "127.0.0.1", origin_port).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let written = os.bytes_written.load(Ordering::SeqCst);
    let done = os.done_at.lock().is_some();
    // The 16 MiB body may not be "finished" into shell-side buffers while
    // the client read nothing: allow socket-sized slack only.
    assert!(
        !done,
        "flood completed against an idle client - unbounded buffering"
    );
    assert!(
        written < 4 * 1024 * 1024,
        "idle-client chain absorbed {written} bytes (> 4 MiB is not socket-sized)"
    );
    drop(s);
}

#[tokio::test]
async fn sse_upstream_reset_gets_in_band_error_frame() {
    // Assertion 4: an event stream that breaks abnormally must terminate
    // with an in-band error event, not a silently-truncated body.
    let (origin_port, _os) = spawn_origin(OriginMode::SseThenReset).await;
    let (engine_port, _es) = spawn_engine(origin_port).await;
    let entry = free_port().await;
    let _f = boot_shell_forwarder(entry, engine_port, "mixed").await;

    let mut s = TcpStream::connect(("127.0.0.1", entry)).await.unwrap();
    let req = "GET http://127.0.0.1:{origin_port}/v1/stream HTTP/1.1\r\nAccept: text/event-stream\r\n\r\n"
        .replace("{origin_port}", &origin_port.to_string());
    s.write_all(req.as_bytes()).await.unwrap();
    let mut acc = Vec::new();
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(4) {
        let mut t = [0u8; 2048];
        match tokio::time::timeout(Duration::from_millis(1200), s.read(&mut t)).await {
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => acc.extend_from_slice(&t[..n]),
            Err(_) => break,
        }
        if String::from_utf8_lossy(&acc).contains("upstream_aborted") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&acc).to_string();
    assert!(text.contains("data: e0"), "client got: {text}");
    assert!(text.contains("event: error"), "client got: {text}");
    assert!(text.contains("upstream_aborted"), "client got: {text}");
}

// ---------------------------------------------------------------------------
// A-015: p95 added-latency initial check (mock upstream full chain)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn paired_request_added_latency_p95_under_5ms() {
    // D-004 item: full-chain added latency p95 <= 5 ms measured against a
    // mock upstream, paired per iteration (direct vs via-forwarder) so host
    // noise cancels out. Initial value pending calibration after CI runs.
    let (origin_port, _os) = spawn_origin(OriginMode::Echo).await;
    let (engine_port, _es) = spawn_engine(origin_port).await;
    let entry = free_port().await;
    let _f = boot_shell_forwarder(entry, engine_port, "mixed").await;

    let direct = |op: u16| async move {
        let t0 = Instant::now();
        let mut s = TcpStream::connect(("127.0.0.1", op)).await.unwrap();
        s.write_all(b"ping").await.unwrap();
        let mut b = [0u8; 4];
        s.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"ping");
        t0.elapsed()
    };
    let vias = |ep: u16, op: u16| async move {
        let t0 = Instant::now();
        let mut s = socks_open(ep, "127.0.0.1", op).await;
        s.write_all(b"ping").await.unwrap();
        let mut b = [0u8; 4];
        s.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"ping");
        t0.elapsed()
    };

    // Warm the code paths (first iterations include task-start costs).
    for _ in 0..20 {
        direct(origin_port).await;
        vias(entry, origin_port).await;
    }
    const N: usize = 200;
    let mut deltas = Vec::with_capacity(N);
    for _ in 0..N {
        let d = direct(origin_port).await;
        let v = vias(entry, origin_port).await;
        deltas.push(v.saturating_sub(d).as_secs_f64() * 1000.0);
    }
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = deltas[(N as f64 * 0.95) as usize - 1];
    let median = deltas[N / 2];
    println!("paired added latency: median {median:.3} ms, p95 {p95:.3} ms over {N}");
    assert!(
        p95 <= 5.0,
        "p95 added latency {p95:.3} ms exceeds the D-004 5 ms initial value"
    );
}

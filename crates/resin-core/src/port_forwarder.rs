//! Multi-port identity layer - the ADR-0012 listener surface, re-materialised
//! as Data-Plane Mode A (ADR-0068 D1/D3; ledger A-001).
//!
//! Each Entry Port is an identity. The mode decides which process realises it:
//!
//!   Engine (Mode B - VPS/headless default): Resin binds every entry port
//!     natively and the client supplies the Platform.Account credential once.
//!     The shell binds nothing - exactly the thin shape ADR-0015 left behind.
//!
//!   Shell (Mode A - desktop default): the shell binds every enabled entry
//!     port, the client connects credential-free, and every connection is
//!     relayed to Resin's consolidated proxy port (resin_addr) with the
//!     port's (platform, account) injected in the dialect the connection
//!     speaks (ADR-0068 D3):
//!       SOCKS5: RFC 1929 username-password subnegotiation,
//!         username = Platform.Account, password = the sidecar proxy token.
//!       HTTP:   Proxy-Authorization: Basic base64(Platform.Account:token),
//!         with absolute-form requests re-tunnelled through CONNECT so the
//!         relayed bytes never traverse the engine's buffered forward path.
//!         That path copies a proxied response with io.Copy and NO flush, so
//!         a text/event-stream body arrives in 2-4 KB batches; the CONNECT tunnel copies
//!         raw bytes and per-event flush survives.
//!
//! The connection's first byte picks the dialect (0x05 = SOCKS5, otherwise
//! HTTP, detect_protocol). The port's declared protocol (entry_protocol:
//! mixed | http | socks5) gates what is ACCEPTED here, because the
//! consolidated port accepts both dialects and will not refuse the wrong one
//! on the forwarder's behalf. The refusals mirror
//! the per-endpoint behaviour the ADR-0068 D4 gate observed live: a SOCKS5
//! client on an http-only port gets a 05 FF method rejection, an HTTP client
//! on a socks5-only port gets 403 + X-Resin-Error: ENDPOINT_CAPABILITY_DISABLED.
//!
//! Relay semantics (D-004 SSE hard acceptance, tested in
//! tests/forwarder_dataplane.rs):
//!   - every chunk read is written and flushed before the next read: no
//!     internal batching, per-event flush end to end;
//!   - either side closing shuts the session down immediately: a client
//!     disconnect cascades into the upstream so the engine cancels the origin
//!     request instead of burning tokens;
//!   - one fixed buffer per direction per session: a slow peer applies
//!     backpressure through the sockets instead of buffering unbounded;
//!   - errors are signalled inside the client's own channel: a failed
//!     CONNECT gets a SOCKS5 failure reply / an HTTP 502, and an
//!     event-stream response that breaks abnormally (upstream reset, not a
//!     clean EOF) is terminated in-band with an event:error frame.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::db::{DbPool, PortMapping};
use crate::stream_sensor::{StreamSensor, StreamSensorSnapshot};

/// Max concurrent entry ports the shell will bind (industrial safety).
pub const MAX_ENTRY_PORTS: usize = 256;
/// Reserved / privileged ports rejected at the IPC boundary.
pub const MIN_USER_PORT: u16 = 1024;
/// Fixed per-direction relay buffer. The relay never allocates per byte of
/// in-flight data: a slow peer blocks the pump and the TCP windows carry the
/// backpressure (D-004 bounded-backpressure assertion).
pub const RELAY_BUF_BYTES: usize = 16 * 1024;
/// Cap on a request / response head read off either side.
pub const MAX_HEAD_BYTES: usize = 64 * 1024;
/// Dial timeout toward the consolidated Resin port (loopback; a timeout
/// means the sidecar is not accepting).
const UPSTREAM_DIAL_MS: u64 = 5_000;
/// SOCKS5 greeting+auth+CONNECT handshake timeout toward the engine.
const UPSTREAM_HANDSHAKE_MS: u64 = 5_000;
/// First backoff while an entry-port bind is failing (port held).
const BIND_RETRY_MIN_MS: u64 = 1_000;
/// Backoff ceiling for a persistently-held entry port.
const BIND_RETRY_MAX_MS: u64 = 30_000;

/// Which process realises the port = identity data plane (ADR-0068 D1).
/// Engine = Mode B (Resin listens, client credential once).
/// Shell  = Mode A (shell listens, client credential-free).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataPlaneMode {
    Engine,
    Shell,
}

/// Build the Resin V1 identity string Platform.Account.
/// Account defaults to port-<n> when empty so two ports never collapse.
pub fn resin_identity(platform_name: &str, account: &str, port: u16) -> String {
    let platform = platform_name.trim();
    let platform = if platform.is_empty() {
        "Default"
    } else {
        platform
    };
    let account = account.trim();
    let account = if account.is_empty() {
        format!("port-{port}")
    } else {
        account.to_string()
    };
    format!("{platform}.{account}")
}

/// First-byte protocol detect: 0x05 = SOCKS5, otherwise treat as HTTP.
pub fn detect_protocol(first: u8) -> &'static str {
    if first == 0x05 {
        "socks5"
    } else {
        "http"
    }
}

/// Whether a connection whose first byte sniffed as sniffed_dialect may use
/// an entry port declaring port_protocol. mixed (and the total fallback for
/// an out-of-set token, which every boundary rejects before storage) accepts
/// both dialects; single-protocol values accept only their own.
pub fn dialect_allowed(port_protocol: &str, sniffed_dialect: &str) -> bool {
    match crate::entry_protocol::canonical_protocol(port_protocol) {
        Some("http") => sniffed_dialect == "http",
        Some("socks5") => sniffed_dialect == "socks5",
        _ => true,
    }
}

/// Proxy-Authorization value injected toward Resin for an identity: the
/// V1 credential pair identity:proxy_token base64-encoded (Basic scheme,
/// standard alphabet with padding — base64 crate).
pub fn basic_proxy_auth(identity: &str, proxy_token: &str) -> String {
    let raw = format!("{identity}:{proxy_token}");
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
    )
}

/// Runtime port-identity + stream-sensor holder + (Mode A) the per-port
/// accept loops.
#[derive(Clone)]
pub struct PortForwarder {
    inner: Arc<PortForwarderInner>,
}

struct PortForwarderInner {
    db: DbPool,
    resin_host: String,
    resin_port: u16,
    proxy_token: String,
    mode: DataPlaneMode,
    stream_sensor: StreamSensor,
    /// Shell mode: port -> join handle of its accept task (a task that may
    /// still be retrying a failed bind counts as running, so reload does not
    /// double-spawn it).
    running: Mutex<HashMap<u16, JoinHandle<()>>>,
    /// Shell mode: the ports whose listener is actually bound right now.
    /// The authoritative snapshot unions this with the Resin endpoint list
    /// so a shell-listened entry port reads Consistent.
    bound: Arc<Mutex<HashSet<u16>>>,
}

impl PortForwarder {
    /// Mode B constructor (headless default and every legacy call
    /// site): binds nothing, keeps the ADR-0015 thin behaviour.
    pub fn new(
        db: DbPool,
        resin_host: impl Into<String>,
        resin_port: u16,
        proxy_token: impl Into<String>,
    ) -> Self {
        Self::with_mode(db, resin_host, resin_port, proxy_token, DataPlaneMode::Engine)
    }

    /// Mode A constructor (desktop default): reload() binds one accept loop
    /// per enabled entry port and every connection is relayed to the
    /// consolidated engine port with the port's identity injected.
    pub fn shell(
        db: DbPool,
        resin_host: impl Into<String>,
        resin_port: u16,
        proxy_token: impl Into<String>,
    ) -> Self {
        Self::with_mode(db, resin_host, resin_port, proxy_token, DataPlaneMode::Shell)
    }

    fn with_mode(
        db: DbPool,
        resin_host: impl Into<String>,
        resin_port: u16,
        proxy_token: impl Into<String>,
        mode: DataPlaneMode,
    ) -> Self {
        let inner = Arc::new(PortForwarderInner {
            db,
            resin_host: resin_host.into(),
            resin_port,
            proxy_token: proxy_token.into(),
            mode,
            stream_sensor: StreamSensor::new(),
            running: Mutex::new(HashMap::new()),
            bound: Arc::new(Mutex::new(HashSet::new())),
        });
        Self { inner }
    }

    pub fn mode(&self) -> DataPlaneMode {
        self.inner.mode
    }

    /// Whether the shell owns the entry-port listeners (Mode A).
    pub fn is_shell(&self) -> bool {
        self.inner.mode == DataPlaneMode::Shell
    }

    /// Engine address (for IPC layer to build ResinClient; Mode A relays
    /// proxy bytes to the same host:port).
    pub fn resin_addr(&self) -> String {
        format!("http://{}:{}", self.inner.resin_host, self.inner.resin_port)
    }

    pub fn proxy_token(&self) -> &str {
        &self.inner.proxy_token
    }

    /// Header-only AI stream observations from plain HTTP proxy traffic.
    pub fn stream_snapshot(&self) -> StreamSensorSnapshot {
        self.inner.stream_sensor.snapshot()
    }

    /// Mode view of the live entry ports:
    ///   Shell:  the ports whose accept loop currently holds a bound
    ///           listener (a retrying bind is NOT yet serving, so it is
    ///           honestly absent - the snapshot reads it MissingOnResin).
    ///   Engine: the shell-side PortMapping metadata view the ADR-0015
    ///           thin shell kept (the real listeners live in Resin).
    pub fn running_ports(&self) -> Vec<u16> {
        if self.is_shell() {
            let mut v: Vec<u16> = self.inner.bound.lock().iter().copied().collect();
            v.sort_unstable();
            v
        } else {
            self.inner
                .db
                .list_ports()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m.enabled)
                .map(|m| m.port)
                .collect()
        }
    }

    /// Reconcile the accept loops with an already-validated port row set
    /// (the whitebox apply transaction calls this; Engine mode no-ops).
    /// Disabled and removed rows stop their loop; enabled rows without a
    /// loop get one spawned. A bind failure (port held) does not fail the
    /// call - the task keeps retrying with capped backoff while the
    /// snapshot shows the port missing until it lands.
    pub async fn reload(&self, rows: &[PortMapping]) -> Result<usize, String> {
        if !self.is_shell() {
            return Ok(0);
        }
        if rows.iter().filter(|m| m.enabled).count() > MAX_ENTRY_PORTS {
            return Err(format!("too many enabled entry ports (max {MAX_ENTRY_PORTS})"));
        }
        let enabled: HashMap<u16, PortMapping> = rows
            .iter()
            .filter(|m| m.enabled)
            .map(|m| (m.port, m.clone()))
            .collect();

        let to_stop: Vec<u16> = {
            let running = self.inner.running.lock();
            running.keys().copied().filter(|p| !enabled.contains_key(p)).collect()
        };
        for p in to_stop {
            let handle = self.inner.running.lock().remove(&p);
            if let Some(h) = handle {
                h.abort();
            }
            self.inner.bound.lock().remove(&p);
            tracing::info!(port = p, "forwarder: entry listener stopped");
        }

        let mut started = 0usize;
        for (port, m) in enabled {
            let already = self.inner.running.lock().contains_key(&port);
            if already {
                continue;
            }
            self.spawn_port(m);
            started += 1;
        }
        Ok(started)
    }

    /// Convenience reload straight from the shell DB (restore / reconcile
    /// paths that do not hold the row set).
    pub async fn reload_from_db(&self) -> Result<usize, String> {
        let rows = self.inner.db.list_ports()?;
        self.reload(&rows).await
    }

    fn spawn_port(&self, m: PortMapping) {
        let host = self.inner.resin_host.clone();
        let port = self.inner.resin_port;
        let token = self.inner.proxy_token.clone();
        let bound = self.inner.bound.clone();
        let identity = resin_identity(&m.platform_name, &m.account, m.port);
        let declared = m.protocol.clone();
        let entry_port = m.port;
        let handle = tokio::spawn(async move {
            accept_loop(entry_port, identity, declared, host, port, token, bound).await;
        });
        self.inner.running.lock().insert(entry_port, handle);
    }

    /// Abort every accept loop (app exit).
    pub fn shutdown(&self) {
        let mut running = self.inner.running.lock();
        for (p, h) in running.drain() {
            h.abort();
            self.inner.bound.lock().remove(&p);
            tracing::info!(port = p, "forwarder: shutdown abort");
        }
    }
}

/// One entry port's lifetime: bind (retrying while held), then accept until
/// the task is aborted by reload/shutdown.
async fn accept_loop(
    entry_port: u16,
    identity: String,
    declared: String,
    resin_host: String,
    resin_port: u16,
    proxy_token: String,
    bound: Arc<Mutex<HashSet<u16>>>,
) {
    let addr: std::net::SocketAddr = match format!("127.0.0.1:{entry_port}").parse() {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(port = entry_port, error = %e, "forwarder: bad bind addr");
            return;
        }
    };
    let mut retry = Duration::from_millis(BIND_RETRY_MIN_MS);
    let listener = loop {
        match TcpListener::bind(addr).await {
            Ok(l) => break l,
            Err(e) => {
                tracing::warn!(
                    port = entry_port,
                    error = %e,
                    retry_ms = retry.as_millis(),
                    "forwarder: entry bind failed; backing off"
                );
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(Duration::from_millis(BIND_RETRY_MAX_MS));
            }
        }
    };
    bound.lock().insert(entry_port);
    tracing::info!(
        port = entry_port,
        identity = %identity,
        protocol = %declared,
        "forwarder: entry listening (mode A)"
    );
    loop {
        match listener.accept().await {
            Ok((client, peer)) => {
                let ctx = SessionCtx {
                    identity: identity.clone(),
                    declared: declared.clone(),
                    resin_host: resin_host.clone(),
                    resin_port,
                    proxy_token: proxy_token.clone(),
                };
                tokio::spawn(async move {
                    if let Err(e) = handle_client(client, peer, ctx).await {
                        tracing::debug!(port = entry_port, error = %e, "forwarder: session ended");
                    }
                });
            }
            Err(e) => {
                tracing::warn!(port = entry_port, error = %e, "forwarder: accept error");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Per-connection context: what to inject and where to relay.
struct SessionCtx {
    identity: String,
    declared: String,
    resin_host: String,
    resin_port: u16,
    proxy_token: String,
}

async fn handle_client(
    mut client: TcpStream,
    peer: std::net::SocketAddr,
    ctx: SessionCtx,
) -> Result<(), String> {
    let _ = peer;
    let mut first = [0u8; 1];
    client
        .read_exact(&mut first)
        .await
        .map_err(|e| format!("read first byte: {e}"))?;
    let sniffed = detect_protocol(first[0]);
    if !dialect_allowed(&ctx.declared, sniffed) {
        return refuse_dialect(&mut client, sniffed).await;
    }
    if sniffed == "socks5" {
        handle_socks5(client, ctx).await
    } else {
        handle_http(client, first[0], ctx).await
    }
}

/// Protocol-faithful refusal of the wrong dialect (mirrors what a tightened
/// single-protocol Resin endpoint answers, ADR-0068 D4 observation).
async fn refuse_dialect(client: &mut TcpStream, sniffed: &str) -> Result<(), String> {
    let out: &[u8] = if sniffed == "socks5" {
        &[0x05u8, 0xFFu8][..]
    } else {
        b"HTTP/1.1 403 Forbidden\r\nX-Resin-Error: ENDPOINT_CAPABILITY_DISABLED\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    };
    let _ = client.write_all(out).await;
    let _ = client.flush().await;
    Err(format!("dialect refused: {sniffed}"))
}

async fn handle_socks5(mut client: TcpStream, ctx: SessionCtx) -> Result<(), String> {
    // Greeting: VER(0x05 already read) NMETHODS METHODS...
    let mut nmethods = [0u8; 1];
    client
        .read_exact(&mut nmethods)
        .await
        .map_err(|e| format!("socks nmethods: {e}"))?;
    let mut methods = vec![0u8; nmethods[0] as usize];
    if !methods.is_empty() {
        client
            .read_exact(&mut methods)
            .await
            .map_err(|e| format!("socks methods: {e}"))?;
    }
    // Mode A: the client presents NO credential - the port is the identity.
    // Reply NoAuth regardless of the offered methods (0x00 is always a
    // selection this listener honours).
    client
        .write_all(&[0x05, 0x00])
        .await
        .map_err(|e| format!("socks method reply: {e}"))?;
    client.flush().await.map_err(|e| e.to_string())?;

    // Request: VER CMD RSV ATYP DST.ADDR DST.PORT
    let mut hdr = [0u8; 4];
    client
        .read_exact(&mut hdr)
        .await
        .map_err(|e| format!("socks request: {e}"))?;
    if hdr[0] != 0x05 {
        return Err("socks bad ver in request".into());
    }
    if hdr[1] != 0x01 {
        // CONNECT only: BIND (0x02) and UDP_ASSOCIATE (0x03) get the
        // command-unsupported reply. D-001 scope is the HTTPS+SSE workload.
        let _ = client.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
        return Err("socks cmd not CONNECT".into());
    }
    let (host, port) = read_socks_addr(&mut client, hdr[3]).await?;

    let upstream = match tokio::time::timeout(
        Duration::from_millis(UPSTREAM_HANDSHAKE_MS),
        socks5_connect_authed(&ctx, &host, port),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            // In-band signalling: a general-failure reply, not a hang.
            let _ = client.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
            let _ = client.flush().await;
            return Err(format!("socks upstream: {e}"));
        }
        Err(_) => {
            let _ = client.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
            let _ = client.flush().await;
            return Err("socks upstream handshake timeout".into());
        }
    };

    // Success reply (bind 0.0.0.0:0).
    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|e| format!("socks connect reply: {e}"))?;
    client.flush().await.map_err(|e| e.to_string())?;
    relay_pair(client, upstream).await;
    Ok(())
}

async fn read_socks_addr(stream: &mut TcpStream, atyp: u8) -> Result<(String, u16), String> {
    match atyp {
        0x01 => {
            let mut ip = [0u8; 4];
            stream.read_exact(&mut ip).await.map_err(|e| e.to_string())?;
            let mut p = [0u8; 2];
            stream.read_exact(&mut p).await.map_err(|e| e.to_string())?;
            Ok((format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]), u16::from_be_bytes(p)))
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await.map_err(|e| e.to_string())?;
            let mut name = vec![0u8; len[0] as usize];
            stream.read_exact(&mut name).await.map_err(|e| e.to_string())?;
            let mut p = [0u8; 2];
            stream.read_exact(&mut p).await.map_err(|e| e.to_string())?;
            let host = String::from_utf8(name).map_err(|e| e.to_string())?;
            Ok((host, u16::from_be_bytes(p)))
        }
        0x04 => {
            let mut ip = [0u8; 16];
            stream.read_exact(&mut ip).await.map_err(|e| e.to_string())?;
            let mut p = [0u8; 2];
            stream.read_exact(&mut p).await.map_err(|e| e.to_string())?;
            let segs: Vec<String> = ip
                .chunks(2)
                .map(|c| format!("{:x}", u16::from_be_bytes([c[0], c[1]])))
                .collect();
            Ok((segs.join(":"), u16::from_be_bytes(p)))
        }
        _ => Err(format!("unsupported atyp {atyp}")),
    }
}

async fn read_socks_reply_addr(stream: &mut TcpStream) -> Result<(), String> {
    let mut rh = [0u8; 4];
    stream.read_exact(&mut rh).await.map_err(|e| e.to_string())?;
    if rh[1] != 0x00 {
        return Err(format!("upstream socks CONNECT failed status={}", rh[1]));
    }
    let _ = read_socks_addr(stream, rh[3]).await?;
    Ok(())
}

/// Authed SOCKS5 CONNECT toward the consolidated engine port: RFC 1929
/// username-password subnegotiation carrying the port's identity
/// (ADR-0068 D3 dialect injection).
async fn socks5_connect_authed(
    ctx: &SessionCtx,
    dest_host: &str,
    dest_port: u16,
) -> Result<TcpStream, String> {
    let mut s = dial_upstream(ctx).await?;
    s.write_all(&[0x05, 0x01, 0x02])
        .await
        .map_err(|e| format!("socks offer: {e}"))?;
    let mut resp = [0u8; 2];
    s.read_exact(&mut resp).await.map_err(|e| format!("socks method select: {e}"))?;
    if resp[0] != 0x05 || resp[1] != 0x02 {
        return Err(format!("socks auth method rejected: {:02x?}", resp));
    }
    let user = ctx.identity.as_bytes();
    let pass = ctx.proxy_token.as_bytes();
    if user.len() > 255 || pass.len() > 255 {
        return Err("identity/token too long for socks5".into());
    }
    let mut auth = Vec::with_capacity(3 + user.len() + pass.len());
    auth.push(0x01);
    auth.push(user.len() as u8);
    auth.extend_from_slice(user);
    auth.push(pass.len() as u8);
    auth.extend_from_slice(pass);
    s.write_all(&auth).await.map_err(|e| format!("socks auth send: {e}"))?;
    let mut auth_resp = [0u8; 2];
    s.read_exact(&mut auth_resp).await.map_err(|e| format!("socks auth reply: {e}"))?;
    if auth_resp[1] != 0x00 {
        return Err(format!("socks auth failed: {:02x?}", auth_resp));
    }
    let host_b = dest_host.as_bytes();
    if host_b.len() > 255 {
        return Err("dest host too long".into());
    }
    let mut req = Vec::with_capacity(5 + host_b.len() + 2);
    req.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host_b.len() as u8]);
    req.extend_from_slice(host_b);
    req.extend_from_slice(&dest_port.to_be_bytes());
    s.write_all(&req).await.map_err(|e| format!("socks connect send: {e}"))?;
    read_socks_reply_addr(&mut s).await?;
    Ok(s)
}

async fn dial_upstream(ctx: &SessionCtx) -> Result<TcpStream, String> {
    tokio::time::timeout(
        Duration::from_millis(UPSTREAM_DIAL_MS),
        TcpStream::connect((ctx.resin_host.as_str(), ctx.resin_port)),
    )
    .await
    .map_err(|_| "upstream dial timeout".to_string())?
    .map_err(|e| format!("upstream dial: {e}"))
}

async fn handle_http(mut client: TcpStream, first: u8, ctx: SessionCtx) -> Result<(), String> {
    let mut buf = vec![first];
    read_head_into(&mut client, &mut buf).await?;
    let head = String::from_utf8_lossy(&buf).to_string();
    let header_end = head
        .find("\r\n\r\n")
        .ok_or_else(|| "no head terminator".to_string())?
        + 4;
    let body_tail = buf[header_end..].to_vec();
    let mut lines: Vec<String> = head[..header_end]
        .split("\r\n")
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .collect();
    let request_line = lines.first().ok_or_else(|| "empty http".to_string())?.clone();
    let mut parts = request_line.splitn(3, ' ');
    let (method, target) = match (parts.next(), parts.next()) {
        (Some(m), Some(t)) => (m.to_ascii_uppercase(), t.to_string()),
        _ => return Err("malformed request line".into()),
    };
    lines.remove(0);

    if method == "CONNECT" {
        // Validate the authority (port parse) then relay the original
        // target string verbatim - the engine does the dial.
        let (_host, _port) = split_authority(&target)?;
        let mut upstream = dial_upstream(&ctx).await?;
        let auth = basic_proxy_auth(&ctx.identity, &ctx.proxy_token);
        let req = format!(
            "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Authorization: {auth}\r\n\r\n"
        );
        upstream
            .write_all(req.as_bytes())
            .await
            .map_err(|e| format!("connect send: {e}"))?;
        upstream.flush().await.map_err(|e| e.to_string())?;
        let mut rbuf = Vec::new();
        read_head_into(&mut upstream, &mut rbuf).await?;
        let resp_head = String::from_utf8_lossy(&rbuf).to_string();
        let ok = response_head_is_2xx(&resp_head);
        // Relay the engine CONNECT verdict verbatim: 2xx opens the tunnel,
        // a 4xx/5xx already carries the engine error face in-band.
        client
            .write_all(&rbuf)
            .await
            .map_err(|e| format!("connect reply: {e}"))?;
        client.flush().await.map_err(|e| e.to_string())?;
        if !ok {
            return Err(format!("engine refused CONNECT: {}", first_line(&resp_head)));
        }
        relay_pair(client, upstream).await;
        return Ok(());
    }

    // Absolute-form plain-HTTP request. Tunnel it through CONNECT and replay
    // origin-form: the engine forward path would buffer the response without
    // flushing, the tunnel copies raw bytes.
    let (host, port, path) = match split_absolute_form(&target)? {
        Some(v) => v,
        None => {
            // Origin-form (not proxy traffic) or https:// absolute-form
            // (a CONNECT is the correct client behaviour there).
            return http_error(&mut client, 400, "bad proxy request target").await;
        }
    };
    let mut upstream = match dial_upstream(&ctx).await {
        Ok(s) => s,
        Err(e) => return http_error(&mut client, 502, &format!("upstream dial: {e}")).await,
    };
    let authority = format!("{host}:{port}");
    let auth = basic_proxy_auth(&ctx.identity, &ctx.proxy_token);
    let creq = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: {auth}\r\n\r\n"
    );
    let opened = async {
        upstream
            .write_all(creq.as_bytes())
            .await
            .map_err(|e| format!("connect send: {e}"))?;
        upstream.flush().await.map_err(|e| e.to_string())?;
        let mut rbuf = Vec::new();
        read_head_into(&mut upstream, &mut rbuf).await?;
        let resp_head = String::from_utf8_lossy(&rbuf).to_string();
        if response_head_is_2xx(&resp_head) {
            Ok(())
        } else {
            Err(format!("engine refused CONNECT: {}", first_line(&resp_head)))
        }
    }
    .await;
    if let Err(e) = opened {
        return http_error(&mut client, 502, &e).await;
    }

    let replayed = build_replayed_head(&mut lines, &method, &path, &authority);
    upstream
        .write_all(replayed.as_bytes())
        .await
        .map_err(|e| format!("replay head: {e}"))?;
    if !body_tail.is_empty() {
        upstream
            .write_all(&body_tail)
            .await
            .map_err(|e| format!("replay body: {e}"))?;
    }
    upstream.flush().await.map_err(|e| e.to_string())?;

    // Response head first, so the in-band error policy can branch on the
    // negotiated content type.
    let mut hbuf = Vec::new();
    if read_head_into(&mut upstream, &mut hbuf).await.is_err() {
        return http_error(&mut client, 502, "upstream closed before response head").await;
    }
    let resp = String::from_utf8_lossy(&hbuf).to_string();
    let event_stream = head_has_event_stream(&resp);
    client
        .write_all(&hbuf)
        .await
        .map_err(|e| format!("forward head: {e}"))?;
    client.flush().await.map_err(|e| e.to_string())?;

    // client -> upstream: only the request body can still arrive; a close on
    // this direction is an abort and must tear the upstream side down.
    let (mut up_rd, mut up_w) = upstream.into_split();
    let (mut cl_r, mut cl_w) = client.into_split();
    let c2u = tokio::spawn(async move {
        let mut buf = vec![0u8; RELAY_BUF_BYTES];
        loop {
            match cl_r.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if up_w.write_all(&buf[..n]).await.is_err() || up_w.flush().await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = up_w.shutdown().await;
    });

    let mut buf = vec![0u8; RELAY_BUF_BYTES];
    let mut status = Ok(());
    loop {
        match up_rd.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if cl_w.write_all(&buf[..n]).await.is_err() || cl_w.flush().await.is_err() {
                    status = Err("client write failed".to_string());
                    break;
                }
            }
            Err(e) => {
                // Abnormal break, not a clean EOF: for an event stream the
                // only in-band signal still reaching the client is an error
                // frame, so emit one before closing.
                if event_stream {
                    let _ = cl_w
                        .write_all(b"event: error\r\ndata: {\"error\":\"upstream_aborted\"}\r\n\r\n")
                        .await;
                    let _ = cl_w.flush().await;
                }
                status = Err(format!("upstream read: {e}"));
                break;
            }
        }
    }
    c2u.abort();
    let _ = cl_w.shutdown().await;
    drop(up_rd);
    status
}

/// Rewrite the stored head into origin-form over the tunnel: replace the
/// absolute target with the path, drop the client (and any injected)
/// proxy-scope headers, and pin the exchange to one response per tunnel.
fn build_replayed_head(lines: &mut Vec<String>, method: &str, path: &str, authority: &str) -> String {
    let mut out_lines = vec![format!("{method} {path} HTTP/1.1")];
    let has_host = lines
        .iter()
        .any(|l| l.to_ascii_lowercase().starts_with("host:"));
    for l in lines.iter() {
        let lower = l.to_ascii_lowercase();
        if lower.starts_with("proxy-authorization:")
            || lower.starts_with("proxy-connection:")
            || lower.starts_with("connection:")
        {
            continue;
        }
        out_lines.push(l.clone());
    }
    if !has_host {
        out_lines.insert(1, format!("Host: {authority}"));
    }
    out_lines.push("Connection: close".into());
    let mut joined = out_lines.join("\r\n");
    joined.push_str("\r\n\r\n");
    joined
}

fn split_authority(target: &str) -> Result<(String, u16), String> {
    let (host, port) = match target.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| "bad CONNECT port".to_string())?),
        None => (target.to_string(), 443),
    };
    if host.is_empty() {
        return Err("empty CONNECT host".into());
    }
    Ok((host, port))
}

/// http://host[:port]/path -> (host, port, origin-form path). An empty path
/// normalises to /. https:// absolute-form targets have no replay story
/// (TLS must ride a CONNECT) and read as None, as do origin-form paths.
fn split_absolute_form(target: &str) -> Result<Option<(String, u16, String)>, String> {
    if !target.starts_with("http://") {
        return Ok(None);
    }
    let rest = &target["http://".len()..];
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = if authority.ends_with(']') {
        (authority.to_string(), 80u16)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| "bad port".to_string())?),
            None => (authority.to_string(), 80u16),
        }
    };
    if host.is_empty() {
        return Ok(None);
    }
    let path = if path.is_empty() { "/".to_string() } else { path.to_string() };
    Ok(Some((host, port, path)))
}

fn response_head_is_2xx(head: &str) -> bool {
    match head.split_once(' ') {
        Some((proto, rest))
            if proto.eq_ignore_ascii_case("HTTP/1.1") || proto.eq_ignore_ascii_case("HTTP/1.0") =>
        {
            matches!(rest.split(' ').next(), Some(s) if s.starts_with('2') && s.len() == 3)
        }
        _ => false,
    }
}

fn head_has_event_stream(head: &str) -> bool {
    head.lines().any(|l| {
        let lower = l.to_ascii_lowercase();
        lower.starts_with("content-type:") && lower.contains("text/event-stream")
    })
}

fn first_line(s: &str) -> &str {
    s.split("\r\n").next().unwrap_or(s)
}

async fn http_error(client: &mut TcpStream, status: u16, reason: &str) -> Result<(), String> {
    let body = format!("proxy error: {reason}");
    // The reason is diagnostic text echoed into a head: strip byte-level
    // protocol characters so a hostile target cannot inject response heads.
    let safe: String = body
        .chars()
        .map(|c| if matches!(c, '\r' | '\n' | '\0') { ' ' } else { c })
        .collect();
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{safe}",
        if status == 502 { "Bad Gateway" } else { "Bad Request" },
        safe.len()
    );
    let _ = client.write_all(head.as_bytes()).await;
    let _ = client.flush().await;
    Err(format!("http error {status}: {reason}"))
}

/// Read from stream into buf until the double-CRLF terminator appears or the
/// head cap / EOF is hit.
async fn read_head_into(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Result<(), String> {
    let mut tmp = [0u8; 1024];
    loop {
        if find_head_end(buf).is_some() {
            return Ok(());
        }
        if buf.len() >= MAX_HEAD_BYTES {
            return Err("http head too large".into());
        }
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("head read: {e}"))?;
        if n == 0 {
            return Err("peer closed before head end".into());
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// Bidirectional raw relay with per-chunk flush and close cascade: when
/// either direction ends (EOF or error) the opposite peer write side is
/// FINed immediately, so a client abort propagates to the engine as an
/// upstream cancel (D-004 assertion 2). Byte-level error injection is
/// impossible inside a tunnel (the payload is TLS), so both tunnels share
/// this form; the absolute-form HTTP path with the event-stream error frame
/// is handled by its own inline loop instead.
async fn relay_pair(client: TcpStream, upstream: TcpStream) {
    let (cr, cw) = client.into_split();
    let (ur, uw) = upstream.into_split();
    let a = tokio::spawn(async move { pump_direction(cr, uw).await; });
    let b = tokio::spawn(async move { pump_direction(ur, cw).await; });
    let _ = tokio::join!(a, b);
}

async fn pump_direction(mut rd: OwnedReadHalf, mut wr: OwnedWriteHalf) {
    let mut buf = vec![0u8; RELAY_BUF_BYTES];
    loop {
        match rd.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if wr.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                if wr.flush().await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    // Session half over: FIN the peer that was being written to. A blocked
    // reader on the other task then finishes on the returned close.
    let _ = wr.shutdown().await;
    let _ = rd;
}

// ---------------------------------------------------------------------------
// exit-IP probe helper (used by commands::probe_exit_ip)
// ---------------------------------------------------------------------------

/// Parse the exit IP from a Cloudflare cdn-cgi/trace response body.
/// Returns the value after "ip=" on the first matching line, trimmed.
/// Returns empty string if no "ip=" line is found.
pub fn parse_trace_body_ip(body: &str) -> String {
    body.lines()
        .find_map(|l| l.strip_prefix("ip=").map(|s| s.trim().to_string()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_defaults_account_to_port() {
        let id = resin_identity("MyPlatform", "", 17990);
        assert_eq!(id, "MyPlatform.port-17990");
    }

    #[test]
    fn distinct_ports_distinct_identities() {
        let a = resin_identity("P", "", 17990);
        let b = resin_identity("P", "", 17991);
        assert_ne!(a, b);
        assert!(a.contains("17990"));
        assert!(b.contains("17991"));
    }

    #[test]
    fn detect_socks_vs_http() {
        assert_eq!(detect_protocol(0x05), "socks5");
        assert_eq!(detect_protocol(0x47), "http");
        assert_eq!(detect_protocol(0x00), "http");
    }

    #[test]
    fn new_defaults_to_engine_mode_binds_nothing() {
        let db = crate::db::DbPool::open_in_memory().expect("mem db");
        let f = PortForwarder::new(db, "127.0.0.1", 9, "");
        assert_eq!(f.mode(), DataPlaneMode::Engine);
        assert!(!f.is_shell());
        // An Engine forwarder never reports real listeners beyond the DB view.
        assert!(!f.running_ports().contains(&9));
    }

    #[test]
    fn shell_mode_flag() {
        let db = crate::db::DbPool::open_in_memory().expect("mem db");
        let f = PortForwarder::shell(db, "127.0.0.1", 9, "");
        assert_eq!(f.mode(), DataPlaneMode::Shell);
        assert!(f.is_shell());
    }

    #[test]
    fn dialect_gate_table() {
        // mixed serves both; the single-protocol values serve only their own.
        assert!(dialect_allowed("mixed", "http"));
        assert!(dialect_allowed("mixed", "socks5"));
        assert!(dialect_allowed("http", "http"));
        assert!(!dialect_allowed("http", "socks5"));
        assert!(dialect_allowed("socks5", "socks5"));
        assert!(!dialect_allowed("socks5", "http"));
        // Out-of-set tokens are rejected at the boundaries before storage;
        // if one still arrives, the gate opens both ways (never half-dead).
        assert!(dialect_allowed("nonsense", "http"));
        assert!(dialect_allowed("nonsense", "socks5"));
    }

    #[test]
    fn basic_auth_known_vector() {
        // base64("Default.port-1:") == "RGVmYXVsdC5wb3J0LTE6"
        assert_eq!(
            basic_proxy_auth("Default.port-1", ""),
            "Basic RGVmYXVsdC5wb3J0LTE6"
        );
    }

    #[test]
    fn basic_proxy_auth_base64_vectors() {
        // Standard-alphabet base64 of "<identity>:<proxy_token>" (RFC 7617).
        assert_eq!(basic_proxy_auth("a", ""), "Basic YTo=");
        assert_eq!(basic_proxy_auth("ab", ""), "Basic YWI6");
        assert_eq!(basic_proxy_auth("abc", ""), "Basic YWJjOg==");
    }

    #[test]
    fn response_head_is_2xx_table() {
        assert!(response_head_is_2xx("HTTP/1.1 200 OK\r\n\r\n"));
        assert!(response_head_is_2xx("HTTP/1.0 204 No Content\r\n\r\n"));
        assert!(!response_head_is_2xx("HTTP/1.1 403 Forbidden\r\n\r\n"));
        assert!(!response_head_is_2xx("garbage"));
    }

    #[test]
    fn head_has_event_stream_detects_sse() {
        assert!(head_has_event_stream(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\n\r\n"
        ));
        assert!(!head_has_event_stream(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n"
        ));
    }

    #[test]
    fn split_absolute_form_table() {
        assert_eq!(
            split_absolute_form("http://api.example.com/v1/chat?x=1").unwrap(),
            Some((("api.example.com".into()), 80, ("/v1/chat?x=1".into())))
        );
        assert_eq!(
            split_absolute_form("http://127.0.0.1:8080").unwrap(),
            Some((("127.0.0.1".into()), 8080, ("/".into())))
        );
        assert_eq!(
            split_absolute_form("http://127.0.0.1:8080/").unwrap(),
            Some((("127.0.0.1".into()), 8080, ("/".into())))
        );
        // https absolute-form and origin-form are not tunnel-replayable here.
        assert_eq!(split_absolute_form("https://api.example.com/v1").unwrap(), None);
        assert_eq!(split_absolute_form("/v1/chat").unwrap(), None);
    }

    #[test]
    fn replayed_head_strips_proxy_headers_and_closes() {
        let mut lines = vec![
            "Host: api.example.com".to_string(),
            "Proxy-Authorization: Basic dGVzdA==".to_string(),
            "Accept: text/event-stream".to_string(),
            "Connection: keep-alive".to_string(),
        ];
        let out = build_replayed_head(&mut lines, "POST", "/v1/chat", "api.example.com:80");
        assert!(out.starts_with("POST /v1/chat HTTP/1.1\r\n"));
        assert!(!out.contains("Proxy-Authorization"));
        assert!(!out.to_ascii_lowercase().contains("keep-alive"));
        assert!(out.contains("Connection: close"));
        assert!(out.ends_with("\r\n\r\n"));
    }

    #[test]
    fn split_authority_ports() {
        assert_eq!(
            split_authority("api.example.com:8443").unwrap(),
            (("api.example.com".into()), 8443)
        );
        assert_eq!(split_authority("only.host").unwrap(), (("only.host".into()), 443));
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

    async fn wait_bound(f: &PortForwarder, port: u16, want: bool) {
        for _ in 0..100 {
            let has = f.running_ports().contains(&port);
            if has == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("bound set did not reach want={want} for port {port}");
    }

    #[tokio::test]
    async fn shell_reload_binds_and_stops_entry_ports() {
        let db = crate::db::DbPool::open_in_memory().expect("mem db");
        let rows = vec![mapping(47991, "mixed", true), mapping(47992, "http", false)];
        let f = PortForwarder::shell(db, "127.0.0.1", 9, "tok");
        let started = f.reload(&rows).await.expect("reload");
        assert_eq!(started, 1, "only the enabled row spawns");
        wait_bound(&f, 47991, true).await;
        assert!(!f.running_ports().contains(&47992), "disabled must not bind");
        // Idempotent reload adds nothing.
        let again = f.reload(&rows).await.expect("reload 2");
        assert_eq!(again, 0);
        // Removing the row stops the loop.
        f.reload(&[]).await.expect("reload empty");
        wait_bound(&f, 47991, false).await;
    }

    #[tokio::test]
    async fn engine_reload_is_noop() {
        let db = crate::db::DbPool::open_in_memory().expect("mem db");
        let f = PortForwarder::new(db, "127.0.0.1", 9, "tok");
        assert_eq!(f.reload(&[mapping(47993, "mixed", true)]).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn reload_rejects_more_than_max_entry_ports() {
        let db = crate::db::DbPool::open_in_memory().expect("mem db");
        let rows: Vec<PortMapping> = (0..(MAX_ENTRY_PORTS + 1) as u32)
            .map(|i| mapping(20000 + i as u16, "mixed", true))
            .collect();
        let f = PortForwarder::shell(db, "127.0.0.1", 9, "tok");
        assert!(f.reload(&rows).await.is_err());
    }

    #[tokio::test]
    async fn entry_bind_survives_held_port_with_retry() {
        // Hold a port so the first bind must fail; the retry loop must bind
        // once the holder goes away (no apply-side error, snapshot-visible).
        let holder = std::net::TcpListener::bind("127.0.0.1:47995").expect("hold port");
        let db = crate::db::DbPool::open_in_memory().expect("mem db");
        let f = PortForwarder::shell(db, "127.0.0.1", 9, "tok");
        f.reload(&[mapping(47995, "mixed", true)]).await.expect("reload");
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!f.running_ports().contains(&47995), "held port must not read bound");
        drop(holder);
        wait_bound(&f, 47995, true).await;
    }
}

#[cfg(test)]
mod trace_tests {
    use super::parse_trace_body_ip;

    #[test]
    fn parses_ip_from_cloudflare_trace() {
        let body = "fl=123f\nh=cloudflare.com\nip=203.0.113.50\ntls=TLSv1.3\n";
        assert_eq!(parse_trace_body_ip(body), "203.0.113.50");
    }

    #[test]
    fn returns_empty_when_no_ip_line() {
        let body = "fl=123f\nh=cloudflare.com\nvisited=2026-08-12\n";
        assert_eq!(parse_trace_body_ip(body), "");
    }

    #[test]
    fn trims_whitespace_around_ip() {
        let body = "ip=  198.51.100.1  \n";
        assert_eq!(parse_trace_body_ip(body), "198.51.100.1");
    }
}

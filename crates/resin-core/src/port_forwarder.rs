//! Thin multi-port forwarder (ADR-0012 / Phase 2).
//!
//! Each Entry Port is an identity. Traffic arriving on a mapped local port is
//! forwarded to the Resin sidecar's single listen port with Resin V1 proxy
//! auth rewritten to `Platform.Account:PROXY_TOKEN`:
//!   - SOCKS5: RFC1929 username=Platform.Account, password=PROXY_TOKEN
//!   - HTTP:   Proxy-Authorization: Basic base64(Platform.Account:PROXY_TOKEN)
//!
//! Resin v1.1.2 has no live /api/v1/endpoints (DESIGN.md documents it; the
//! release binary returns 404). So the shell owns multi-port listening.
//!
//! Ponytail: hand-rolled minimal SOCKS5 CONNECT + HTTP CONNECT/absolute-form
//! only. No full proxy stack. Bidirectional copy via tokio::io::copy_bidirectional.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::db::{DbPool, PortMapping};
use crate::{StreamSensor, StreamSensorSnapshot};

/// Max concurrent entry ports the shell will bind (industrial safety).
pub const MAX_ENTRY_PORTS: usize = 256;
/// Reserved / privileged ports rejected at the IPC boundary.
pub const MIN_USER_PORT: u16 = 1024;

/// Build the Resin V1 identity string `Platform.Account`.
/// Account defaults to `port-<n>` when empty so two ports never collapse.
pub fn resin_identity(platform_name: &str, account: &str, port: u16) -> String {
    let platform = platform_name.trim();
    let platform = if platform.is_empty() {
        "Default"
    } else {
        platform
    };
    let acct = account.trim();
    let acct = if acct.is_empty() {
        format!("port-{port}")
    } else {
        acct.to_string()
    };
    format!("{platform}.{acct}")
}

/// First-byte protocol detect: 0x05 = SOCKS5, otherwise treat as HTTP.
pub fn detect_protocol(first: u8) -> &'static str {
    if first == 0x05 {
        "socks5"
    } else {
        "http"
    }
}

fn basic_proxy_auth(identity: &str, proxy_token: &str) -> String {
    // base64 of "identity:token" — minimal encoder, no extra crate.
    let raw = format!("{identity}:{proxy_token}");
    format!("Basic {}", b64_encode(raw.as_bytes()))
}

fn b64_encode(input: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if input.len() - i == 1 {
        let n = (input[i] as u32) << 16;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if input.len() - i == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

/// Runtime multi-port manager. Holds abort handles per bound port.
#[derive(Clone)]
pub struct PortForwarder {
    inner: Arc<PortForwarderInner>,
}

struct PortForwarderInner {
    db: DbPool,
    resin_host: String,
    resin_port: u16,
    proxy_token: String,
    stream_sensor: StreamSensor,
    /// port -> join handle of accept loop
    running: Mutex<HashMap<u16, JoinHandle<()>>>,
    /// broadcast cancel: when false, all accept loops exit
    alive: watch::Sender<bool>,
}

impl PortForwarder {
    pub fn new(
        db: DbPool,
        resin_host: impl Into<String>,
        resin_port: u16,
        proxy_token: impl Into<String>,
    ) -> Self {
        let (alive, _) = watch::channel(true);
        Self {
            inner: Arc::new(PortForwarderInner {
                db,
                resin_host: resin_host.into(),
                resin_port,
                proxy_token: proxy_token.into(),
                stream_sensor: StreamSensor::new(),
                running: Mutex::new(HashMap::new()),
                alive,
            }),
        }
    }

    pub fn resin_addr(&self) -> String {
        format!("{}:{}", self.inner.resin_host, self.inner.resin_port)
    }

    pub fn proxy_token(&self) -> &str {
        &self.inner.proxy_token
    }

    /// Header-only AI stream observations from plain HTTP proxy traffic.
    pub fn stream_snapshot(&self) -> StreamSensorSnapshot {
        self.inner.stream_sensor.snapshot()
    }

    /// Reload listeners from DB: start enabled missing ports, stop disabled/deleted.
    pub async fn reload(&self) -> Result<usize, String> {
        let mappings = self.inner.db.list_ports()?;
        if mappings.iter().filter(|m| m.enabled).count() > MAX_ENTRY_PORTS {
            return Err(format!(
                "too many enabled entry ports (max {MAX_ENTRY_PORTS})"
            ));
        }
        let enabled: HashMap<u16, PortMapping> = mappings
            .into_iter()
            .filter(|m| m.enabled)
            .map(|m| (m.port, m))
            .collect();

        // Stop removed / disabled
        let to_stop: Vec<u16> = {
            let running = self.inner.running.lock();
            running
                .keys()
                .copied()
                .filter(|p| !enabled.contains_key(p))
                .collect()
        };
        for p in to_stop {
            self.stop_port(p).await;
        }

        // Start new
        let mut started = 0usize;
        for (port, m) in enabled {
            let already = self.inner.running.lock().contains_key(&port);
            if already {
                continue;
            }
            match self.spawn_port(m).await {
                Ok(()) => started += 1,
                Err(e) => {
                    tracing::error!(port, error = %e, "port_forwarder: failed to bind entry port");
                    return Err(e);
                }
            }
        }
        Ok(started)
    }

    async fn stop_port(&self, port: u16) {
        let handle = self.inner.running.lock().remove(&port);
        if let Some(h) = handle {
            h.abort();
            tracing::info!(port, "port_forwarder: stopped entry port");
        }
    }

    async fn spawn_port(&self, m: PortMapping) -> Result<(), String> {
        if m.port < MIN_USER_PORT {
            return Err(format!("port {} is privileged (< {MIN_USER_PORT})", m.port));
        }
        let addr: SocketAddr = format!("127.0.0.1:{}", m.port)
            .parse()
            .map_err(|e| format!("bad bind addr: {e}"))?;
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| format!("bind 127.0.0.1:{}: {e}", m.port))?;
        let identity = resin_identity(&m.platform_name, &m.account, m.port);
        let protocol_hint = m.protocol.to_lowercase();
        let resin_host = self.inner.resin_host.clone();
        let resin_port = self.inner.resin_port;
        let proxy_token = self.inner.proxy_token.clone();
        let stream_sensor = self.inner.stream_sensor.clone();
        let port = m.port;
        let mut alive_rx = self.inner.alive.subscribe();

        let handle = tokio::spawn(async move {
            tracing::info!(port, identity = %identity, protocol = %protocol_hint, "port_forwarder: listening");
            loop {
                tokio::select! {
                    _ = alive_rx.changed() => {
                        if !*alive_rx.borrow() { break; }
                    }
                    acc = listener.accept() => {
                        match acc {
                            Ok((client, peer)) => {
                                let identity = identity.clone();
                                let resin_host = resin_host.clone();
                                let proxy_token = proxy_token.clone();
                                let stream_sensor = stream_sensor.clone();
                                let protocol_hint = protocol_hint.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = handle_client(
                                        client, peer, &identity, &resin_host, resin_port, &proxy_token, &protocol_hint, &stream_sensor,
                                    ).await {
                                        tracing::debug!(port, error = %e, "port_forwarder: client session ended");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::warn!(port, error = %e, "port_forwarder: accept error");
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            }
                        }
                    }
                }
            }
            tracing::info!(port, "port_forwarder: accept loop exited");
        });

        self.inner.running.lock().insert(port, handle);
        Ok(())
    }

    pub fn running_ports(&self) -> Vec<u16> {
        let mut v: Vec<u16> = self.inner.running.lock().keys().copied().collect();
        v.sort_unstable();
        v
    }

    pub fn shutdown(&self) {
        let _ = self.inner.alive.send(false);
        let mut running = self.inner.running.lock();
        for (p, h) in running.drain() {
            h.abort();
            tracing::info!(port = p, "port_forwarder: shutdown abort");
        }
    }
}

async fn handle_client(
    mut client: TcpStream,
    peer: SocketAddr,
    identity: &str,
    resin_host: &str,
    resin_port: u16,
    proxy_token: &str,
    protocol_hint: &str,
    stream_sensor: &StreamSensor,
) -> Result<(), String> {
    let _ = peer;
    let mut first = [0u8; 1];
    client
        .read_exact(&mut first)
        .await
        .map_err(|e| format!("peek first byte: {e}"))?;
    let detected = detect_protocol(first[0]);
    // Prefer mapping.protocol when it is explicit socks5/http; otherwise use detect.
    let proto = if protocol_hint == "socks5" || protocol_hint == "http" {
        protocol_hint
    } else {
        detected
    };
    if proto == "socks5" {
        handle_socks5(
            client,
            first[0],
            identity,
            resin_host,
            resin_port,
            proxy_token,
        )
        .await
    } else {
        handle_http(
            client,
            first[0],
            identity,
            resin_host,
            resin_port,
            proxy_token,
            stream_sensor,
        )
        .await
    }
}

async fn handle_socks5(
    mut client: TcpStream,
    first: u8,
    identity: &str,
    resin_host: &str,
    resin_port: u16,
    proxy_token: &str,
) -> Result<(), String> {
    // Greeting: VER NMETHODS METHODS...
    if first != 0x05 {
        return Err("not socks5".into());
    }
    let mut nmethods = [0u8; 1];
    client
        .read_exact(&mut nmethods)
        .await
        .map_err(|e| e.to_string())?;
    let mut methods = vec![0u8; nmethods[0] as usize];
    if !methods.is_empty() {
        client
            .read_exact(&mut methods)
            .await
            .map_err(|e| e.to_string())?;
    }
    // We accept NO AUTH from the AI gateway — port is the identity.
    client
        .write_all(&[0x05, 0x00])
        .await
        .map_err(|e| e.to_string())?;

    // Request: VER CMD RSV ATYP DST.ADDR DST.PORT
    let mut hdr = [0u8; 4];
    client
        .read_exact(&mut hdr)
        .await
        .map_err(|e| e.to_string())?;
    if hdr[0] != 0x05 {
        return Err("bad socks ver in req".into());
    }
    if hdr[1] != 0x01 {
        // only CONNECT
        let _ = client
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await;
        return Err("socks cmd not CONNECT".into());
    }
    let (host, port) = read_socks_addr(&mut client, hdr[3]).await?;

    // Open authed SOCKS5 to Resin and issue the same CONNECT.
    let mut upstream =
        socks5_connect_authed(resin_host, resin_port, identity, proxy_token, &host, port).await?;

    // Success reply to client (bind 0.0.0.0:0)
    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|e| e.to_string())?;

    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

async fn read_socks_addr(stream: &mut TcpStream, atyp: u8) -> Result<(String, u16), String> {
    match atyp {
        0x01 => {
            let mut ip = [0u8; 4];
            stream
                .read_exact(&mut ip)
                .await
                .map_err(|e| e.to_string())?;
            let mut p = [0u8; 2];
            stream.read_exact(&mut p).await.map_err(|e| e.to_string())?;
            Ok((
                format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
                u16::from_be_bytes(p),
            ))
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|e| e.to_string())?;
            let mut name = vec![0u8; len[0] as usize];
            stream
                .read_exact(&mut name)
                .await
                .map_err(|e| e.to_string())?;
            let mut p = [0u8; 2];
            stream.read_exact(&mut p).await.map_err(|e| e.to_string())?;
            let host = String::from_utf8(name).map_err(|e| e.to_string())?;
            Ok((host, u16::from_be_bytes(p)))
        }
        0x04 => {
            let mut ip = [0u8; 16];
            stream
                .read_exact(&mut ip)
                .await
                .map_err(|e| e.to_string())?;
            let mut p = [0u8; 2];
            stream.read_exact(&mut p).await.map_err(|e| e.to_string())?;
            // compact v6 string
            let segs: Vec<String> = ip
                .chunks(2)
                .map(|c| format!("{:x}", u16::from_be_bytes([c[0], c[1]])))
                .collect();
            Ok((segs.join(":"), u16::from_be_bytes(p)))
        }
        _ => Err(format!("unsupported atyp {atyp}")),
    }
}

async fn socks5_connect_authed(
    resin_host: &str,
    resin_port: u16,
    identity: &str,
    proxy_token: &str,
    dest_host: &str,
    dest_port: u16,
) -> Result<TcpStream, String> {
    let addr = format!("{resin_host}:{resin_port}");
    let mut s = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("connect resin {addr}: {e}"))?;
    // greeting: offer user/pass only
    s.write_all(&[0x05, 0x01, 0x02])
        .await
        .map_err(|e| e.to_string())?;
    let mut resp = [0u8; 2];
    s.read_exact(&mut resp).await.map_err(|e| e.to_string())?;
    if resp[0] != 0x05 || resp[1] != 0x02 {
        return Err(format!("resin socks auth method rejected: {:02x?}", resp));
    }
    // RFC1929
    let user = identity.as_bytes();
    let pass = proxy_token.as_bytes();
    if user.len() > 255 || pass.len() > 255 {
        return Err("identity/token too long for socks5".into());
    }
    let mut auth = Vec::with_capacity(3 + user.len() + pass.len());
    auth.push(0x01);
    auth.push(user.len() as u8);
    auth.extend_from_slice(user);
    auth.push(pass.len() as u8);
    auth.extend_from_slice(pass);
    s.write_all(&auth).await.map_err(|e| e.to_string())?;
    let mut auth_resp = [0u8; 2];
    s.read_exact(&mut auth_resp)
        .await
        .map_err(|e| e.to_string())?;
    if auth_resp[1] != 0x00 {
        return Err(format!("resin socks auth failed: {:02x?}", auth_resp));
    }
    // CONNECT with domain ATYP
    let host_b = dest_host.as_bytes();
    if host_b.len() > 255 {
        return Err("dest host too long".into());
    }
    let mut req = Vec::with_capacity(7 + host_b.len());
    req.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host_b.len() as u8]);
    req.extend_from_slice(host_b);
    req.extend_from_slice(&dest_port.to_be_bytes());
    s.write_all(&req).await.map_err(|e| e.to_string())?;
    // reply
    let mut rh = [0u8; 4];
    s.read_exact(&mut rh).await.map_err(|e| e.to_string())?;
    if rh[1] != 0x00 {
        return Err(format!("resin socks CONNECT failed status={}", rh[1]));
    }
    // consume bind addr
    let _ = read_socks_addr(&mut s, rh[3]).await?;
    Ok(s)
}

async fn handle_http(
    mut client: TcpStream,
    first: u8,
    identity: &str,
    resin_host: &str,
    resin_port: u16,
    proxy_token: &str,
    stream_sensor: &StreamSensor,
) -> Result<(), String> {
    // Read headers (cap 64 KiB)
    let mut buf = vec![first];
    let mut tmp = [0u8; 1024];
    loop {
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            return Err("http headers too large".into());
        }
        let n = client.read(&mut tmp).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("client closed before headers".into());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let header_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no header end")?
        + 4;
    let head = String::from_utf8_lossy(&buf[..header_end]);
    // split drops the final empty segment after trailing CRLFCRLFs
    let mut lines: Vec<String> = head
        .split("\r\n")
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .collect();
    if lines.is_empty() {
        return Err("empty http".into());
    }
    // Header-only telemetry; never decrypts CONNECT traffic or reads request bodies.
    let header_value = |name: &str| -> String {
        lines
            .iter()
            .find_map(|line| {
                line.split_once(':')
                    .and_then(|(k, v)| k.eq_ignore_ascii_case(name).then(|| v.trim().to_string()))
            })
            .unwrap_or_default()
    };
    stream_sensor.observe_headers(
        &header_value("accept"),
        &header_value("upgrade"),
        &header_value("content-type"),
    );
    // Drop any inbound Proxy-Authorization; inject ours (port = identity).
    lines.retain(|l| !l.to_ascii_lowercase().starts_with("proxy-authorization:"));
    let auth = basic_proxy_auth(identity, proxy_token);
    lines.insert(1, format!("Proxy-Authorization: {auth}"));
    let mut out = lines.join("\r\n");
    out.push_str("\r\n\r\n");
    // body already in buf after header_end (rare for CONNECT)
    let body = &buf[header_end..];

    let addr = format!("{resin_host}:{resin_port}");
    let mut upstream = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("connect resin {addr}: {e}"))?;
    upstream
        .write_all(out.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    if !body.is_empty() {
        upstream.write_all(body).await.map_err(|e| e.to_string())?;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_defaults_account_to_port() {
        assert_eq!(resin_identity("OpenAI", "", 17990), "OpenAI.port-17990");
        assert_eq!(resin_identity("OpenAI", "k1", 17990), "OpenAI.k1");
        assert_eq!(resin_identity("", "", 1), "Default.port-1");
    }

    #[test]
    fn distinct_ports_distinct_identities() {
        let a = resin_identity("Pool", "", 17990);
        let b = resin_identity("Pool", "", 17991);
        assert_ne!(a, b);
    }

    #[test]
    fn detect_socks_vs_http() {
        assert_eq!(detect_protocol(0x05), "socks5");
        assert_eq!(detect_protocol(b'C'), "http");
        assert_eq!(detect_protocol(b'G'), "http");
    }

    #[test]
    fn basic_auth_roundtrip_shape() {
        let h = basic_proxy_auth("OpenAI.port-17990", "tok");
        assert!(h.starts_with("Basic "));
        // "OpenAI.port-17990:tok" base64
        assert_eq!(h, format!("Basic {}", b64_encode(b"OpenAI.port-17990:tok")));
    }

    #[test]
    fn b64_padding() {
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
    }
    #[tokio::test]
    async fn reload_binds_enabled_ports_and_stops_disabled() {
        let db = crate::db::DbPool::open_in_memory().expect("mem db");
        // Pick high ephemeral ports unlikely to collide on CI hosts.
        let p1: u16 = 37991;
        let p2: u16 = 37992;
        db.upsert_port(&PortMapping {
            port: p1,
            protocol: "socks5".into(),
            platform_name: "OpenAI".into(),
            account: format!("port-{p1}"),
            label: "a".into(),
            enabled: true,
        })
        .unwrap();
        db.upsert_port(&PortMapping {
            port: p2,
            protocol: "http".into(),
            platform_name: "OpenAI".into(),
            account: format!("port-{p2}"),
            label: "b".into(),
            enabled: true,
        })
        .unwrap();

        // resin_port is unused until a client connects; use a dummy.
        let fwd = PortForwarder::new(db.clone(), "127.0.0.1", 9, "tok-test");
        let started = fwd.reload().await.expect("reload start");
        assert_eq!(started, 2, "both enabled ports should start");
        let running = fwd.running_ports();
        assert_eq!(running, vec![p1, p2]);

        // Disable p2 + delete identity of p1 stays; reload must stop p2 only.
        db.upsert_port(&PortMapping {
            port: p2,
            protocol: "http".into(),
            platform_name: "OpenAI".into(),
            account: format!("port-{p2}"),
            label: "b".into(),
            enabled: false,
        })
        .unwrap();
        let started2 = fwd.reload().await.expect("reload disable");
        assert_eq!(started2, 0, "no new ports");
        assert_eq!(fwd.running_ports(), vec![p1]);

        // Remove p1 entirely.
        db.delete_port(p1).unwrap();
        let started3 = fwd.reload().await.expect("reload remove");
        assert_eq!(started3, 0);
        assert!(fwd.running_ports().is_empty());
        fwd.shutdown();
    }

    #[tokio::test]
    async fn two_http_entry_ports_inject_distinct_resin_identities() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use tokio::sync::mpsc;

        async fn read_headers(stream: &mut TcpStream) -> String {
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 512];
            loop {
                let n = stream.read(&mut chunk).await.expect("read request");
                assert!(n > 0, "client closed before HTTP headers");
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    return String::from_utf8(bytes).expect("HTTP headers are UTF-8");
                }
            }
        }

        let resin_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock Resin");
        let resin_port = resin_listener
            .local_addr()
            .expect("mock Resin address")
            .port();
        let (headers_tx, mut headers_rx) = mpsc::channel(2);
        tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = resin_listener
                    .accept()
                    .await
                    .expect("accept forwarded request");
                let headers = read_headers(&mut stream).await;
                headers_tx
                    .send(headers)
                    .await
                    .expect("collect forwarded headers");
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                    )
                    .await
                    .expect("respond from mock Resin");
            }
        });

        async fn free_port() -> u16 {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("reserve entry port");
            listener.local_addr().expect("entry address").port()
        }
        let first_port = free_port().await;
        let second_port = free_port().await;
        assert_ne!(first_port, second_port);

        let db = crate::db::DbPool::open_in_memory().expect("memory database");
        for (port, account) in [(first_port, "account-a"), (second_port, "account-b")] {
            db.upsert_port(&PortMapping {
                port,
                protocol: "http".into(),
                platform_name: "SharedPlatform".into(),
                account: account.into(),
                label: format!("entry-{port}"),
                enabled: true,
            })
            .expect("persist port mapping");
        }
        let forwarder = PortForwarder::new(db, "127.0.0.1", resin_port, "proxy-token");
        assert_eq!(forwarder.reload().await.expect("start entry listeners"), 2);

        for port in [first_port, second_port] {
            let mut client = TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("connect entry port");
            client.write_all(b"GET http://example.test/v1/models HTTP/1.1\r\nHost: example.test\r\nAccept: application/json\r\nProxy-Authorization: Basic attacker-controlled\r\nConnection: close\r\n\r\n")
                .await
                .expect("send client request");
            let mut response = Vec::new();
            client
                .read_to_end(&mut response)
                .await
                .expect("read forwarded response");
            assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK"));
        }

        let first = headers_rx.recv().await.expect("first forwarded request");
        let second = headers_rx.recv().await.expect("second forwarded request");
        let expected_a = basic_proxy_auth("SharedPlatform.account-a", "proxy-token");
        let expected_b = basic_proxy_auth("SharedPlatform.account-b", "proxy-token");
        assert!(first.contains(&expected_a) || first.contains(&expected_b));
        assert!(second.contains(&expected_a) || second.contains(&expected_b));
        assert_ne!(
            first, second,
            "two entry ports must inject distinct Resin identities"
        );
        assert!(!first.contains("attacker-controlled"));
        assert!(!second.contains("attacker-controlled"));
        forwarder.shutdown();
    }

    #[tokio::test]
    async fn reload_rejects_over_capacity() {
        let db = crate::db::DbPool::open_in_memory().expect("mem db");
        // Insert MAX_ENTRY_PORTS+1 enabled rows without binding (ports may already be used;
        // capacity check runs BEFORE bind).
        for i in 0..=MAX_ENTRY_PORTS as u16 {
            let port = 40000 + i;
            db.upsert_port(&PortMapping {
                port,
                protocol: "socks5".into(),
                platform_name: "P".into(),
                account: format!("port-{port}"),
                label: String::new(),
                enabled: true,
            })
            .unwrap();
        }
        let fwd = PortForwarder::new(db, "127.0.0.1", 9, "tok");
        let err = fwd.reload().await.expect_err("must reject over capacity");
        assert!(err.contains("too many enabled entry ports"), "got: {err}");
    }
}

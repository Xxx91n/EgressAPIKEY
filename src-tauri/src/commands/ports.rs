//! ports domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by architecture-recovery
//! ticket 08: pure mechanical move - no behavior, naming, or IPC-surface change.
use serde::{Serialize};
use tauri::{State};
use crate::sidecar::SidecarHandle;
use resin_core::DbPool;
use resin_core::IpcError;
use super::common::{NAME_MAX_LEN, find_endpoint_id_by_port, resin_client, validate_short_name};

/// Validate an entry-port mapping before touching DB / listeners.
pub fn validate_port_mapping(
    port: u16,
    protocol: &str,
    platform_name: &str,
    account: &str,
    label: &str,
) -> Result<(), String> {
    use resin_core::{MAX_ENTRY_PORTS, MIN_USER_PORT};
    if port < MIN_USER_PORT {
        return Err(format!("port {port} is privileged (< {MIN_USER_PORT})"));
    }
    let proto = protocol.trim().to_ascii_lowercase();
    if proto != "socks5" && proto != "http" {
        return Err("protocol must be socks5 or http".into());
    }
    if !platform_name.is_empty() { validate_short_name(platform_name, "platform_name")?; }
    // account + label optional but length/control capped
    if account.len() > NAME_MAX_LEN || account.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err("account invalid".into());
    }
    if label.len() > NAME_MAX_LEN || label.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err("label invalid".into());
    }
    // Resin Platform.Account forbids these chars in either side
    let forbidden = |s: &str| s.chars().any(|ch| ".:/\\@?#%~ ".contains(ch));
    if !platform_name.is_empty() && forbidden(platform_name) {
        return Err("platform_name contains Resin-forbidden chars".into());
    }
    if !account.is_empty() && forbidden(account) {
        return Err("account contains Resin-forbidden chars".into());
    }
    let _ = MAX_ENTRY_PORTS; // capacity enforced at reload
    Ok(())
}

/// Worker-port range guard for port_* commands that take only a port number.
/// Mirrors the lower-bound check inside `validate_port_mapping` but without
/// the protocol/account checks (auth-info + health-check only need the port
/// segment). Ensures a hostile GUI caller cannot ask the shell to dial a
/// privileged system port.
pub fn validate_port_segments(port: u16) -> Result<(), String> {
    if port < resin_core::MIN_USER_PORT {
        return Err(format!("port {port} is privileged (< {})", resin_core::MIN_USER_PORT));
    }
    // u16 upper bound is 65535, no range check needed above MIN_USER_PORT.
    Ok(())
}

#[tauri::command]
pub async fn port_list(db: State<'_, DbPool>) -> Result<Vec<resin_core::PortMapping>, IpcError> {
    db.list_ports().map_err(IpcError::from)
}

/// T10-6: Smart port suggestion (ADR-0031).
/// Reads port_mappings for used ports, starts from 17990, skips used,
/// probes each candidate with TcpListener::bind, returns first available.
#[tauri::command]
pub async fn port_suggest(db: State<'_, DbPool>) -> Result<u16, IpcError> {
    let used: std::collections::HashSet<u16> = db
        .list_ports()
        .map_err(IpcError::from)?
        .into_iter()
        .map(|m| m.port)
        .collect();
    for candidate in 17990u16..=65535u16 {
        if used.contains(&candidate) { continue; }
        if std::net::TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            return Ok(candidate);
        }
    }
    // Fallback: OS-assigned free port (port 0)
    Ok(std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| IpcError::from(format!("port_suggest: no free port: {e}")))?
        .local_addr()
        .map_err(|e| IpcError::from(format!("port_suggest: no local addr: {e}")))?
        .port())
}

/// Upsert one entry-port via the whitebox config transaction
/// (validate -> SQLite replace -> listener reload -> atomic JSON -> hotswap).
#[tauri::command]
pub async fn port_upsert(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    port: u16,
    protocol: String,
    platform_name: String,
    account: String,
    label: String,
    enabled: bool,
    auth_required: bool,
) -> Result<resin_core::PortMapping, IpcError> {
    validate_port_mapping(port, &protocol, &platform_name, &account, &label)?;
    let acct = if account.trim().is_empty() {
        format!("port-{port}")
    } else {
        account
    };
    let proto = protocol.trim().to_ascii_lowercase();
    let m = resin_core::PortMapping {
        port,
        protocol: proto.clone(),
        platform_name,
        account: acct,
        label,
        enabled,
        auth_required,
    };
    // Step 1: Resin endpoint API CRUD (owns listener lifecycle)
    if enabled {
        let client = resin_client(&sidecar)?;
        let existing = client.list_endpoints().await
            .map_err(|e| IpcError::from(format!("list_endpoints: {e:?}")))?;
        let found = find_endpoint_id_by_port(&existing, port);
        let allow_socks5 = proto == "socks5";
        let allow_http_forward = proto == "http" || proto == "socks5";
        let body = serde_json::json!({
            "port": port,
            "allow_management": false,
            "allow_proxy": true,
            "allow_http_forward": allow_http_forward,
            "allow_http_reverse": false,
            "allow_socks5": allow_socks5,
            "require_proxy_auth_info": auth_required,
        });
        if let Some(ep_id) = found {
            // PATCH if port exists
            client.update_endpoint(&ep_id, body).await
                .map_err(|e| IpcError::from(format!("update_endpoint: {e:?}")))?;
        } else {
            // POST if port does not exist (create new listener)
            client.create_endpoint(body).await
                .map_err(|e| IpcError::from(format!("create_endpoint: {e:?}")))?;
        }
    }
    // Step 2: Shell DB + whitebox metadata (port -> platform_name binding)
    let mut next = whitebox.snapshot();
    if let Some(existing) = next.entry_ports.iter_mut().find(|row| row.port == m.port) {
        *existing = m.clone();
    } else {
        next.entry_ports.push(m.clone());
        next.entry_ports.sort_by_key(|row| row.port);
    }
    whitebox.apply(&db, &forwarder, next).await?;
    Ok(m)
}

#[tauri::command]
pub async fn port_remove(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    port: u16,
) -> Result<bool, IpcError> {
    if port < resin_core::MIN_USER_PORT {
        return Err(IpcError::from(format!("port {port} is privileged")));
    }
    // Step 1: Resin endpoint API delete (find by port -> endpoint_id -> DELETE)
    let client = resin_client(&sidecar)?;
    let existing = client.list_endpoints().await
        .map_err(|e| IpcError::from(format!("list_endpoints: {e:?}")))?;
    if let Some(ep_id) = find_endpoint_id_by_port(&existing, port) {
        client.delete_endpoint(&ep_id).await
            .map_err(|e| IpcError::from(format!("delete_endpoint: {e:?}")))?;
    } else {
        tracing::warn!("port_remove: no Resin endpoint found for port {port}, proceeding with shell DB cleanup");
    }
    // Step 2: Remove from shell DB + whitebox metadata
    let mut next = whitebox.snapshot();
    next.entry_ports.retain(|row| row.port != port);
    whitebox.apply(&db, &forwarder, next).await?;
    Ok(true)
}

/// T18-S2 (ADR-0042): Toggle enabled flag on an entry-port without
/// re-POST/Create or DELETE. Patches the Resin endpoint `{enabled: bool}`
/// (Resin v1.2.0 supports `enabled` on PATCH — `inactive` keeps the record)
/// then persists the same flag into the shell whitebox `entry_ports[].enabled`
/// so the whitebox is the authoritative record. The listener is NOT removed
/// from Resin's DB when toggled off, so toggled back on is a PATCH only.
#[tauri::command]
pub async fn port_toggle(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    port: u16,
    enabled: bool,
) -> Result<resin_core::PortMapping, IpcError> {
    validate_port_segments(port)?;
    // Step 1: Resin endpoint PATCH {enabled} — find endpoint by port.
    let client = resin_client(&sidecar)?;
    let existing = client.list_endpoints().await
        .map_err(|e| IpcError::from(format!("list_endpoints: {e:?}")))?;
    match find_endpoint_id_by_port(&existing, port) {
        Some(ep_id) => {
            let body = serde_json::json!({ "enabled": enabled });
            client.update_endpoint(&ep_id, body).await
                .map_err(|e| IpcError::from(format!("update_endpoint (toggle): {e:?}")))?;
            tracing::info!(target: "ipc.port_toggle", port, enabled, %ep_id, "endpoint patched");
        }
        None => {
            // Port was never created at Resin (or was DELETEd). Toggling off is
            // a no-op; toggling on without an endpoint record is impossible —
            // the user must re-create via port_upsert. Log and proceed to
            // update the shell whitebox enabled flag so the GUI still reflects
            // the requested state.
            tracing::warn!(target: "ipc.port_toggle", port, enabled, "no Resin endpoint for port; only updating shell whitebox");
        }
    }
    // Step 2: Update shell DB + whitebox metadata.
    let mut next = whitebox.snapshot();
    let row = next.entry_ports.iter_mut().find(|r| r.port == port);
    if let Some(r) = row {
        r.enabled = enabled;
        let m = r.clone();
        whitebox.apply(&db, &forwarder, next).await?;
        Ok(m)
    } else {
        Err(IpcError::from(format!("port_toggle: port {port} not in whitebox")))
    }
}

/// T8-1 (ADR-0029): Bind an entry-port to a platform WITHOUT touching
/// auth_required. Only updates the shell-side whitebox PortMapping
/// platform_name field. Does NOT call port_upsert (which would default
/// auth_required=true and flip no-auth ports to require-auth).
#[tauri::command]
pub async fn port_bind_platform(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    port: u16,
    platform_name: String,
) -> Result<bool, IpcError> {
    validate_port_segments(port)?;
    // Allow empty platform_name = unbind. Non-empty must pass name validation.
    if !platform_name.is_empty() {
        validate_short_name(&platform_name, "platform_name")?;
        let forbidden = |s: &str| s.chars().any(|ch| ".:/\\@?#%~ ".contains(ch));
        if forbidden(&platform_name) {
            return Err(IpcError::from("platform_name contains Resin-forbidden chars".to_string()));
        }
    }
    let mut next = whitebox.snapshot();
    let found = next.entry_ports.iter_mut().find(|row| row.port == port);
    match found {
        Some(m) => {
            m.platform_name = platform_name;
            whitebox.apply(&db, &forwarder, next).await?;
            Ok(true)
        }
        None => Err(IpcError::from(format!("port {port} not found in entry_ports"))),
    }
}

#[tauri::command]
pub async fn port_running(
    forwarder: State<'_, resin_core::PortForwarder>,
) -> Result<Vec<u16>, IpcError> {
    Ok(forwarder.running_ports())
}

/// Explicit reload of the whitebox file (hand-edit path). Invalid files are
/// rejected by hotswap-config validation and leave the active map unchanged.
#[tauri::command]
pub async fn port_reload(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<usize, IpcError> {
    whitebox.reload_file(&db, &forwarder).await.map_err(IpcError::from)
}

/// T6-3: Save network-layer config (DNS + idle conns + probe + bypass) to the
/// whitebox JSON file, atomically updating in-memory state + env injection.
#[tauri::command]
pub async fn whitebox_save_network(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    network: resin_core::NetworkConfig,
) -> Result<usize, IpcError> {
    let mut current = whitebox.snapshot();
    current.network = network;
    whitebox.apply(&db, &forwarder, current).await.map_err(IpcError::from)
}

#[tauri::command]
pub async fn whitebox_path(
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<String, IpcError> {
    Ok(whitebox.path().display().to_string())
}

#[tauri::command]
pub async fn whitebox_get(
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<resin_core::WhiteboxConfig, IpcError> {
    Ok(whitebox.snapshot())
}

#[tauri::command]
pub async fn whitebox_reload(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<usize, IpcError> {
    whitebox.reload_file(&db, &forwarder).await.map_err(IpcError::from)
}

#[tauri::command]
pub async fn stream_sensor_snapshot(
    forwarder: State<'_, resin_core::PortForwarder>,
) -> Result<resin_core::StreamSensorSnapshot, IpcError> {
    Ok(forwarder.stream_snapshot())
}

/// ADR-0021 Q1: return the SOCKS5 authentication credentials a gateway must
/// present to reach one entry-port. Resin's SOCKS5 server, when `proxy_token`
/// is set in the sidecar env, requires UserPass auth: username equals the
/// port's bound `platform.account` string (`Platform.Account`), password is
/// the sidecar global proxy token kept in `SidecarHandle.proxy_token`. As long
/// as the sidecar sets `RESIN_PROXY_TOKEN`, every port requires auth — there
/// is no per-port no-auth fallback without forking Resin, so `auth_required`
/// is read from the port_mapping row; ADR-0027: proxy_token is now empty so
/// per-endpoint `require_proxy_auth_info` controls auth per port.
#[derive(Debug, Serialize, Clone)]
pub struct PortAuthInfo {
    /// SOCKS5 username to present = the port's bound Platform.Account string.
    pub username: String,
    /// SOCKS5 password = the sidecar global proxy token (session-stable).
    pub password: String,
    /// Read from the port_mapping row; when false, no credential is needed.
    pub auth_required: bool,
    /// Bound platform name (for GUI display).
    pub platform_name: String,
    /// Port number echoed back so the GUI can pair the auth with the row.
    pub port: u16,
}

#[tauri::command]
pub async fn port_auth_info(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    port: u16,
) -> Result<PortAuthInfo, IpcError> {
    // The port_mapping row tells us the bound platform_name + account string.
    let mapping = db
        .list_ports()?
        .into_iter()
        .find(|m| m.port == port)
        .ok_or_else(|| format!("port {port} is not configured"))?;
    // Validate port range early so a hostile caller cannot pivot to an
    // arbitrary port number for a dial probe (Path-traversal safety:reuse the
    // same guard the whitebox config transaction already enforces).
    validate_port_segments(port)?;
    // T6-Bug5: Resin SOCKS5 requires `<Platform>.<Account>` format as the
    // username. Without the platform prefix, SOCKS5 auth succeeds (password
    // = proxy_token validates) but CONNECT returns "General failure" because
    // Resin cannot determine which platform the traffic belongs to.
    let username = if mapping.account.trim().is_empty() {
        format!("{}.port-{}", mapping.platform_name, port)
    } else {
        format!("{}.{}", mapping.platform_name, mapping.account)
    };
    // T7-fix: proxy_token is now empty (enables no-auth). When auth_required=true,
    // the password is empty — Resin socks5.go:307 short-circuits the check when
    // s.token=="" and forward.go:103-115 accepts any credential. The username
    // (Platform.Account) is still used for routing.
    let password = sidecar.proxy_token.clone();
    let _ = forwarder; // forwarder carries the live listener state; not needed for auth-info lookup
    Ok(PortAuthInfo {
        username,
        password,
        auth_required: mapping.auth_required,
        platform_name: mapping.platform_name,
        port,
    })
}

/// ADR-0021 Q1: live TCP probe + minimal SOCKS5 method-negotiation so the GUI
/// can show a green/red health chip per port (same pattern clash-verge-rev
/// uses for `CoreManager` reachability). We send the 3-byte greeting
/// `05 01 02` (SOCKS5 version + 1 method + method 0x02 UserPass). A healthy
/// listener replies `05 02`; an HTTP-mode listener replies with an HTTP status
/// line or a non-SOCKS5 byte we flag as `protocol_mismatch` but still
/// `reachable=true`. Takes ~100ms typical; capped at 750ms.
#[derive(Debug, Serialize, Clone)]
pub struct PortHealthCheck {
    pub port: u16,
    /// TCP connect succeeded.
    pub reachable: bool,
    /// SOCKS5 greeting got a plausible `05 <method>` reply.
    pub socks5_ok: bool,
    /// Listener replied with bytes but not SOCKS5 shape -> probably HTTP.
    pub protocol_mismatch: bool,
    /// Measured round-trip in millis (connect + greeting/reply).
    pub latency_ms: u64,
    /// Human-facing reason: "ok" | "refused" | "timeout" | "noop_no_reply" | "protocol_mismatch"
    pub reason: String,
}

#[tauri::command]
pub async fn port_health_check(port: u16, protocol: Option<String>) -> Result<PortHealthCheck, IpcError> {
    tracing::info!(port, protocol = ?protocol, "port_health_check: probing port");
    validate_port_segments(port)?;
    let proto = protocol
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "socks5".into());
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let started = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    // 200ms connect timeout (headroom under the 750ms request budget).
    let connect = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        tokio::net::TcpStream::connect(&addr),
    )
    .await;
    let mut stream = match connect {
        Ok(Ok(s)) => s,
        Ok(Err(_)) => {
            return Ok(PortHealthCheck {
                port,
                reachable: false,
                socks5_ok: false,
                protocol_mismatch: false,
                latency_ms: started.elapsed().as_millis() as u64,
                reason: "refused".into(),
            });
        }
        Err(_) => {
            return Ok(PortHealthCheck {
                port,
                reachable: false,
                socks5_ok: false,
                protocol_mismatch: false,
                latency_ms: started.elapsed().as_millis() as u64,
                reason: "timeout".into(),
            });
        }
    };
    // SOCKS5 greeting: version 5, 2 method candidates: NoAuth(0x00) + UserPass(0x02).
    // With empty proxy_token, Resin accepts either; with non-empty, only UserPass.
    let greeting: Vec<u8> = if proto == "http" {
        // T6-Bug1: HTTP GET probe. Resin's HTTP proxy does NOT support CONNECT
        // tunneling — CONNECT returns 404/error. A plain GET / gets any HTTP
        // response (200/404/400) which proves the port is alive.
        format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").into_bytes()
    } else {
        vec![0x05, 0x02, 0x00, 0x02]
    };
    if let Err(e) = stream.write_all(&greeting).await {
        tracing::info!(port, proto = %proto, error = %e, "port_health_check: write probe failed");
        return Ok(PortHealthCheck {
            port,
            reachable: true,
            socks5_ok: false,
            protocol_mismatch: false,
            latency_ms: started.elapsed().as_millis() as u64,
            reason: "noop_no_reply".into(),
        });
    }
    // Reply should be exactly 2 bytes: 0x05 0x00 (NoAuth) or 0x05 0x02 (UserPass). HTTP
    // listeners reply with an HTTP status line e.g. `HTTP/1.1 400...`.
    let mut buf = [0u8; 16];
    let read = tokio::time::timeout(
        std::time::Duration::from_millis(550),
        stream.read(&mut buf),
    )
    .await;
    let elapsed = started.elapsed().as_millis() as u64;
    match read {
        Ok(Ok(n)) if proto == "http" && n >= 12 && buf.starts_with(b"HTTP/") => Ok(PortHealthCheck {
            port,
            reachable: true,
            socks5_ok: false,
            protocol_mismatch: false,
            latency_ms: elapsed,
            reason: "ok".into(),
        }),
        Ok(Ok(n)) if proto == "socks5" && n >= 2 && buf[0] == 0x05 && (buf[1] == 0x00 || buf[1] == 0x02) => Ok(PortHealthCheck {
            port,
            reachable: true,
            socks5_ok: true,
            protocol_mismatch: false,
            latency_ms: elapsed,
            reason: "ok".into(),
        }),
        Ok(Ok(n)) if n >= 4 => Ok(PortHealthCheck {
            // Has bytes but not a SOCKS5 shape: most likely an HTTP listener
            // replying with an error status line (`HTTP/1.1 ...`). Still
            // reachable; mark protocol_mismatch so the GUI shows a distinct chip.
            port,
            reachable: true,
            socks5_ok: false,
            protocol_mismatch: true,
            latency_ms: elapsed,
            reason: "protocol_mismatch".into(),
        }),
        _ => Ok(PortHealthCheck {
            port,
            reachable: true,
            socks5_ok: false,
            protocol_mismatch: false,
            latency_ms: elapsed,
            reason: "noop_no_reply".into(),
        }),
    }
}

use std::sync::atomic::AtomicBool;

use std::sync::Arc;

/// Shared pause flag managed by main.rs and read by every watch_port_health task.
#[derive(Clone)]
pub struct PortHealthPaused(pub Arc<AtomicBool>);

impl PortHealthPaused {
    pub fn new(start_paused: bool) -> Self {
        Self(Arc::new(AtomicBool::new(start_paused)))
    }
}

/// T18 Phase 1: stream port-health snapshots down a Tauri Channel. One Tokio
/// task per invocation; it ends when the channel is closed (frontend unmount).
#[tauri::command]
pub async fn watch_port_health(
    on_event: tauri::ipc::Channel<resin_core::PortHealthSnapshot>,
    db: State<'_, DbPool>,
    paused: State<'_, PortHealthPaused>,
) -> Result<(), IpcError> {
    // ports_fn snapshot is taken from the shell DB so the watcher retries the
    // latest port set on every tick (new ports appear, deleted ports drop out
    // without restarting the watcher). Disabled ports are filtered here.
    let db_clone: DbPool = db.inner().clone();
    let ports_fn = move || {
        db_clone.list_ports().unwrap_or_default().into_iter().filter(|m| m.enabled).collect::<Vec<_>>()
    };
    // Translate the Tauri Channel into the emit closure resin_core expects.
    let channel_clone = on_event.clone();
    let emit = move |snap: resin_core::PortHealthSnapshot| -> Result<(), ()> {
        // send() returns Err if the webview has dropped the channel; that ends the watcher.
        channel_clone.send(snap).map_err(|_| ())
    };
    let paused_arc = paused.inner().0.clone();
    resin_core::port_health::spawn_watcher(ports_fn, emit, paused_arc);
    Ok(())
}

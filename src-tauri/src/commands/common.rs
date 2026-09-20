//! Shared domain IPC commands (EgressAPIKEY).
//! Shared IPC-boundary helpers used by every command domain.

use crate::sidecar::SidecarHandle;
use resin_core::ResinClient;

pub const KEY_MAX_LEN: usize = 4096;

pub const NAME_MAX_LEN: usize = 128;

pub fn validate_short_name(name: &str, field: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > NAME_MAX_LEN {
        return Err(format!("{field} length out of range (1..={NAME_MAX_LEN})"));
    }
    if name.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err(format!("{field} contains control characters"));
    }
    Ok(())
}

pub fn validate_ip(ip: &str) -> Result<(), String> {
    if ip.is_empty() || ip.len() > 253 {
        return Err("exit_ip length out of range".to_string());
    }
    if ip
        .bytes()
        .any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ')
    {
        return Err("exit_ip contains control/space characters".to_string());
    }
    Ok(())
}

pub fn resin_client(h: &SidecarHandle) -> Result<ResinClient, String> {
    ResinClient::new(&h.api_base(), h.admin_token.clone())
        .map_err(|e| format!("sidecar client: {e:?}"))
}

pub use resin_core::map_resin_error;

/// Extract the array from a Resin list response. Resin wraps paginated
/// collections as `{"items":[...], "total", "limit", "offset"}`; a few
/// legacy endpoints still return a bare array. Accept both so a future
/// Resin API tightening cannot silently empty the UI.
pub use resin_core::items_arr;

/// Find a Resin endpoint ID by port number from the list-endpoints response.
/// Handles both `{"items":[...]}` wrapper and bare-array shapes.
/// Skips the read-only `default` endpoint.
pub fn find_endpoint_id_by_port(existing: &serde_json::Value, port: u16) -> Option<String> {
    for ep in items_arr(existing).iter() {
        if ep.get("port").and_then(|p| p.as_u64()) == Some(port as u64) {
            if let Some(id) = ep.get("id").and_then(|v| v.as_str()) {
                if id != "default" {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

/// (ADR-0042 S6): Restore Resin endpoints from whitebox config on startup.
/// Spawns-safe: failures log only, never fail the app. Skips ports already in Resin (409 Conflict).
/// Mode A (ADR-0068 D1) delegates to restore_shell_listeners: the
/// shell's accept loops are the entry-port listeners there, not engine rows.
pub async fn restore_ports_from_whitebox(
    sidecar: &SidecarHandle,
    whitebox: &resin_core::WhiteboxConfigStore,
    forwarder: &resin_core::PortForwarder,
) -> Result<(), String> {
    let cfg = whitebox.snapshot();
    if forwarder.is_shell() {
        return restore_shell_listeners(sidecar, &cfg, forwarder).await;
    }
    let enabled = resin_core::enabled_entries_for_restore(&cfg.entry_ports);
    if enabled.is_empty() {
        tracing::info!("no enabled entry_ports to restore");
        return Ok(());
    }
    let client = resin_client(sidecar)?;
    let existing = client
        .list_endpoints()
        .await
        .map_err(|e| format!("list_endpoints: {e:?}"))?;
    let mut restored = 0u32;
    let mut skipped = 0u32;
    for m in enabled {
        if find_endpoint_id_by_port(&existing, m.port).is_some() {
            skipped += 1;
            continue;
        }
        // ONE shared derivation, not a fourth
        // hand-rolled copy. mixed opens both capabilities, http only HTTP
        // forwarding, socks5 only SOCKS5.
        let (allow_socks5, allow_http_forward) =
            resin_core::entry_protocol::engine_flags(&m.protocol);
        let body = serde_json::json!({
            "port": m.port,
            "allow_management": false,
            "allow_proxy": true,
            "allow_http_forward": allow_http_forward,
            "allow_http_reverse": false,
            "allow_socks5": allow_socks5,
            "require_proxy_auth_info": m.auth_required,
        });
        match client.create_endpoint(body).await {
            Ok(_) => {
                restored += 1;
                tracing::info!(port = m.port, "restored Resin endpoint from whitebox");
            }
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("409") || msg.contains("CONFLICT") || msg.contains("Only one usage")
                {
                    skipped += 1;
                    tracing::info!(port = m.port, "port already in Resin; skipping");
                } else {
                    tracing::warn!(port = m.port, error = %msg, "restore failed; user can re-save in GUI");
                }
            }
        }
    }
    tracing::info!(restored, skipped, "whitebox port restore complete");
    Ok(())
}

/// Mode A restore: the whitebox rows apply to the shell's own
/// accept loops, so the startup work is two convergence acts over L3-derived
/// state (ADR-0042 S6: rebuildable, never user data):
///   1. retire stale per-port Resin endpoints - a B-era row persisted in
///      Resin's state.db would EADDRINUSE the shell bind and shadow the
///      forwarder's credential-free entry; the default consolidated
///      endpoint is never touched;
///   2. reload the accept loops from the whitebox snapshot.
/// Endpoint-delete failures are warnings, not fatal: the listener-retry loop
/// keeps backing off and the snapshot reports the port missing meanwhile.
async fn restore_shell_listeners(
    sidecar: &SidecarHandle,
    cfg: &resin_core::WhiteboxConfig,
    forwarder: &resin_core::PortForwarder,
) -> Result<(), String> {
    let client = resin_client(sidecar)?;
    let existing = client
        .list_endpoints()
        .await
        .map_err(|e| format!("list_endpoints: {e:?}"))?;
    let mut retired = 0u32;
    for ep in items_arr(&existing) {
        let id = match ep.get("id").and_then(|v| v.as_str()) {
            Some(i) if i != "default" => i,
            _ => continue,
        };
        let ep_port = ep.get("port").and_then(|p| p.as_u64());
        let shadowing = cfg
            .entry_ports
            .iter()
            .any(|m| Some(m.port as u64) == ep_port);
        if !shadowing {
            continue;
        }
        match client.delete_endpoint(id).await {
            Ok(_) => {
                retired += 1;
                tracing::info!(
                    port = ep_port,
                    endpoint = id,
                    "retired stale Resin endpoint so the shell listener owns the entry port"
                );
            }
            Err(e) => {
                tracing::warn!(port = ep_port, error = %format!("{e:?}"), "stale endpoint delete failed; shell bind will retry behind it");
            }
        }
    }
    forwarder.reload(&cfg.entry_ports).await?;
    tracing::info!(retired, "shell entry listeners restored from whitebox");
    Ok(())
}

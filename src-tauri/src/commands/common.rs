//! Shared domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by architecture-recovery
//! ticket 08: pure mechanical move - no behavior, naming, or IPC-surface change.
//! Shared IPC-boundary helpers used by every command domain.

use crate::sidecar::SidecarHandle;
use resin_core::{ResinClient};

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
/// Resin API tightening cannot silently empty the UI (P13 root cause).
pub fn items_arr<'a>(v: &'a serde_json::Value) -> &'a [serde_json::Value] {
    if let Some(arr) = v.get("items").and_then(|i| i.as_array()) {
        return arr.as_slice();
    }
    if let Some(arr) = v.as_array() {
        return arr.as_slice();
    }
    &[]
}

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

/// T18-6 (ADR-0042 S6): Restore Resin endpoints from whitebox config on startup.
/// Spawns-safe: failures log only, never fail the app. Skips ports already in Resin (409 Conflict).
pub async fn restore_ports_from_whitebox(
    sidecar: &SidecarHandle,
    whitebox: &resin_core::WhiteboxConfigStore,
) -> Result<(), String> {
    let cfg = whitebox.snapshot();
    let enabled = resin_core::enabled_entries_for_restore(&cfg.entry_ports);
    if enabled.is_empty() {
        tracing::info!("T18-6: no enabled entry_ports to restore");
        return Ok(());
    }
    let client = resin_client(sidecar)?;
    let existing = client.list_endpoints().await
        .map_err(|e| format!("list_endpoints: {e:?}"))?;
    let mut restored = 0u32;
    let mut skipped = 0u32;
    for m in enabled {
        if find_endpoint_id_by_port(&existing, m.port).is_some() {
            skipped += 1;
            continue;
        }
        let proto = m.protocol.trim().to_ascii_lowercase();
        let allow_socks5 = proto == "socks5";
        let allow_http_forward = proto == "http" || proto == "socks5";
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
                tracing::info!(port = m.port, "T18-6: restored Resin endpoint from whitebox");
            }
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("409") || msg.contains("CONFLICT") || msg.contains("Only one usage") {
                    skipped += 1;
                    tracing::info!(port = m.port, "T18-6: port already in Resin; skipping");
                } else {
                    tracing::warn!(port = m.port, error = %msg, "T18-6: restore failed; user can re-save in GUI");
                }
            }
        }
    }
    tracing::info!(restored, skipped, "T18-6: whitebox port restore complete");
    Ok(())
}

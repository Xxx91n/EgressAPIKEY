//! Tauri 2 desktop shell for EgressAPIKEY. Owns sidecar lifecycle, the
//! system tray, and the Ghost-style safety net. Wires frontend IPC commands
//! to the Resin Go sidecar control plane (ADR-0012 thin-shell multi-port).
//!
//! IPC commands live in the `commands` submodule so the names registered via
//! `tauri::generate_handler!` do not collide with the macro-generated helper
//! items (Tauri 2 + Rust 1.97 build-time check). ADR-0024 removed the legacy
//! SharedGateway/GatewayState dead code: the data path now runs through the
//! Resin sidecar, so the shell no longer constructs a gateway state instance.

pub mod commands;
pub mod headless_security;
pub mod lightweight;
pub mod sidecar;
pub mod trace;
pub mod tray;
pub mod specta_bindings;

use std::sync::Arc;

/// Shared PlatformRegistry (Resin Platform/Account model) wrapped for Tauri
/// commands. Locked per-call, never held across an await.
pub type SharedRegistry = Arc<resin_core::platform::PlatformRegistry>;

/// Construct a fresh empty PlatformRegistry for the desktop session. The
/// frontend populates it via the `platform_add` / `account_add` IPC commands.
pub fn build_shared_registry() -> SharedRegistry {
    Arc::new(resin_core::platform::PlatformRegistry::new())
}

/// Legacy placeholder kept for any external callers/tests that used `init()`.
pub fn init() -> anyhow::Result<()> {
    tracing::trace!("EgressAPIKEY shell init");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_registry_is_empty() {
        let r = build_shared_registry();
        assert!(r.list().is_empty());
    }
}

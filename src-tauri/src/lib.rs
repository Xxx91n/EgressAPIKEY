//! Tauri 2 desktop shell for EgressAPIKEY. Owns sidecar lifecycle, the
//! system tray, and the Ghost-style safety net. Wires frontend IPC commands
//! to a resin-core GatewayState instance.
//!
//! IPC commands live in the `commands` submodule so the names registered via
//! `tauri::generate_handler!` do not collide with the macro-generated helper
//! items (Tauri 2 + Rust 1.97 build-time check).

pub mod commands;
pub mod sidecar;
pub mod tray;

use resin_core::gateway::GatewayState;
use std::sync::Arc;

/// Shared gateway state wrapped so Tauri commands can lock it cheaply.
pub type SharedGateway = Arc<parking_lot::Mutex<GatewayState>>;

/// Construct a fresh resin_core::gateway::GatewayState for the desktop session.
pub fn new_gateway_state() -> GatewayState {
    GatewayState::new(resin_core::DEFAULT_LANES)
}

/// Initialise the shared gateway state used by all Tauri commands.
pub fn build_shared_gateway(lanes: usize) -> SharedGateway {
    Arc::new(parking_lot::Mutex::new(GatewayState::new(lanes)))
}

/// Shared PlatformRegistry (Resin Platform/Account model) wrapped for Tauri
/// commands. Same concurrency story as SharedGateway — locked per-call, never
/// held across an await.
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
    fn shared_gateway_creates_with_default_lanes() {
        let g = build_shared_gateway(resin_core::DEFAULT_LANES);
        assert_eq!(g.lock().tdewma_snapshot().len(), 0);
    }

    #[test]
    fn reservation_keeps_lane() {
        let g = build_shared_gateway(resin_core::DEFAULT_LANES);
        let r = g.lock().reserve("sk-test", "acct", "api.openai.com", Some("1.2.3.4"));
        assert!(r.lease.is_some());
    }
}

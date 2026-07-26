//! Tauri 2 desktop shell for ai-api-route. Owns sidecar lifecycle, the
//! system tray, and the Ghost-style safety net. Wires frontend IPC commands
//! to a resin-core GatewayState instance.
//!
//! P2 wires the full Tauri plugin set; this module builds standalone so the
//! workspace compiles and the pure-backend target produces a resin-core binary
//! before the GUI is integrated.

use anyhow::Result;

/// Construct a fresh resin_core::gateway::GatewayState for the desktop session.
/// Tauri commands will wrap this handle.
pub fn new_gateway_state() -> resin_core::gateway::GatewayState {
    resin_core::gateway::GatewayState::new(resin_core::DEFAULT_LANES)
}

/// Placeholder entrypoint used while the GUI talks only to resin-core. The
/// real Tauri main is added in src/main.rs once the plugin set is wired.
pub fn init() -> Result<()> {
    tracing::trace!("ai-api-route shell init (placeholder)");
    Ok(())
}

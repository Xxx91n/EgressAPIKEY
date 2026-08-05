//! resin-core: Resin-pattern L7 gateway for AI API keys.
//!
//! Modules:
//! - [`lane`]: FxHash key -> lane index (N lanes, default 10, max 50)
//! - [`lease`]: SSE session sticky lease table (lane, account, exit IP)
//! - [`tdewma`]: TD-EWMA-lite per-authority latency EMA
//! - [`platform`]: Platform/Account model (Resin architecture, Rust)
//! - [`mihomo`]: mihomo sidecar REST API control (subscription -> YAML -> reload)
//! - [`gateway`]: axum HTTP + SSE forwarding gateway on 127.0.0.1
//! - [`resin_client`]: loopback REST client for the Resin Go sidecar API
//! - [`db`]: SQLite port->platform mapping store (reuses DbPool infra)

pub mod lane;
pub mod lease;
pub mod tdewma;
pub mod platform;
pub mod mihomo;
pub mod gateway;
pub mod resin_client;
pub mod db;
pub mod port_forwarder;

pub use lane::{lane_index, LaneConfig};
pub use lease::{LeaseTable, LeaseId};
pub use tdewma::TdEwma;
pub use platform::{Platform, Account, PlatformRegistry};
pub use resin_client::{ResinClient, fetch_clash_subscription, clash_yaml_to_proxies_block};
pub use db::{DbPool, PortMapping};
pub use port_forwarder::{PortForwarder, resin_identity, detect_protocol, MAX_ENTRY_PORTS, MIN_USER_PORT};

/// Re-export canonical config for the whole core.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CoreConfig {
    /// Number of lanes (1..=50). Default 10.
    pub lanes: usize,
    /// Bind address for the L7 gateway (127.0.0.1:7897 recommended).
    pub bind: String,
    /// mihomo REST API base URL (http://127.0.0.1:9090).
    pub mihomo_api: String,
    /// mihomo admin secret, if any.
    pub mihomo_secret: Option<String>,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            lanes: 10,
            bind: "127.0.0.1:7897".to_string(),
            mihomo_api: "http://127.0.0.1:9090".to_string(),
            mihomo_secret: None,
        }
    }
}

/// Validate lane count against the README contract (default 10, max 50).
pub const MAX_LANES: usize = 50;
pub const MIN_LANES: usize = 1;
pub const DEFAULT_LANES: usize = 10;

pub fn sanitize_lanes(n: usize) -> usize {
    n.clamp(MIN_LANES, MAX_LANES).max(1)
}

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

pub mod db;
pub mod ipc_error;
pub use ipc_error::{IpcError, map_resin_error};
pub mod gateway;
pub mod ip_reputation;
pub mod lane;
pub mod lease;
pub mod mihomo;
pub mod platform;
pub mod port_forwarder;
pub mod port_health;

pub mod resin_client;
pub mod strategy;
pub mod strategy_engine;
pub mod stream_sensor;
pub mod tdewma;
pub mod whitebox_config;

pub use db::{DbPool, PortMapping};
pub use ip_reputation::{
    parse_public_ips, ReputationClient, ReputationEntry, ReputationProvider, ReputationSnapshot,
};
pub use lane::{lane_index, LaneConfig};
pub use lease::{LeaseId, LeaseTable};
pub use platform::{Account, Platform, PlatformRegistry};
pub use port_forwarder::{
    detect_protocol, parse_trace_body_ip, resin_identity, PortForwarder, MAX_ENTRY_PORTS, MIN_USER_PORT,
};
pub use resin_client::{clash_yaml_to_proxies_block, fetch_clash_subscription, ResinClient};
pub use strategy::{protocol_weight, strategy_catalog, StrategyId, StrategyInfo};
pub use strategy_engine::{a_class_regions, compute_plan, liveness_filter, parse_nodes, AClassStrategy, BClassParams, PlatformStrategy, StrategyConfig, NodeSummary};
pub use stream_sensor::{classify_http_headers, StreamKind, StreamSensor, StreamSensorSnapshot};
pub use tdewma::TdEwma;
pub use port_health::{adaptive_interval, HealthState, PortHealthEntry, PortHealthSnapshot};
pub use whitebox_config::{NetworkConfig,
    validate as validate_whitebox_config, WhiteboxConfig, WhiteboxConfigStore, WHITEBOX_CONFIG_FILE,
    enabled_entries_for_restore,
};

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

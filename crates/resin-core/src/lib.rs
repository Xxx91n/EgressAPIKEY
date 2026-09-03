//! resin-core: shell-side support crate for the EgressAPIKEY desktop app.
//!
//! The gateway data path lives in the Resin Go sidecar (P2C scheduler,
//! sticky-IP leases, TD-EWMA latency, mihomo runtime). This crate keeps the
//! pieces the Tauri shell really uses: the loopback REST client, the whitebox
//! config store, port forwarder/health helpers, the strategy engine, IP
//! reputation, stream sensing, the SQLite port mapping, and the typed IPC
//! error contract.
//!
//! Modules:
//! - db: SQLite port->platform mapping store (reuses DbPool infra)
//! - ipc_error: typed IPC error contract (ADR-0045)
//! - ip_reputation: pluggable egress-IP reputation providers
//! - platform: Platform/Account registry (shell SharedRegistry state)
//! - port_forwarder: entry-port forwarding + exit-IP probe helpers
//! - port_health: entry-port TCP/SOCKS5 health checks
//! - resin_client: loopback REST client for the Resin Go sidecar API
//! - strategy: B-class StrategyId type (shell strategy vocabulary)
//! - strategy_engine: A/B-class strategy evaluation (shell-side)
//! - strategy_service: StrategyConfig pipeline owner (read/validate/store/apply/snapshot, ADR-0052)
//! - stream_sensor: AI stream (SSE/WS) classification
//! - whitebox_config: egressapikey-ports.json whitebox store (ADR-0036)
//!
//! The former in-Rust gateway face (lane/lease/tdewma/gateway/mihomo modules,
//! CoreConfig, sanitize_lanes, and the resin-core stub bin) was deleted per
//! ADR-0050; that ADR records the reference evidence.

pub mod config_transfer;
pub mod db;
pub mod ipc_error;
pub mod ip_reputation;
pub mod platform;
pub mod port_forwarder;
pub mod port_health;
pub mod resin_client;
pub mod snapshot;
pub mod strategy;
pub mod strategy_engine;
pub mod strategy_service;
pub mod stream_sensor;
pub mod throttle;
pub mod whitebox_backup;
pub mod whitebox_config;

pub use config_transfer::{build_export_doc, parse_import_doc, ConfigImportDoc};
pub use db::{DbPool, PortMapping};
pub use ipc_error::{map_resin_error, IpcError};
pub use ip_reputation::{
    parse_public_ips, ReputationClient, ReputationProvider, ReputationSnapshot,
};
pub use port_forwarder::{parse_trace_body_ip, PortForwarder, MAX_ENTRY_PORTS, MIN_USER_PORT};
pub use port_health::PortHealthSnapshot;
pub use resin_client::ResinClient;
pub use snapshot::{
    AuthoritativeSnapshot, ConvergePhase, PortSnapshot, ProcessRouteSnapshot, StrategySnapshot,
    SubscriptionSnapshot,
};
pub use strategy_engine::{compute_plan, parse_nodes, StrategyConfig};
pub use strategy_service::{
    endpoint_live_ports, FsStrategyStore, ReconcileMemory, ReconcilePortsOutcome, StrategyService,
};
pub use whitebox_backup::WhiteboxBackupEntry;
pub use whitebox_config::{
    enabled_entries_for_restore, migrate_l1_process_routes, process_route_conflict_check,
    NetworkConfig, ProcessRouteRule, WhiteboxConfig, WhiteboxConfigStore, WHITEBOX_CONFIG_FILE,
};

/// IPC lane-range contract (AGENTS.md section 7.5): lane indices arriving
/// over Tauri IPC must be rejected when lane >= MAX_LANES. Kept here so the
/// shell and any future lane-indexed table share one ceiling.
pub const MAX_LANES: usize = 50;

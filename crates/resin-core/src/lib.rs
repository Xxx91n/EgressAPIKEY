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
//! - strategy: strategy catalog + protocol weight table
//! - strategy_engine: A/B-class strategy evaluation (shell-side)
//! - strategy_service: StrategyConfig pipeline owner (read/validate/store/apply/snapshot, ADR-0052)
//! - stream_sensor: AI stream (SSE/WS) classification
//! - whitebox_config: egressapikey-ports.json whitebox store (ADR-0036)
//!
//! The former in-Rust gateway face (lane/lease/tdewma/gateway/mihomo modules,
//! CoreConfig, sanitize_lanes, and the resin-core stub bin) was deleted per
//! ADR-0050; that ADR records the reference evidence.

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
pub mod whitebox_backup;
pub mod whitebox_config;

pub use db::{DbPool, PortMapping};
pub use ipc_error::{map_resin_error, IpcError};
pub use ip_reputation::{
    parse_public_ips, ReputationClient, ReputationEntry, ReputationProvider, ReputationSnapshot,
};
pub use platform::{Account, Platform, PlatformRegistry};
pub use port_forwarder::{
    detect_protocol, parse_trace_body_ip, resin_identity, PortForwarder, MAX_ENTRY_PORTS,
    MIN_USER_PORT,
};
pub use port_health::{adaptive_interval, HealthState, PortHealthEntry, PortHealthSnapshot};
pub use resin_client::ResinClient;
pub use snapshot::{AuthoritativeSnapshot, PortSnapshot, StrategySnapshot};
pub use strategy::{protocol_weight, strategy_catalog, StrategyId, StrategyInfo};
pub use strategy_engine::{
    a_class_regions, compute_plan, liveness_filter, parse_nodes, AClassStrategy, BClassParams,
    NodeSummary, PlatformStrategy, StrategyConfig,
};
pub use strategy_service::{
    clean_stale, compute_reconcile_plan, endpoint_live_ports, validate as validate_strategy_config,
    AppliedPlatform, ApplyReport, FsStrategyStore, ReconcileMemory, ReconcilePlan,
    ReconcilePortsOutcome, ReconcileReport, StrategyConfigStore, StrategyService,
    RECONCILE_PORT_TTL_SECS,
};
pub use stream_sensor::{classify_http_headers, StreamKind, StreamSensor, StreamSensorSnapshot};
pub use whitebox_backup::{
    atomic_write_bytes, backup_before_write, backup_dir, backup_list, now_unix, parse_backup_name,
    read_backup, read_backup_parsed, validate_backup_name, WhiteboxBackupEntry, BACKUP_DIR_NAME,
    WHITEBOX_BACKUP_KEEP,
};
pub use whitebox_config::{
    enabled_entries_for_restore, validate as validate_whitebox_config, NetworkConfig,
    WhiteboxConfig, WhiteboxConfigStore, WHITEBOX_CONFIG_FILE,
};

/// IPC lane-range contract (AGENTS.md section 7.5): lane indices arriving
/// over Tauri IPC must be rejected when lane >= MAX_LANES. Kept here so the
/// shell and any future lane-indexed table share one ceiling.
pub const MAX_LANES: usize = 50;

//! Whitebox entry-port configuration (Phase 3 / NEW-7).
//!
//! `HotswapConfig` owns the active, validated document. Every update is
//! validated before an atomic swap; application code persists the same document
//! to disk only after the SQLite + listener reload transaction succeeds.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use hotswap_config::{
    notify::SubscriptionHandle,
    prelude::{HotswapConfig, ValidationError},
};
use parking_lot::Mutex as SyncMutex;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::{DbPool, PortForwarder, PortMapping, MAX_ENTRY_PORTS, MIN_USER_PORT};

pub const WHITEBOX_CONFIG_FILE: &str = "egressapikey-ports.json";

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct WhiteboxConfig {
    pub version: u8,
    #[serde(default)]
    pub entry_ports: Vec<PortMapping>,
    #[serde(default)]
    pub network: NetworkConfig,
    /// Ticket 12 / ADR-0054 §D: optional exemption list. Members are decimal
    /// port numbers the user has marked "known drift, don't notify". Absent
    /// = empty (older configs load unchanged). NEVER enters the three-state
    /// merge — read-side presentation only. Shape checks in `validate`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acknowledged: Vec<String>,
}

impl WhiteboxConfig {
    pub fn from_ports(entry_ports: Vec<PortMapping>) -> Self {
        Self {
            version: 1,
            entry_ports,
            network: NetworkConfig::default(),
            acknowledged: Vec::new(),
        }
    }
}


#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct NetworkConfig {
    /// DNS upstream chain for sing-box node resolution.
    /// Empty = use Resin default DoH failover chain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns_upstreams: Vec<String>,
    /// Max idle connections in proxy transport pool. None = Resin default (1024).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_idle_conns: Option<u32>,
    /// Max idle connections per host. None = Resin default (64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_idle_conns_per_host: Option<u32>,
    /// Idle connection timeout in seconds. None = Resin default (90s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_conn_timeout_secs: Option<u64>,
    /// Node probe timeout in seconds. None = Resin default (15s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_timeout_secs: Option<u64>,
    /// Probe concurrency. None = Resin default (1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_concurrency: Option<u32>,
    /// Proxy bypass rules. Empty = none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proxy_bypass: Vec<String>,
}

/// Validate before the config is made active or persisted.
pub fn validate(config: &WhiteboxConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("whitebox config version must be 1".into());
    }
    if config.entry_ports.len() > MAX_ENTRY_PORTS {
        return Err(format!("too many entry ports (max {MAX_ENTRY_PORTS})"));
    }
    let mut seen = HashSet::with_capacity(config.entry_ports.len());
    for port in &config.entry_ports {
        if port.port < MIN_USER_PORT {
            return Err(format!(
                "port {} is privileged (< {MIN_USER_PORT})",
                port.port
            ));
        }
        if !seen.insert(port.port) {
            return Err(format!("duplicate port {}", port.port));
        }
        if !matches!(port.protocol.as_str(), "socks5" | "http") {
            return Err("protocol must be socks5 or http".into());
        }
        validate_identity(&port.platform_name, "platform_name")?;
        if !port.account.is_empty() {
            validate_identity(&port.account, "account")?;
        }
        validate_text(&port.label, "label")?;
    }
    validate_network(&config.network)?;
    // Ticket 12 / ADR-0054 §D: the exemption array shares the strategy
    // whitebox shape rules (≤64 × 1..128 chars, no control chars, no dupes).
    crate::strategy_service::validate_acknowledged(&config.acknowledged, "acknowledged")?;
    Ok(())
}

fn validate_text(value: &str, field: &str) -> Result<(), String> {
    if value.len() > 128 || value.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err(format!("{field} invalid"));
    }
    Ok(())
}

fn validate_identity(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 {
        return Err(format!("{field} invalid"));
    }
    validate_text(value, field)?;
    if value.chars().any(|ch| ".:/\\@?#%~ ".contains(ch)) {
        return Err(format!("{field} contains Resin-forbidden chars"));
    }
    Ok(())
}

fn validate_network(n: &NetworkConfig) -> Result<(), String> {
    if !n.dns_upstreams.is_empty() {
        for (i, up) in n.dns_upstreams.iter().enumerate() {
            if up.is_empty() {
                return Err(format!("dns_upstreams[{i}] must not be empty"));
            }
            if up.len() > 512 {
                return Err(format!("dns_upstreams[{i}] too long (max 512)"));
            }
        }
    }
    if let Some(v) = n.max_idle_conns {
        if v == 0 { return Err("max_idle_conns must be >= 1".into()); }
    }
    if let Some(v) = n.max_idle_conns_per_host {
        if v == 0 { return Err("max_idle_conns_per_host must be >= 1".into()); }
    }
    if let Some(v) = n.probe_concurrency {
        if v == 0 || v > 10000 { return Err("probe_concurrency must be 1..=10000".into()); }
    }
    for (i, b) in n.proxy_bypass.iter().enumerate() {
        if b.is_empty() {
            return Err(format!("proxy_bypass[{i}] must not be empty"));
        }
        if b.len() > 253 {
            return Err(format!("proxy_bypass[{i}] too long (max 253)"));
        }
    }
    Ok(())
}

fn as_validation_error(config: &WhiteboxConfig) -> Result<(), ValidationError> {
    validate(config).map_err(|e| ValidationError::invalid_field("entry_ports", e))
}

/// Runtime handle for the whitebox file and its in-memory atomic state.
#[derive(Clone)]
pub struct WhiteboxConfigStore {
    path: PathBuf,
    config: HotswapConfig<WhiteboxConfig>,
    // Writers are serialized so DB snapshot, listener reload and config swap do
    // not interleave. Reads stay lock-free through hotswap-config.
    writer: Arc<AsyncMutex<()>>,
    // Last successfully committed listener map; used to restore the atomic
    // in-memory view if a watched file cannot bind.
    applied: Arc<SyncMutex<WhiteboxConfig>>,
    // Keep the wheel subscription alive for the lifetime of the desktop app.
    subscription: Arc<SyncMutex<Option<SubscriptionHandle>>>,
}

impl WhiteboxConfigStore {
    pub async fn open(path: PathBuf, initial: WhiteboxConfig) -> Result<Self, String> {
        validate(&initial)?;
        if !path.exists() {
            write_atomic(&path, &initial)?;
        }
        let config = HotswapConfig::builder()
            .with_file(&path)
            .with_file_watch(true)
            .with_watch_debounce(Duration::from_millis(500))
            .with_validation(as_validation_error)
            .build::<WhiteboxConfig>()
            .await
            .map_err(|e| format!("open whitebox config: {e}"))?;
        Ok(Self {
            path,
            config,
            writer: Arc::new(AsyncMutex::new(())),
            applied: Arc::new(SyncMutex::new(initial)),
            subscription: Arc::new(SyncMutex::new(None)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn snapshot(&self) -> WhiteboxConfig {
        (*self.config.get()).clone()
    }

    /// Attach the file-watch bridge once runtime dependencies exist. Accepted
    /// file writes are applied to SQLite/listeners asynchronously; invalid files
    /// never trigger this callback because hotswap-config preserves the old value.
    pub async fn watch_apply(&self, db: DbPool, forwarder: PortForwarder) {
        let config = self.config.clone();
        let writer = self.writer.clone();
        let applied = self.applied.clone();
        let handle = self
            .config
            .subscribe(move || {
                let config = config.clone();
                let db = db.clone();
                let forwarder = forwarder.clone();
                let writer = writer.clone();
                let applied = applied.clone();
                tokio::spawn(async move {
                    let _guard = writer.lock().await;
                    let next = (*config.get()).clone();
                    let previous = applied.lock().clone();
                    match apply_ports(&db, &forwarder, &next.entry_ports).await {
                        Ok(_) => {
                            *applied.lock() = next;
                            tracing::info!("whitebox config file update applied");
                        }
                        Err(error) => {
                            // The wheel already atomically swapped the parsed file.
                            // Restore the last fully applied document so memory, DB
                            // and listeners remain one transactionally consistent view.
                            if let Err(restore_error) = config.update(previous).await {
                                tracing::error!(
                                    %restore_error,
                                    "whitebox config failed to restore active snapshot"
                                );
                            }
                            tracing::error!(
                                %error,
                                "whitebox config file update rejected during listener apply"
                            );
                        }
                    }
                });
            })
            .await;
        *self.subscription.lock() = Some(handle);
    }

    /// Explicitly reload a hand-edited file. Invalid data is rejected by the
    /// wheel and the old atomic config remains active.
    pub async fn reload_file(
        &self,
        db: &DbPool,
        _forwarder: &PortForwarder,
    ) -> Result<usize, String> {
        let _guard = self.writer.lock().await;
        self.config
            .reload()
            .await
            .map_err(|e| format!("reload whitebox config: {e}"))?;
        let next = self.snapshot();
        let started = apply_ports(db, _forwarder, &next.entry_ports).await?;
        *self.applied.lock() = next;
        Ok(started)
    }

    /// GUI writes use the identical validate -> DB -> listener -> file -> atomic
    /// config sequence as a whitebox file reload.
    pub async fn apply(
        &self,
        db: &DbPool,
        forwarder: &PortForwarder,
        next: WhiteboxConfig,
    ) -> Result<usize, String> {
        validate(&next)?;
        let _guard = self.writer.lock().await;
        let previous = self.snapshot();
        let started = apply_ports(db, forwarder, &next.entry_ports).await?;
        if let Err(e) = write_atomic(&self.path, &next) {
            let _ = apply_ports(db, forwarder, &previous.entry_ports).await;
            return Err(e);
        }
        if let Err(e) = self.config.update(next.clone()).await {
            let _ = apply_ports(db, forwarder, &previous.entry_ports).await;
            return Err(format!("activate whitebox config: {e}"));
        }
        *self.applied.lock() = next;
        Ok(started)
    }
}

/// T18-6 (ADR-0042 S6): Filter entry_ports to only enabled entries for
/// Resin endpoint restore on startup. Pure helper so it is unit-testable
/// without a live Resin sidecar.
pub fn enabled_entries_for_restore(entry_ports: &[PortMapping]) -> Vec<&PortMapping> {
    entry_ports.iter().filter(|m| m.enabled).collect()
}

async fn apply_ports(
    db: &DbPool,
    _forwarder: &PortForwarder,
    next: &[PortMapping],
) -> Result<usize, String> {
    validate(&WhiteboxConfig::from_ports(next.to_vec()))?;
    let _previous = db.list_ports()?;
    // Ponytail: Resin v1.2.0 owns listener lifecycle via /api/v1/endpoints.
    // The shell DB only stores port -> platform_name binding metadata.
    // Port CRUD (create/update/delete listener) happens through IPC commands
    // (port_upsert/port_remove) which call ResinClient endpoint API directly.
    // So hot-swap of shell metadata is just a DB write — no listener restart.
    db.replace_ports(next).map_err(|e| format!("entry-port DB replace failed: {e}"))?;
    Ok(next.len())
}

fn write_atomic(path: &Path, config: &WhiteboxConfig) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "whitebox config has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("create whitebox config directory: {e}"))?;
    let bytes =
        serde_json::to_vec_pretty(config).map_err(|e| format!("encode whitebox config: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("write whitebox config temp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("activate whitebox config: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(port: u16) -> PortMapping {
        PortMapping {
            port,
            protocol: "socks5".into(),
            platform_name: "OpenAI".into(),
            account: format!("port-{port}"),
            label: String::new(),
            enabled: true,
            auth_required: true,
        }
    }

    #[test]
    fn validator_rejects_duplicate_privileged_and_invalid_protocol() {
        let mut duplicate = WhiteboxConfig::from_ports(vec![mapping(17990), mapping(17990)]);
        assert!(validate(&duplicate).unwrap_err().contains("duplicate"));
        duplicate.entry_ports = vec![mapping(80)];
        assert!(validate(&duplicate).unwrap_err().contains("privileged"));
        duplicate.entry_ports = vec![PortMapping {
            protocol: "https".into(),
            ..mapping(17990)
        }];
        assert!(validate(&duplicate).unwrap_err().contains("protocol"));
    }

    #[test]
    fn validator_accepts_two_distinct_ports() {
        assert!(validate(&WhiteboxConfig::from_ports(vec![
            mapping(17990),
            mapping(17991)
        ]))
        .is_ok());
    }

    #[test]
    fn network_validation_rejects_empty_dns_entry() {
        let mut cfg = WhiteboxConfig::from_ports(vec![]);
        cfg.network.dns_upstreams = vec!["".to_string()];
        assert!(validate(&cfg).unwrap_err().contains("dns_upstreams[0] must not be empty"));
    }

    #[test]
    fn network_validation_rejects_zero_idle_conns() {
        let mut cfg = WhiteboxConfig::from_ports(vec![]);
        cfg.network.max_idle_conns = Some(0);
        assert!(validate(&cfg).unwrap_err().contains("max_idle_conns must be >= 1"));
    }

    #[test]
    fn network_validation_rejects_probe_concurrency_out_of_range() {
        let mut cfg = WhiteboxConfig::from_ports(vec![]);
        cfg.network.probe_concurrency = Some(10001);
        assert!(validate(&cfg).unwrap_err().contains("probe_concurrency must be 1..=10000"));
    }

    #[test]
    fn network_validation_accepts_empty_defaults() {
        let cfg = WhiteboxConfig::from_ports(vec![]);
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn network_validation_accepts_valid_dns_chain() {
        let mut cfg = WhiteboxConfig::from_ports(vec![]);
        cfg.network.dns_upstreams = vec!["https://doh.pub/dns-query".into(), "local".into()];
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn write_atomic_persists_json_without_partial_file() {
        let dir =
            std::env::temp_dir().join(format!("egressapikey-whitebox-{}", std::process::id()));
        let path = dir.join(WHITEBOX_CONFIG_FILE);
        let config = WhiteboxConfig::from_ports(vec![mapping(17990)]);
        write_atomic(&path, &config).unwrap();
        let loaded: WhiteboxConfig =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded, config);
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn acknowledged_field_parses_optional_and_round_trips() {
        // Absent field on an old file loads as empty (serde default).
        let raw = serde_json::json!({
            "version": 1,
            "entry_ports": []
        });
        let cfg: WhiteboxConfig = serde_json::from_value(raw).unwrap();
        assert!(cfg.acknowledged.is_empty());

        // Present field round-trips through write_atomic.
        let dir = std::env::temp_dir().join(format!("egressapikey-wb-ack-{}", std::process::id()));
        let path = dir.join(WHITEBOX_CONFIG_FILE);
        let mut cfg2 = WhiteboxConfig::from_ports(vec![mapping(17990)]);
        cfg2.acknowledged = vec!["17990".to_string()];
        assert!(validate(&cfg2).is_ok());
        write_atomic(&path, &cfg2).unwrap();
        let loaded: WhiteboxConfig = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded.acknowledged, vec!["17990".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn acknowledged_validate_rejects_bad_shapes() {
        let mut cfg = WhiteboxConfig::from_ports(vec![]);
        // non-string member is a deserialize error, not a validate pass-through
        let raw = serde_json::json!({"version": 1, "entry_ports": [], "acknowledged": ["17990", 7]});
        assert!(serde_json::from_value::<WhiteboxConfig>(raw).is_err());
        // empty member
        cfg.acknowledged = vec![String::new()];
        assert!(validate(&cfg).unwrap_err().contains("1..128"));
        // control character
        cfg.acknowledged = vec!["bad\u{0}port".to_string()];
        assert!(validate(&cfg).unwrap_err().contains("control"));
        // duplicate members
        cfg.acknowledged = vec!["17990".to_string(), "17990".to_string()];
        assert!(validate(&cfg).unwrap_err().contains("duplicated"));
        // oversized list
        cfg.acknowledged = (0..65).map(|i| i.to_string()).collect();
        assert!(validate(&cfg).unwrap_err().contains("max 64"));
        // valid passes
        cfg.acknowledged = vec!["17990".to_string(), "65535".to_string()];
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn enabled_entries_for_restore_filters_disabled() {
        let ports = vec![
            PortMapping { port: 17990, protocol: "socks5".into(), platform_name: "Default".into(), account: "".into(), label: "".into(), enabled: true, auth_required: true },
            PortMapping { port: 17991, protocol: "http".into(), platform_name: "Default".into(), account: "".into(), label: "".into(), enabled: false, auth_required: true },
            PortMapping { port: 17992, protocol: "socks5".into(), platform_name: "OpenAI".into(), account: "port-17992".into(), label: "".into(), enabled: true, auth_required: false },
        ];
        let enabled = enabled_entries_for_restore(&ports);
        assert_eq!(enabled.len(), 2);
        assert_eq!(enabled[0].port, 17990);
        assert_eq!(enabled[1].port, 17992);
    }
}

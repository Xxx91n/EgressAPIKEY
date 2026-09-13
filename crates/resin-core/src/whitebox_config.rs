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

use crate::whitebox_backup::{
    atomic_write_bytes, backup_before_write, backup_list, now_unix, read_backup_parsed,
    WhiteboxBackupEntry,
};
use crate::entry_protocol::{
    canonical_protocol, is_valid_protocol, DEFAULT_ENTRY_PORT_PROTOCOL, ENTRY_PORT_PROTOCOL_ERROR,
};
use crate::{DbPool, PortForwarder, PortMapping, MAX_ENTRY_PORTS, MIN_USER_PORT};

pub const WHITEBOX_CONFIG_FILE: &str = "egressapikey-ports.json";

/// Current whitebox document version.
///
/// v1 -> v2 is round 8 ticket 13 / D-007 (ADR-0068 D3): the entry-port protocol
/// enum gained `mixed` and `socks5` was tightened to SOCKS5-only. v1 documents
/// still load - the two-value vocabulary is flag-preserving onto `mixed` - and
/// are upgraded in place by `migrate_entry_port_protocols`.
pub const WHITEBOX_CONFIG_VERSION: u8 = 2;

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
    /// Ticket 17 / ADR-0055 D1: per-process -> entry-port routing rules,
    /// migrated out of L1 settings.json. Absent = empty (older files load
    /// unchanged); single write entry = WhiteboxConfigStore::apply via the
    /// process_route_* commands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_routes: Vec<ProcessRouteRule>,
    /// Ticket 17 / ADR-0055 D6: ADR-0054 §D exemption vocabulary for the
    /// route family (process names). Read-side presentation only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route_acknowledged: Vec<String>,
    /// Round 5 T09 / ADR-0058 (D-27): write-authority generation counter —
    /// ports half is SINGLE-generation by design: the writer and the apply
    /// executor are the same process (`WhiteboxConfigStore::apply` completes
    /// synchronously), the Crossplane "external resource needs a second
    /// observed generation" case does not exist here. Bumped by every
    /// accepted apply. Absent in a v1 file = 0.
    #[serde(default)]
    pub generation: u64,
    /// Unix seconds of the last accepted whitebox write. Pure metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

impl WhiteboxConfig {
    pub fn from_ports(entry_ports: Vec<PortMapping>) -> Self {
        Self {
            version: WHITEBOX_CONFIG_VERSION,
            entry_ports,
            network: NetworkConfig::default(),
            acknowledged: Vec::new(),
            process_routes: Vec::new(),
            route_acknowledged: Vec::new(),
            generation: 0,
            updated_at: None,
        }
    }
}


/// Ticket 17 / ADR-0055: one process-routing rule (L2 whitebox family).
/// This is the SINGLE wire shape — the former dual writers (webview
/// settings.ts camelCase `targetPort` vs Rust snake_case `target_port`)
/// collapsed into this snake_case form on disk and over IPC.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProcessRouteRule {
    pub process: String,
    pub target_port: u16,
}

/// Ticket 17 / ADR-0055 D1: upper bound for whitebox process routes
/// (AGENTS 7.5 bounded-array template).
pub const MAX_PROCESS_ROUTES: usize = 256;

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
    if config.version != 1 && config.version != WHITEBOX_CONFIG_VERSION {
        return Err(format!(
            "whitebox config version must be 1 or {WHITEBOX_CONFIG_VERSION}"
        ));
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
        if !is_valid_protocol(&port.protocol) {
            return Err(ENTRY_PORT_PROTOCOL_ERROR.into());
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
    // Ticket 17 / ADR-0055: route family validation. Bounded count, unique
    // normalized process names, port >= MIN_USER_PORT, and the legacy
    // one-port-one-process conflict rule so a hand-edited file cannot
    // smuggle two processes onto one port.
    if config.process_routes.len() > MAX_PROCESS_ROUTES {
        return Err(format!("too many process routes (max {MAX_PROCESS_ROUTES})"));
    }
    let mut seen_processes = HashSet::with_capacity(config.process_routes.len());
    for r in &config.process_routes {
        if r.process.is_empty() || r.process.len() > 128 {
            return Err("process route name must be 1..128 chars".into());
        }
        if r.process.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err("process route name contains control characters".into());
        }
        if r.target_port < MIN_USER_PORT {
            return Err(format!("process route port {} is privileged", r.target_port));
        }
        if !seen_processes.insert(r.process.trim().to_lowercase()) {
            return Err(format!("duplicate process route: {}", r.process));
        }
    }
    process_route_conflict_check(&config.process_routes)?;
    crate::strategy_service::validate_acknowledged(&config.route_acknowledged, "route_acknowledged")?;
    Ok(())
}

/// Ticket 17 / ADR-0055 D2: one target port may carry at most one process.
/// Returns Err naming the bound process (the legacy typed-conflict
/// contract, now part of document validation). Pure; unit-tested.
/// Round 8 ticket 13 / D-007 (spec IMP-1, ADR-0068 D3): the flag-preserving
/// rewrite of ONE legacy protocol token.
///
/// Under the v1 vocabulary a `socks5` row produced BOTH engine flags
/// (`allow_socks5` + `allow_http_forward`), i.e. it behaved exactly like
/// today's `mixed`. Rewriting it to `mixed` therefore keeps the effective
/// listener behaviour identical while the token catches up with the tightened
/// mapping. `http` and the three-value tokens are returned unchanged; anything
/// outside the closed set falls back to the declared default so a hand-edited
/// row cannot yield a half-dead port.
pub fn migrate_legacy_protocol_token(protocol: &str) -> &'static str {
    match canonical_protocol(protocol) {
        Some("socks5") => DEFAULT_ENTRY_PORT_PROTOCOL,
        Some(other) => other,
        None => DEFAULT_ENTRY_PORT_PROTOCOL,
    }
}

/// The v1 -> v2 protocol migration applied to a bare mapping list - the form the
/// SQLite seed and the document share. Returns true when a row changed.
pub fn migrate_legacy_port_rows(ports: &mut [PortMapping]) -> bool {
    let mut changed = false;
    for row in ports.iter_mut() {
        let migrated = migrate_legacy_protocol_token(&row.protocol);
        if migrated != row.protocol {
            row.protocol = migrated.to_string();
            changed = true;
        }
    }
    changed
}

/// Round 8 ticket 13 / D-007: one-time entry-port protocol migration over a
/// whole whitebox document.
///
/// The `version` stamp is what makes this ONE-TIME, and is why it is not a bare
/// token rewrite: once a document is v2, a `socks5` row is a DELIBERATE
/// SOCKS5-only port and must never be rewritten again. A v1 document - any
/// document written before the three-value enum - has no such intent, so every
/// `socks5` row is flag-preservingly promoted to `mixed`.
///
/// Always reports true for a v1 document (the version stamp alone is a document
/// change) and false for a v2 one, so a second pass is a no-op. Pure.
pub fn migrate_entry_port_protocols(doc: &mut WhiteboxConfig) -> bool {
    if doc.version >= WHITEBOX_CONFIG_VERSION {
        return false;
    }
    migrate_legacy_port_rows(&mut doc.entry_ports);
    doc.version = WHITEBOX_CONFIG_VERSION;
    true
}

/// Round 8 ticket 13 / D-007: the FILE form of the migration, run once at boot
/// before the document is parsed (see `WhiteboxConfigStore::open`).
///
/// Reads the raw bytes, applies `migrate_entry_port_protocols`, re-validates and
/// lands the document with an atomic write. Returns true when the file was
/// rewritten, false when it was already current (or absent), and an error only
/// when the file exists but cannot be read or parsed - in which case the
/// caller's normal parse path reports the malformed document, so a corrupt file
/// is never silently "repaired".
///
/// No generation bump: the v1 -> v2 table is flag-preserving, so the desired
/// state is unchanged and a bump would manufacture a false PendingApply / drift
/// episode on an already-converged runtime (the discipline ticket 01's
/// `migrate_b_class_values_once` follows).
pub fn migrate_entry_port_protocols_file_once(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let raw_text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut doc: WhiteboxConfig = serde_json::from_str(&raw_text)
        .map_err(|e| format!("whitebox config parse error: {e}"))?;
    if !migrate_entry_port_protocols(&mut doc) {
        return Ok(false);
    }
    validate(&doc)?;
    write_atomic(path, &doc)?;
    Ok(true)
}

pub fn process_route_conflict_check(rules: &[ProcessRouteRule]) -> Result<(), String> {
    for (i, r) in rules.iter().enumerate() {
        for other in rules.iter().skip(i + 1) {
            if other.target_port == r.target_port
                && other.process.trim().eq_ignore_ascii_case(r.process.trim())
            {
                // Same process twice on one port is caught by duplicate-name
                // validation; a DIFFERENT process on the same port conflicts.
            } else if other.target_port == r.target_port {
                return Err(format!(
                    "conflict: port {} already bound to process '{}'",
                    r.target_port, r.process
                ));
            }
        }
    }
    Ok(())
}

/// Ticket 17 / ADR-0055 D5: parse the LEGACY L1 settings.json value
/// tolerantly. Both historical wire shapes are accepted
/// (webview camelCase `targetPort` / Rust snake_case `target_port`, plus the
/// even older `target_lane`); entries that are not objects, lack a
/// non-empty process name, or carry an out-of-range port are SKIPPED, never
/// fatal. Pure; unit-tested.
pub fn parse_legacy_l1_routes(v: &serde_json::Value) -> Vec<ProcessRouteRule> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        let Some(obj) = item.as_object() else { continue };
        let Some(process) = obj
            .get("process")
            .and_then(|p| p.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.len() <= 128)
        else {
            continue;
        };
        let port = ["target_port", "targetPort", "target_lane"]
            .iter()
            .find_map(|k| obj.get(*k).and_then(|p| p.as_u64()))
            .unwrap_or(0);
        if port < MIN_USER_PORT as u64 || port > u16::MAX as u64 {
            continue;
        }
        out.push(ProcessRouteRule {
            process,
            target_port: port as u16,
        });
    }
    out
}

/// Ticket 17 / ADR-0055 D5: one-time boot migration. Merge the legacy L1
/// rules into the seed document (existing whitebox process names WIN —
/// the whitebox is already the truth source) and return the merged doc.
/// Idempotent by construction: a second pass over an empty legacy value is
/// a no-op, and the caller deletes the L1 key after the first pass, so
/// re-migration cannot re-introduce rules. Pure; unit-tested.
pub fn migrate_l1_process_routes(
    seed: &mut WhiteboxConfig,
    legacy: Option<&serde_json::Value>,
) -> bool {
    let Some(v) = legacy else {
        return false;
    };
    let rules = parse_legacy_l1_routes(v);
    if rules.is_empty() {
        return false;
    }
    let existing: HashSet<String> = seed
        .process_routes
        .iter()
        .map(|r| r.process.trim().to_lowercase())
        .collect();
    let mut changed = false;
    for r in rules {
        if existing.contains(&r.process.trim().to_lowercase()) {
            continue;
        }
        seed.process_routes.push(r);
        changed = true;
    }
    changed
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
        // Round 8 ticket 13 / D-007: one-time entry-port protocol migration.
        // Runs BEFORE the file is parsed so the loaded document already carries
        // the tightened three-value vocabulary. Best-effort: a malformed file is
        // left for the parse path below to report, so the caller's
        // quarantine-and-reseed behaviour is unchanged.
        if path.exists() {
            match migrate_entry_port_protocols_file_once(&path) {
                Ok(true) => tracing::info!(
                    target: "whitebox",
                    "entry-port protocol migrated to the three-value enum (legacy socks5 -> mixed; one-time)"
                ),
                Ok(false) => {}
                Err(e) => tracing::warn!(
                    target: "whitebox",
                    error = %e,
                    "entry-port protocol migration skipped; the parse path reports a malformed whitebox"
                ),
            }
        }
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
        mut next: WhiteboxConfig,
    ) -> Result<usize, String> {
        // Round 8 ticket 13 / D-007: a document that still declares the v1
        // vocabulary (an imported or restored legacy export) gets the
        // flag-preserving `socks5` -> `mixed` rewrite on the way in, so the
        // tightened mapping can never reinterpret it. A v2 document is left
        // alone: there `socks5` is a deliberate SOCKS5-only port.
        migrate_entry_port_protocols(&mut next);
        validate(&next)?;
        let _guard = self.writer.lock().await;
        // Round 5 T09 / ADR-0058 (D-27): single-generation counter bump —
        // every ACCEPTED apply advances the ports write-authority generation
        // past the CURRENT committed value (not the incoming doc's: callers
        // hand-build documents that may not carry the latest counter). The
        // swap below may still fail after this (rollback restores the
        // previous config), but a rejected-because-invalid file never bumps:
        // validate() ran first.
        next.version = WHITEBOX_CONFIG_VERSION;
        next.generation = self.snapshot().generation.wrapping_add(1);
        next.updated_at = Some(now_unix());
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

    /// ADR-0054 section B: list the versioned backups of this whitebox file,
    /// newest first.
    pub fn list_backups(&self) -> Result<Vec<WhiteboxBackupEntry>, String> {
        backup_list(&self.path)
    }

    /// ADR-0054 section B: roll the whitebox file back to a listed backup.
    /// The backup content is parsed and then re-enters the SAME validate ->
    /// DB/listeners -> file -> swap chain as a hand edit (apply), never
    /// bypassing ADR-0042 entries. The rollback write is itself backed up,
    /// so a rollback is reversible.
    pub async fn rollback_to_backup(
        &self,
        db: &DbPool,
        forwarder: &PortForwarder,
        backup_name: &str,
    ) -> Result<usize, String> {
        let next: WhiteboxConfig = read_backup_parsed(&self.path, backup_name)?;
        self.apply(db, forwarder, next).await
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

/// Atomic whitebox write with versioning (ADR-0054 section B): the current
/// file is copied to backup/ and rotated BEFORE the new bytes replace it.
/// Callers have already validated; a failed backup aborts the write.
/// Round 5 T11 / ADR-0059 audit: records a best-effort row with the
/// before/after content hashes (never propagates; rollback context comes
/// from the AUDIT_CTX task-local set by the whitebox_rollback IPC command).
fn write_atomic(path: &Path, config: &WhiteboxConfig) -> Result<(), String> {
    let current = std::fs::read(path).unwrap_or_default();
    let bytes =
        serde_json::to_vec_pretty(config).map_err(|e| format!("encode whitebox config: {e}"))?;
    let before_hash = crate::audit::sha256_hex(&current);
    let after_hash = crate::audit::sha256_hex(&bytes);
    let before_bytes = current.len() as u64;
    let after_bytes = bytes.len() as u64;
    let write_result = (|| -> Result<(), String> {
        backup_before_write(path, now_unix())?;
        atomic_write_bytes(path, &bytes)
    })();
    let ac = crate::audit::ctx();
    let mut ev = crate::audit::event(
        "L2:ports",
        ac.op.as_deref().unwrap_or("apply"),
        ac.actor.as_deref().unwrap_or("whitebox:apply"),
        before_hash,
        after_hash,
        if write_result.is_ok() { "ok" } else { "error:write failed" },
        Some(before_bytes),
        Some(after_bytes),
    );
    ev.source_backup = ac.source_backup;
    ev.reason = ac.reason;
    let _ = crate::audit::append(&ev);
    write_result
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

    // ---- ticket 17 / ADR-0055: process routes family ----

    fn route(process: &str, port: u16) -> ProcessRouteRule {
        ProcessRouteRule {
            process: process.to_string(),
            target_port: port,
        }
    }

    #[test]
    fn route_validation_rejects_conflict_duplicate_and_privileged() {
        let mut cfg = WhiteboxConfig::from_ports(vec![]);
        cfg.process_routes = vec![route("app.exe", 17990), route("other.exe", 17990)];
        let err = validate(&cfg).unwrap_err();
        assert!(err.contains("already bound to process"), "got: {err}");

        cfg.process_routes = vec![route("app.exe", 17990), route("APP.EXE", 17991)];
        assert!(validate(&cfg).unwrap_err().contains("duplicate process route"));

        cfg.process_routes = vec![route("app.exe", 80)];
        assert!(validate(&cfg).unwrap_err().contains("privileged"));

        cfg.process_routes = vec![route("", 17990)];
        assert!(validate(&cfg).unwrap_err().contains("1..128"));

        cfg.process_routes = (0..257).map(|i| route(&format!("p{i}"), 20000 + i as u16)).collect();
        assert!(validate(&cfg).unwrap_err().contains("too many process routes"));
    }

    #[test]
    fn route_validation_accepts_distinct_and_reused_ports() {
        let mut cfg = WhiteboxConfig::from_ports(vec![]);
        // same process on DIFFERENT ports is an update shape, allowed at doc level
        cfg.process_routes = vec![route("app.exe", 17990), route("other.exe", 17991)];
        assert!(validate(&cfg).is_ok());
        // same port listed once is fine
        cfg.process_routes = vec![route("solo.exe", 17990)];
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn legacy_l1_parser_accepts_both_wire_shapes_and_skips_junk() {
        let v: serde_json::Value = serde_json::json!([
            {"process": "webview.exe", "targetPort": 17990},
            {"process": "rust.exe", "target_port": 17991},
            {"process": "old.exe", "target_lane": 17992},
            {"process": "", "target_port": 17990},
            {"target_port": 17990},
            {"process": "badport.exe", "target_port": 80},
            {"process": "badport2.exe", "target_port": 70000},
            "junk",
            42
        ]);
        let rules = parse_legacy_l1_routes(&v);
        let got: Vec<(String, u16)> = rules.iter().map(|r| (r.process.clone(), r.target_port)).collect();
        assert_eq!(got, vec![
            ("webview.exe".into(), 17990),
            ("rust.exe".into(), 17991),
            ("old.exe".into(), 17992),
        ]);
        // non-array payloads yield empty
        assert!(parse_legacy_l1_routes(&serde_json::json!({"a": 1})).is_empty());
        assert!(parse_legacy_l1_routes(&serde_json::json!(null)).is_empty());
    }

    #[test]
    fn migration_is_idempotent_and_whitebox_wins() {
        let legacy = serde_json::json!([
            {"process": "a.exe", "target_port": 17990},
            {"process": "b.exe", "targetPort": 17991}
        ]);
        let mut seed = WhiteboxConfig::from_ports(vec![]);
        assert!(migrate_l1_process_routes(&mut seed, Some(&legacy)));
        assert_eq!(seed.process_routes.len(), 2);
        let snapshot_after_first = seed.clone();

        // Second boot: legacy value gone (None) => no change.
        let mut seed2 = snapshot_after_first.clone();
        assert!(!migrate_l1_process_routes(&mut seed2, None));
        assert_eq!(seed2, snapshot_after_first);

        // Even a REPLAYED legacy value is a no-op (whitebox wins per name).
        let mut seed3 = snapshot_after_first.clone();
        assert!(!migrate_l1_process_routes(&mut seed3, Some(&legacy)));
        assert_eq!(seed3, snapshot_after_first);

        // A whitebox-owned rule keeps its port when the legacy value has the same name.
        let mut seed4 = WhiteboxConfig::from_ports(vec![]);
        seed4.process_routes = vec![route("a.exe", 19999)];
        let whitebox_wins = serde_json::json!([{"process": "a.exe", "target_port": 17990}]);
        assert!(!migrate_l1_process_routes(&mut seed4, Some(&whitebox_wins)));
        assert_eq!(seed4.process_routes[0].target_port, 19999);
    }

    #[test]
    fn route_acknowledged_validation_applies() {
        let mut cfg = WhiteboxConfig::from_ports(vec![]);
        cfg.route_acknowledged = vec!["a.exe".into(), "a.exe".into()];
        assert!(validate(&cfg).unwrap_err().contains("duplicated"));
        cfg.route_acknowledged = vec![String::new()];
        assert!(validate(&cfg).unwrap_err().contains("1..128"));
    }

    #[test]
    fn old_document_without_route_fields_parses_unchanged() {
        // pre-ticket-17 file shape: no process_routes / route_acknowledged keys
        let raw = serde_json::json!({
            "version": 1,
            "entry_ports": [],
            "network": {}
        });
        let cfg: WhiteboxConfig = serde_json::from_value(raw).unwrap();
        assert!(cfg.process_routes.is_empty());
        assert!(cfg.route_acknowledged.is_empty());
        // and serialization omits the empty fields (wire compat both ways)
        let v = serde_json::to_value(&cfg).unwrap();
        assert!(v.get("process_routes").is_none());
        assert!(v.get("route_acknowledged").is_none());
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

    #[tokio::test]
    async fn apply_backs_up_previous_file_and_rollback_restores_it() {
        let dir =
            std::env::temp_dir().join(format!("egressapikey-wb-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(WHITEBOX_CONFIG_FILE);
        let db = DbPool::open_in_memory().unwrap();
        let forwarder = PortForwarder::new(db.clone(), "127.0.0.1", 1, "");
        // First store creation: file did not exist, so no backup is made.
        let store = WhiteboxConfigStore::open(
            path.clone(),
            WhiteboxConfig::from_ports(vec![mapping(17990)]),
        )
        .await
        .unwrap();
        assert!(store.list_backups().unwrap().is_empty());

        // Second write: the previous file is backed up before the swap.
        let next = WhiteboxConfig::from_ports(vec![mapping(17991)]);
        store.apply(&db, &forwarder, next).await.unwrap();
        assert_eq!(store.snapshot().entry_ports[0].port, 17991);
        let backups = store.list_backups().unwrap();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].size_bytes > 0);

        // Rollback re-enters validate -> apply; the restored state is 17990.
        let restored = store
            .rollback_to_backup(&db, &forwarder, &backups[0].file_name)
            .await
            .unwrap();
        assert_eq!(restored, 1);
        assert_eq!(store.snapshot().entry_ports[0].port, 17990);
        // The rollback write itself was backed up (reversible).
        assert_eq!(store.list_backups().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rollback_rejects_tampered_backup_content_without_swapping() {
        let dir =
            std::env::temp_dir().join(format!("egressapikey-wb-tamper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(WHITEBOX_CONFIG_FILE);
        let db = DbPool::open_in_memory().unwrap();
        let forwarder = PortForwarder::new(db.clone(), "127.0.0.1", 1, "");
        let store = WhiteboxConfigStore::open(
            path.clone(),
            WhiteboxConfig::from_ports(vec![mapping(17990)]),
        )
        .await
        .unwrap();
        store
            .apply(
                &db,
                &forwarder,
                WhiteboxConfig::from_ports(vec![mapping(17991)]),
            )
            .await
            .unwrap();
        let backup_name = store.list_backups().unwrap()[0].file_name.clone();
        // Tamper with the listed backup: not JSON at all.
        std::fs::write(
            crate::whitebox_backup::backup_dir(&path).join(&backup_name),
            b"not-json",
        )
        .unwrap();
        assert!(store
            .rollback_to_backup(&db, &forwarder, &backup_name)
            .await
            .is_err());
        // The active config is unchanged after the rejected rollback.
        assert_eq!(store.snapshot().entry_ports[0].port, 17991);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round 5 T09 / ADR-0058 (D-27): every ACCEPTED apply bumps the ports
    /// single-generation counter and stamps updated_at. A v1 file (no
    /// generation fields) loads with generation=0 — serde(default) zero
    /// migration, and apply #1 lands at generation=1.
    #[tokio::test]
    async fn apply_bumps_generation_and_v1_file_loads_at_zero() {
        let dir =
            std::env::temp_dir().join(format!("egressapikey-wb-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(WHITEBOX_CONFIG_FILE);
        let db = DbPool::open_in_memory().unwrap();
        let forwarder = PortForwarder::new(db.clone(), "127.0.0.1", 1, "");
        // Seed from a v1-shaped document (no generation/updated_at keys).
        let v1_doc: WhiteboxConfig =
            serde_json::from_str(r#"{"version":1,"entry_ports":[]}"#).unwrap();
        assert_eq!(v1_doc.generation, 0);
        let store = WhiteboxConfigStore::open(path.clone(), v1_doc).await.unwrap();
        assert_eq!(store.snapshot().generation, 0);
        assert_eq!(store.snapshot().updated_at, None);

        store
            .apply(&db, &forwarder, WhiteboxConfig::from_ports(vec![mapping(17990)]))
            .await
            .unwrap();
        let after1 = store.snapshot();
        assert_eq!(after1.generation, 1);
        assert!(after1.updated_at.is_some());

        store
            .apply(&db, &forwarder, WhiteboxConfig::from_ports(vec![mapping(17991)]))
            .await
            .unwrap();
        store
            .apply(&db, &forwarder, WhiteboxConfig::from_ports(vec![mapping(17991)]))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        let after2 = store.snapshot();
        assert!(
            after2.generation >= 2 && after2.generation >= after1.generation,
            "generation must advance monotonically: after1={}, after2={}",
            after1.generation,
            after2.generation
        );
        assert!(after2.updated_at.unwrap() >= after1.updated_at.unwrap());

        // NO applied_generation on the ports half (D-27: local type is
        // single-generation — the field must not exist on this struct).
        // Verified by compilation of the struct definition itself.
        let _ = std::fs::remove_dir_all(&dir);
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

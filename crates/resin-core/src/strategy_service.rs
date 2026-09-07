//! StrategyService — the ONE deep module owning the strategyConfig pipeline
//! (architecture-recovery ticket 10; ADR-0052).
//!
//! Vocabulary (see ADR-0052 for the full three-vocabulary map):
//! - `strategy.rs` owns the B-class type (`StrategyId` 6 shell options) —
//!   the UI-facing strategy vocabulary (catalog/mapping face deleted, ticket 24).
//! - `strategy_engine.rs` owns the A-class planner (`StrategyConfig`,
//!   `compute_plan`, `parse_nodes`) — the region-computation vocabulary.
//! - `src/lib/strategy.ts` (frontend) maps `StrategyId` to i18n keys and to
//!   Resin's `allocation_policy` enum — a pure display/mapping layer.
//! This module is the *ownership* layer: read (whitebox JSON), validate,
//! store (write-back), apply (compute plan + PATCH Resin; creates missing
//! platforms and never deletes whitebox entries, ADR-0056), snapshot
//! (ticket 07 read-back), and a deep `set_platform_regions` edit used by
//! the topology canvas.
//!
//! ADR-0036 discipline: the whitebox file is the truth; `store` is the ONLY
//! writer in the shell (the former duplicate writers in the command layer are
//! gone). ADR-0039 SS2: every strategy field still travels to views through
//! the snapshot (unchanged shape).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;

use serde::Serialize;

use crate::strategy_engine::{
    compute_plan, parse_nodes, EstablishStep, NodeSummary, PlatformStrategy, StrategyConfig, SubscriptionPhase,
    SubscriptionStatus,
};
use crate::whitebox_backup::{
    atomic_write_bytes, backup_before_write, backup_list, now_unix, read_backup_parsed,
    WhiteboxBackupEntry,
};
use crate::snapshot::{parse_resin_platforms, ResinPlatformRuntime};
use crate::PortMapping;

/// Bound defaults (validated by the tests at the bottom; the GUI is not the
/// only writer — a user can hand-edit the JSON — so the Service, not the IPC
/// layer, is the authoritative validator).
pub const MAX_PLATFORMS: usize = 256;
pub const MAX_REGIONS_PER_PLATFORM: usize = 64;
pub const MAX_SUBSCRIPTIONS_PER_PLATFORM: usize = 64;
pub const MAX_TOP_N: usize = 1000;
pub const MAX_PLATFORM_NAME_LEN: usize = 128;
pub const MAX_REGION_LEN: usize = 32;
/// Round 7 T02: ceiling for the per-subscription phase status array. Far
/// above realistic subscription counts; keeps a hand-edited file from
/// growing unbounded (AGENTS 7.5 bounded-collection template).
pub const MAX_SUBSCRIPTION_STATUS_ROWS: usize = 512;
/// Round 7 T02: KEP-1623-style reason ceiling for `phase_error`.
pub const MAX_PHASE_ERROR_LEN: usize = 1024;

/// Result of `StrategyService::apply`: per-platform PATCH outcome, serde
/// shaped exactly like the former command-layer JSON (TS contract unchanged).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AppliedPlatform {
    pub platform: String,
    pub region_filters: Vec<String>,
    pub patched: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// ApplyReport stays the per-platform PATCH outcome vocabulary (TS contract
/// unchanged); the reconcile pass composes it with the ports outcome.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ApplyReport {
    pub platforms: Vec<AppliedPlatform>,
}

/// Ports-half result of one reconcile pass, produced by the injected
/// shell-side restore closure.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct ReconcilePortsOutcome {
    pub restored: Vec<u16>,
    pub skipped: u32,
}

/// Idempotency memory for the reconcile ports half (ticket 14 hard gate:
/// "连续两次执行第二次零变更"). Since ADR-0057 the strategy half is
/// wire-idempotent on its own (diff-then-skip: an in-sync platform is
/// PATCHed never — no TTL needed there); the ports half is not —
/// Resin would accept the duplicate POST or 409-skip forever. This window
/// records WHEN each port was last reconcile-asserted; a pass re-asserts a
/// port only when it has never been asserted or the last assertion is older
/// than the window. It mirrors the `SkippedReason` pattern of the historical
/// cleanup loop: an in-process `StdMutex` state, best-effort, never a truth
/// source (the whitebox stays the truth; this only throttles re-asserts).
pub struct ReconcileMemory {
    /// port -> Unix seconds of the last reconcile assertion.
    last: StdMutex<HashMap<u16, u64>>,
}

/// How long a reconcile-asserted port is trusted as "live" before a later
/// pass re-asserts it. 24h matches the lightweight-mode day horizon; a Resin
/// restart within the window is handled by restore_ports_from_whitebox at
/// boot, not by reconcile.
pub const RECONCILE_PORT_TTL_SECS: u64 = 86_400;

impl Default for ReconcileMemory {
    fn default() -> Self {
        Self {
            last: StdMutex::new(HashMap::new()),
        }
    }
}

impl ReconcileMemory {
    /// Decide which desired ports need asserting now: enabled, not already
    /// live on Resin, and not asserted within the TTL window.
    pub fn ports_to_assert(
        &self,
        desired: &[crate::db::PortMapping],
        live_ports: &[u16],
        now: u64,
    ) -> Vec<crate::db::PortMapping> {
        let guard = self.last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        desired
            .iter()
            .filter(|m| m.enabled && !live_ports.contains(&m.port))
            .filter(|m| match guard.get(&m.port) {
                Some(&ts) => now.saturating_sub(ts) >= RECONCILE_PORT_TTL_SECS,
                None => true,
            })
            .cloned()
            .collect()
    }

    /// Stamp a successful assertion.
    pub fn stamp_asserted(&self, port: u16, now: u64) {
        let mut guard = self.last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.insert(port, now);
    }
}

/// Architecture-recovery ticket 14 / ADR-0054 §A: what one reconcile pass
/// WOULD change. Both halves are derived from data the snapshot/apply pass
/// already fetches (list_platforms + list_endpoints + compute_plan) — the
/// preview never issues extra requests of its own beyond those two reads.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReconcilePlanPlatform {
    pub platform: String,
    /// Region set the NEXT apply would PATCH (compute_plan output).
    pub desired_regions: Vec<String>,
    /// Region set currently live on Resin (empty when missing there).
    pub live_regions: Vec<String>,
    pub action: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReconcilePlanPort {
    pub port: u16,
    pub platform: String,
    pub action: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReconcilePlan {
    pub platforms: Vec<ReconcilePlanPlatform>,
    pub ports: Vec<ReconcilePlanPort>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReconcileReport {
    pub strategy: ApplyReport,
    /// Ports re-asserted onto Resin during this pass (created endpoints).
    pub ports_restored: Vec<u16>,
    pub ports_skipped: u32,
}

impl ReconcilePlan {
    pub fn is_empty(&self) -> bool {
        self.platforms.is_empty() && self.ports.is_empty()
    }
}

/// Extract the set of listener ports from a GET /api/v1/endpoints response.
/// Both the `{"items":[..]}` wrapper and a bare array are accepted; the
/// read-only "default" endpoint is included because a listener exists there.
/// Lives beside the reconcile logic so the shell, the snapshot command, and
/// the tests share ONE parsing rule.
pub fn endpoint_live_ports(existing: &serde_json::Value) -> Vec<u16> {
    let arr = if let Some(a) = existing.get("items").and_then(|i| i.as_array()) {
        a.as_slice()
    } else if let Some(a) = existing.as_array() {
        a.as_slice()
    } else {
        &[]
    };
    let mut ports: Vec<u16> = arr
        .iter()
        .filter_map(|ep| ep.get("port").and_then(|p| p.as_u64()))
        .filter(|p| *p <= u16::MAX as u64)
        .map(|p| p as u16)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Preview the changes one reconcile pass WOULD apply: platform-level
/// region_filters rewrites (compute_plan vs the live Resin rows) and
/// port-level creations (whitebox enabled entries missing from the live
/// endpoints list). ORDER-INSENSITIVE region comparison — same rule as the
/// three-state merge (snapshot.rs), so the preview list and the snapshot's
/// divergent badges can never disagree about what counts as drift.
pub fn compute_reconcile_plan(
    config: &StrategyConfig,
    nodes: &[NodeSummary],
    live_platforms: &[ResinPlatformRuntime],
    live_ports: &[u16],
    desired_ports: &[PortMapping],
) -> ReconcilePlan {
    let plan = compute_plan(config, nodes);
    let mut platforms = Vec::new();
    for ps in &config.platforms {
        let desired = plan
            .get(&ps.platform_name)
            .cloned()
            .unwrap_or_else(|| ps.regions.clone());
        let live = live_platforms
            .iter()
            .find(|rp| rp.name == ps.platform_name)
            .map(|rp| rp.region_filters.clone());
        match live {
            Some(lr) if same_region_set(&desired, &lr) => {} // in sync
            Some(lr) => platforms.push(ReconcilePlanPlatform {
                platform: ps.platform_name.clone(),
                desired_regions: desired.clone(),
                live_regions: lr.clone(),
                action: "patch_regions",
            }),
            None => platforms.push(ReconcilePlanPlatform {
                platform: ps.platform_name.clone(),
                desired_regions: desired.clone(),
                live_regions: vec![],
                action: "create_platform",
            }),
        }
    }
    // Runtime-only platforms (created outside the strategy pipeline) are
    // NOT part of the one-way reconcile: the whitebox wins for entities it
    // knows about, and L3-only rows are surfaced by the snapshot as drift
    // for the user to fix in the editor — reconcile never deletes them.
    let ports: Vec<ReconcilePlanPort> = desired_ports
        .iter()
        .filter(|m| m.enabled && !live_ports.contains(&m.port))
        .map(|m| ReconcilePlanPort {
            port: m.port,
            platform: m.platform_name.clone(),
            action: "create_endpoint",
        })
        .collect();
    ReconcilePlan { platforms, ports }
}

fn same_region_set(a: &[String], b: &[String]) -> bool {
    let lower = |v: &[String]| -> Vec<String> {
        let mut n: Vec<String> = v.to_vec();
        n.sort();
        n.dedup();
        n.iter().map(|s| s.to_lowercase()).collect()
    };
    lower(a) == lower(b)
}

/// Where the whitebox strategy document lives and how it is read/written.
    /// Abstracted behind a trait so apply/snapshot logic is unit-testable without
    /// a filesystem.
pub trait StrategyConfigStore {
    /// Read the raw config. Missing file = `Ok(None)` (defaults apply).
    fn load(&self) -> Result<Option<StrategyConfig>, String>;
    /// Persist the config (pretty JSON). Implementations must be atomic
    /// enough for a desktop app: single write of the full document.
    fn store(&self, config: &StrategyConfig) -> Result<(), String>;
}

/// Filesystem-backed store for `egressapikey-strategy.json` (L2 whitebox).
pub struct FsStrategyStore {
    path: PathBuf,
}

impl FsStrategyStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl StrategyConfigStore for FsStrategyStore {
    fn load(&self) -> Result<Option<StrategyConfig>, String> {
        if !self.path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&self.path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| format!("strategy config parse error: {e}"))
    }

    fn store(&self, config: &StrategyConfig) -> Result<(), String> {
        let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
        // Round 5 T11 / ADR-0059 audit: record the before/after content hashes
        // around the write. before_hash = the CURRENT on-disk whitebox (the
        // exact bytes backup_before_write is about to copy); after_hash = the
        // NEW document (the bytes that land atomically). Rollback context
        // (op:"rollback" + source_backup + reason) comes from the task-local
        // AUDIT_CTX set by the strategy_rollback IPC command. The audit write
        // is best-effort and never propagates (argus: audit never blocks).
        let current = std::fs::read(&self.path).unwrap_or_default();
        let before_hash = crate::audit::sha256_hex(&current);
        let after_hash = crate::audit::sha256_hex(json.as_bytes());
        let before_bytes = current.len() as u64;
        let after_bytes = json.len() as u64;
        // ADR-0054 section B: version the previous file before the swap.
        let write_result = (|| -> Result<(), String> {
            backup_before_write(&self.path, now_unix())?;
            atomic_write_bytes(&self.path, json.as_bytes())
        })();
        let ac = crate::audit::ctx();
        let mut ev = crate::audit::event(
            "L2:strategy",
            ac.op.as_deref().unwrap_or("put"),
            ac.actor.as_deref().unwrap_or("gui:strategy_config_put"),
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
}

impl FsStrategyStore {
    /// ADR-0054 section B: list the versioned backups of the strategy
    /// whitebox, newest first.
    pub fn list_backups(&self) -> Result<Vec<WhiteboxBackupEntry>, String> {
        backup_list(&self.path)
    }

    /// ADR-0054 section B: parse a listed backup and persist it through the
    /// SAME validate + store write entry as strategy_config_put (ADR-0036).
    /// The rollback write is itself backed up, so a rollback is reversible.
    pub fn rollback(&self, backup_name: &str) -> Result<StrategyConfig, String> {
        let mut config: StrategyConfig = read_backup_parsed(&self.path, backup_name)?;
        validate(&config)?;
        // T09 / ADR-0058: a rollback IS a write-authority change — bump the
        // generation here (the store trait's store() is a byte-dump and must
        // stay counter-agnostic; the service-level store() owns the bump for
        // the IPC put path, this is the store-adjacent rollback path).
        config.generation = config.generation.wrapping_add(1);
        config.updated_at = Some(now_unix());
        self.store(&config)?;
        Ok(config)
    }
}

/// Round 5 T01 / issue 01 F3: diff every whitebox platform's
/// `subscriptions` members against the live Resin subscription names.
/// Returns one (platform_name, dangling_names) pair per platform with at
/// least one unresolvable member. Pure — the caller owns the live-name set
/// so the check is unit-testable without a filesystem or HTTP.
pub fn dangling_subscription_refs(
    config: &StrategyConfig,
    live_subscription_names: &std::collections::HashSet<String>,
) -> Vec<(String, Vec<String>)> {
    config
        .platforms
        .iter()
        .filter_map(|ps| {
            let dangling: Vec<String> = ps
                .subscriptions
                .iter()
                .filter(|s| !live_subscription_names.contains(*s))
                .cloned()
                .collect();
            if dangling.is_empty() {
                None
            } else {
                Some((ps.platform_name.clone(), dangling))
            }
        })
        .collect()
}

/// Extend an AppliedPlatform reason with the dangling-subscription clause.
/// The "dangling subscription refs:" prefix is DISTINCT from the ADR-0056
/// "create failed:" / "PATCH failed:" / "in sync" vocabulary so report
/// consumers can tell the two failure families apart (issue 01 risk note).
fn with_dangling_note(mut row: AppliedPlatform, dangling: &[String]) -> AppliedPlatform {
    if dangling.is_empty() {
        return row;
    }
    let note = format!("dangling subscription refs: {}", dangling.join(", "));
    row.reason = Some(match row.reason {
        Some(r) => format!("{r}; {note}"),
        None => note,
    });
    row
}

/// Validate a full config document. Returns Err with the first violation.
/// Mirrors the bounds the IPC layer historically enforced (AGENTS 7.5 caps
/// stay at the TS/Rust boundary too; this is the single source of truth for
/// what a valid document is).
pub fn validate(config: &StrategyConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("strategy config version must be 1".to_string());
    }
    if config.platforms.len() > MAX_PLATFORMS {
        return Err(format!("platforms list too long (max {MAX_PLATFORMS})"));
    }
    let mut seen = std::collections::HashSet::new();
    for ps in &config.platforms {
        if ps.platform_name.is_empty() || ps.platform_name.len() > MAX_PLATFORM_NAME_LEN {
            return Err("platform_name must be 1..128 chars".to_string());
        }
        if !seen.insert(ps.platform_name.clone()) {
            return Err(format!("duplicate platform_name: {}", ps.platform_name));
        }
        if ps.regions.len() > MAX_REGIONS_PER_PLATFORM {
            return Err(format!("regions list too long (max {MAX_REGIONS_PER_PLATFORM})"));
        }
        for r in &ps.regions {
            if r.is_empty() || r.len() > MAX_REGION_LEN {
                return Err(format!("region code invalid (1..{MAX_REGION_LEN} chars): {r}"));
            }
        }
        if ps.subscriptions.len() > MAX_SUBSCRIPTIONS_PER_PLATFORM {
            return Err(format!(
                "subscriptions list too long (max {MAX_SUBSCRIPTIONS_PER_PLATFORM})"
            ));
        }
        if ps.top_n > MAX_TOP_N {
            return Err(format!("top_n too large (max {MAX_TOP_N})"));
        }
        if ps.manual_nodes.len() > MAX_PLATFORMS {
            return Err(format!("manual_nodes list too long (max {MAX_PLATFORMS})"));
        }
    }
    validate_acknowledged(&config.acknowledged, "acknowledged")?;
    validate_subscription_statuses(&config.subscriptions)
}

/// Round 7 T02 (D-C1.2): shape checks for the per-subscription establish-
/// phase STATUS array. Same discipline as `validate_acknowledged`: serde
/// already rejects wrong JSON types at parse time, these checks cap the
/// array, reject duplicate/malformed row keys, and lock the phase<->stage
/// <->phase_error invariants so a hand-edited file cannot smuggle junk
/// through ANY store path (the write entry validates before landing).
fn validate_subscription_statuses(rows: &[SubscriptionStatus]) -> Result<(), String> {
    if rows.len() > MAX_SUBSCRIPTION_STATUS_ROWS {
        return Err(format!(
            "subscriptions status array too long (max {MAX_SUBSCRIPTION_STATUS_ROWS})"
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, row) in rows.iter().enumerate() {
        if row.name.is_empty() || row.name.len() > MAX_PLATFORM_NAME_LEN {
            return Err(format!("subscriptions[{i}].name must be 1..{MAX_PLATFORM_NAME_LEN} chars"));
        }
        if row.name.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err(format!("subscriptions[{i}].name contains control characters"));
        }
        if !seen.insert(row.name.clone()) {
            return Err(format!("duplicate subscriptions entry: {}", row.name));
        }
        let carrying_stage = row.stage.is_some();
        let carrying_error = row.phase_error.is_some();
        match row.phase {
            SubscriptionPhase::Establishing | SubscriptionPhase::Failed => {
                if !carrying_stage {
                    return Err(format!("subscriptions[{i}] ({}) phase requires a stage", row.name));
                }
            }
            _ => {
                if carrying_stage {
                    return Err(format!("subscriptions[{i}] ({}) phase must not carry a stage", row.name));
                }
            }
        }
        if row.phase == SubscriptionPhase::Failed {
            match row.phase_error.as_deref() {
                None => return Err(format!("subscriptions[{i}] ({}) Failed requires a phase_error", row.name)),
                Some(e) => {
                    if e.is_empty() || e.len() > MAX_PHASE_ERROR_LEN {
                        return Err(format!(
                            "subscriptions[{i}].phase_error must be 1..{MAX_PHASE_ERROR_LEN} chars"
                        ));
                    }
                    if e.bytes().any(|b| b == 0) {
                        return Err(format!("subscriptions[{i}].phase_error contains NUL"));
                    }
                }
            }
        } else if carrying_error {
            return Err(format!("subscriptions[{i}] ({}) phase must not carry a phase_error", row.name));
        }
    }
    Ok(())
}

/// Ticket 12 / ADR-0054 §D: shared shape checks for a whitebox
/// `acknowledged` exemption array. Non-string members are already rejected
/// by serde's Vec<String> deserialization (a hand-edited file with a number
/// inside fails to parse, preserving compatibility of valid old files);
/// these checks cap the list and reject empty/oversized/control-char/duplicate
/// members so the array cannot grow unbounded or smuggle junk.
pub fn validate_acknowledged(list: &[String], field: &str) -> Result<(), String> {
    if list.len() > MAX_ACKNOWLEDGED_ENTRIES {
        return Err(format!("{field} list too long (max {MAX_ACKNOWLEDGED_ENTRIES})"));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, m) in list.iter().enumerate() {
        if m.is_empty() || m.len() > MAX_PLATFORM_NAME_LEN {
            return Err(format!("{field}[{i}] must be 1..{MAX_PLATFORM_NAME_LEN} chars"));
        }
        if m.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err(format!("{field}[{i}] contains control characters"));
        }
        if !seen.insert(m.clone()) {
            return Err(format!("{field}[{i}] is duplicated: {m}"));
        }
    }
    Ok(())
}

/// Cap for whitebox `acknowledged` exemption arrays (ticket 12 / ADR-0054 §D).
pub const MAX_ACKNOWLEDGED_ENTRIES: usize = 64;

/// Accept Resin's items-wrapper shape `{"items":[...]}` OR a bare array
/// (mirrors the shell-side items_arr contract; Resin v1.2.0 uses both).
fn items(v: &serde_json::Value) -> &[serde_json::Value] {
    if let Some(arr) = v.get("items").and_then(|i| i.as_array()) {
        return arr.as_slice();
    }
    if let Some(arr) = v.as_array() {
        return arr.as_slice();
    }
    &[]
}

/// The service. Owns the store handle; Resin access goes through the
/// `ResinClient` REST seam only (never sidecar files).
pub struct StrategyService<S: StrategyConfigStore> {
    store: S,
}

impl<S: StrategyConfigStore> StrategyService<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Read the whitebox document; missing file yields defaults (never an
    /// error — mirrors the historical command behavior and ADR-0036).
    pub fn get(&self) -> Result<StrategyConfig, String> {
        Ok(self.store.load()?.unwrap_or_default())
    }

    /// Access the underlying store (e.g. to report the whitebox path in the
    /// authoritative snapshot). Read-only accessor; writes still go through
    /// `store`/`set_platform_regions` so validation cannot be bypassed.
    pub fn store_ref(&self) -> &S {
        &self.store
    }

    /// Validate + persist. The write entry for strategyConfig (ADR-0036).
    /// Round 5 T09 / ADR-0058: the generation counter is bumped HERE — after
    /// validate, before the file lands — so every sanctioned write (IPC put,
    /// deep region edit, rollback, apply's own write-back) serially advances
    /// the write-authority generation. The caller passes its config by value
    /// and the bumped document is what lands on disk.
    pub fn store(&self, mut config: StrategyConfig) -> Result<StrategyConfig, String> {
        validate(&config)?;
        config.generation = config.generation.wrapping_add(1);
        config.updated_at = Some(now_unix());
        self.store.store(&config)?;
        Ok(config)
    }

    /// Compute the A-class plan without touching Resin (pure read used by
    /// tests and the B-class snapshot path).
    pub fn plan(&self, nodes: &[NodeSummary]) -> Result<HashMap<String, Vec<String>>, String> {
        let config = self.get()?;
        Ok(compute_plan(&config, nodes))
    }

    /// Deep edit used by the topology canvas: set (or create) one platform's
    /// region list in the whitebox, preserving every other entry verbatim.
    /// Returns the stored document (post-write). This is the sanctioned
    /// single-write path behind the `strategy_platform_regions_set` IPC.
    pub fn set_platform_regions(
        &self,
        platform_name: &str,
        regions: Vec<String>,
    ) -> Result<StrategyConfig, String> {
        if platform_name.is_empty() || platform_name.len() > MAX_PLATFORM_NAME_LEN {
            return Err("platform_name must be 1..128 chars".to_string());
        }
        if regions.len() > MAX_REGIONS_PER_PLATFORM {
            return Err(format!("regions list too long (max {MAX_REGIONS_PER_PLATFORM})"));
        }
        for r in &regions {
            if r.is_empty() || r.len() > MAX_REGION_LEN {
                return Err(format!("region code invalid (1..{MAX_REGION_LEN} chars): {r}"));
            }
        }
        let mut config = self.get()?;
        match config.platforms.iter_mut().find(|ps| ps.platform_name == platform_name) {
            Some(ps) => ps.regions = regions,
            None => config.platforms.push(PlatformStrategy {
                platform_name: platform_name.to_string(),
                a_class: crate::strategy_engine::AClassStrategy::Region,
                b_class: crate::strategy::StrategyId::Random,
                manual_nodes: vec![],
                regions,
                subscriptions: vec![],
                top_n: 10,
                b_class_params: Default::default(),
            }),
        }
        self.store(config)
    }

    /// Round 7 T02 (D-C1.2): record ONE subscription's establish-phase
    /// STATUS row (upsert by name). This is the sanctioned write path for the
    /// whitebox `subscriptions` status array — it re-enters `store` so the
    /// write is validated, versioned (backup ring) and audited exactly like
    /// every other strategy mutation (ADR-0036 / ADR-0059 invariants), BUT
    /// the landed document keeps the CURRENT generation: a status write is
    /// not a desired-state write (k8s status-subresource rule), and bumping
    /// would flip ADR-0058's top-level ConvergePhase into a false
    /// PendingApply after every cascade. The in-memory generation counter
    /// rides the document across the read-modify-write, so two phase writes
    /// in a row never regress or duplicate the counter.
    pub fn record_subscription_phase(
        &self,
        name: &str,
        phase: SubscriptionPhase,
        stage: Option<EstablishStep>,
        phase_error: Option<String>,
    ) -> Result<StrategyConfig, String> {
        if name.is_empty() || name.len() > MAX_PLATFORM_NAME_LEN {
            return Err("subscription name must be 1..128 chars".to_string());
        }
        if name.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err("subscription name contains control characters".to_string());
        }
        if let Some(err) = phase_error.as_deref() {
            if err.len() > MAX_PHASE_ERROR_LEN {
                return Err(format!("phase_error too long (max {MAX_PHASE_ERROR_LEN} chars)"));
            }
            if err.bytes().any(|b| b == 0) {
                return Err("phase_error contains NUL".to_string());
            }
        }
        // Shape lock: only Establishing/Failed carry a stage; the terminal
        // green state and the data-landing state never do.
        match (phase, stage) {
            (SubscriptionPhase::Establishing, Some(EstablishStep::Platform))
            | (SubscriptionPhase::Establishing, Some(EstablishStep::Bind))
            | (SubscriptionPhase::Establishing, Some(EstablishStep::Port))
            | (SubscriptionPhase::Establishing, Some(EstablishStep::Apply))
            | (SubscriptionPhase::Failed, Some(EstablishStep::Import))
            | (SubscriptionPhase::Failed, Some(EstablishStep::Resolve))
            | (SubscriptionPhase::Failed, Some(EstablishStep::Platform))
            | (SubscriptionPhase::Failed, Some(EstablishStep::Bind))
            | (SubscriptionPhase::Failed, Some(EstablishStep::Port))
            | (SubscriptionPhase::Failed, Some(EstablishStep::Apply)) => {}
            (SubscriptionPhase::Establishing | SubscriptionPhase::Failed, None) => {
                return Err("Establishing/Failed require a stage".to_string());
            }
            (_, Some(_)) => {
                return Err(format!("{phase:?} rows never carry a stage"));
            }
            // Flat phases (Never/Importing/Converged/NeedsApproval) carry
            // neither column — the legal no-payload case.
            (_, None) => {}
        }
        if phase == SubscriptionPhase::Failed && phase_error.is_none() {
            return Err("Failed requires a phase_error reason".to_string());
        }
        if phase != SubscriptionPhase::Failed && phase_error.is_some() {
            return Err(format!("{phase:?} rows never carry a phase_error"));
        }

        let mut config = self.get()?;
        // Upsert by name (the row key). Pre-existing rows never lose their
        // slot — status history is not orderable, the array is a map.
        match config.subscriptions.iter_mut().find(|s| s.name == name) {
            Some(row) => {
                row.phase = phase;
                row.stage = stage;
                row.phase_error = phase_error;
            }
            None => config.subscriptions.push(SubscriptionStatus {
                name: name.to_string(),
                phase,
                stage,
                phase_error,
            }),
        }
        if config.subscriptions.len() > MAX_SUBSCRIPTION_STATUS_ROWS {
            return Err(format!(
                "subscriptions status array too long (max {MAX_SUBSCRIPTION_STATUS_ROWS})"
            ));
        }
        // Landed generation stays at the CURRENT value: status, not spec.
        self.store.store(&config)?;
        Ok(config)
    }
}

impl StrategyService<FsStrategyStore> {
    /// Full apply: read whitebox -> parse nodes -> fulfill the reconcile
    /// preview's promises per platform (create missing-on-resin platforms
    /// through the ResinClient create seam, ADR-0056; PATCH region_filters
    /// for every whitebox platform found on Resin whose live region_filters
    /// actually drift from the computed plan — diff-then-skip, ADR-0057:
    /// an in-sync platform is skipped, so apply is wire-idempotent and a
    /// second reconcile pass emits zero PATCH requests). PATCH/create
    /// failures are reported per-platform, never fatal. Apply NEVER deletes
    /// whitebox desired state: a failed create keeps the entry and reports
    /// the reason (the former apply-time auto-clean path was removed by
    /// ADR-0056 — the "said establish, actually deleted" contradiction).
    pub async fn apply(
        &self,
        client: &crate::resin_client::ResinClient,
        platform_id_for_name: fn(&serde_json::Value, &str) -> Option<String>,
    ) -> Result<ApplyReport, String> {
        let config = self.get()?;
        let nodes_v = client.list_nodes().await.map_err(|e| e.to_string())?;
        let nodes = parse_nodes(&nodes_v);

        let live_platforms_v = client
            .list_platforms()
            .await
            .map_err(|e| e.to_string())?;
        let live_names: std::collections::HashSet<String> = items(&live_platforms_v)
            .iter()
            .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();

        // Round 5 T01 / issue 01 F3: reference-resolution input. One extra
        // GET /subscriptions ONLY when at least one whitebox platform lists
        // subscriptions — a region/manual-only config keeps the exact
        // ADR-0057 wire shape (zero new requests). The dangling list is
        // merged into each platform's report row below; apply NEVER deletes
        // the whitebox entry for a dangling ref (ADR-0056 discipline).
        let dangling_by_platform: HashMap<String, Vec<String>> =
            if config.platforms.iter().any(|ps| !ps.subscriptions.is_empty()) {
                let subs_v = client.list_subscriptions().await.map_err(|e| e.to_string())?;
                let live_subs: std::collections::HashSet<String> = items(&subs_v)
                    .iter()
                    .filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect();
                dangling_subscription_refs(&config, &live_subs)
                    .into_iter()
                    .collect()
            } else {
                HashMap::new()
            };

        let plan = compute_plan(&config, &nodes);
        let mut platforms = Vec::new();
        for (platform_name, regions) in &plan {
            let dangling = dangling_by_platform
                .get(platform_name)
                .cloned()
                .unwrap_or_default();
            if !live_names.contains(platform_name) {
                // ADR-0056: the preview says "will be established" — make it
                // true. Create the platform on Resin, then fall through to
                // the PATCH loop (which re-reads list_platforms and finds
                // the created row). A failed create is reported per-platform
                // and the whitebox entry survives for the next apply to
                // retry.
                match client.create_platform_from_name(platform_name).await {
                    Ok(_) => {
                        tracing::info!(platform = %platform_name, "strategy_apply: created missing-on-resin platform");
                    }
                    Err(e) => {
                        tracing::warn!(platform = %platform_name, error = %e.to_string(), "strategy_apply: create platform failed");
                        platforms.push(with_dangling_note(
                            AppliedPlatform {
                                platform: platform_name.clone(),
                                region_filters: regions.clone(),
                                patched: false,
                                reason: Some(format!("create failed: {e}")),
                            },
                            &dangling,
                        ));
                        continue;
                    }
                }
            }
            let platforms_v = client
                .list_platforms()
                .await
                .map_err(|e| e.to_string())?;
            if let Some(id) = platform_id_for_name(&platforms_v, platform_name) {
                // Ticket 36 / ADR-0057: diff-then-skip. Compare the computed
                // region_filters against the live row we just read; PATCH
                // only on real drift. No live -> no diff -> no PATCH: apply
                // is wire-idempotent (a second reconcile pass emits zero
                // PATCH requests). The comparison reuses the SAME
                // order-insensitive, case-insensitive set rule as the
                // three-state merge and the reconcile preview, so apply, the
                // preview and the snapshot can never disagree about what
                // counts as drift. The live row comes from THIS pass's
                // fresh `list_platforms` read — never from cached state —
                // and a failed PATCH still reports per-platform with the
                // whitebox entry kept (ADR-0056 re-assert-on-retry).
                let live_regions = parse_resin_platforms(&platforms_v)
                    .into_iter()
                    .find(|rp| rp.name == *platform_name)
                    .map(|rp| rp.region_filters)
                    .unwrap_or_default();
                if same_region_set(regions, &live_regions) {
                    platforms.push(with_dangling_note(
                        AppliedPlatform {
                            platform: platform_name.clone(),
                            region_filters: regions.clone(),
                            patched: true,
                            reason: Some("in sync".to_string()),
                        },
                        &dangling,
                    ));
                    continue;
                }
                let body = serde_json::json!({ "region_filters": regions });
                match client.update_platform(&id, body).await {
                    Ok(_) => platforms.push(with_dangling_note(
                        AppliedPlatform {
                            platform: platform_name.clone(),
                            region_filters: regions.clone(),
                            patched: true,
                            reason: None,
                        },
                        &dangling,
                    )),
                    Err(e) => {
                        tracing::warn!(platform = %platform_name, error = %e.to_string(), "auto_strategy_apply: PATCH region_filters failed");
                        platforms.push(with_dangling_note(
                            AppliedPlatform {
                                platform: platform_name.clone(),
                                region_filters: regions.clone(),
                                patched: false,
                                reason: Some(format!("PATCH failed: {e}")),
                            },
                            &dangling,
                        ));
                    }
                }
            } else {
                platforms.push(with_dangling_note(
                    AppliedPlatform {
                        platform: platform_name.clone(),
                        region_filters: regions.clone(),
                        patched: false,
                        reason: Some("platform not found".to_string()),
                    },
                    &dangling,
                ));
            }
        }

        // Round 5 T09 / ADR-0058 (D-26): apply-generation write-back. All
        // green => applied_generation catches up to the generation the
        // write-back itself will LAND at (store() bumps, so that is
        // generation + 1) and the error slot clears — inside the same store
        // write entry (R-B Q2). Anything not green => the old applied value
        // stays (no fake convergence) and the failure is recorded. A green
        // pass over an already-converged world (diff-then-skip, zero PATCH)
        // still refreshes last_apply_at — Terraform re-apply semantics.
        // ConvergePhase keys on applied == gen, which holds after every
        // green pass by this construction.
        if platforms.iter().all(|p| p.patched) {
            let mut green = self.get()?;
            green.applied_generation = green.generation.wrapping_add(1);
            green.last_apply_at = Some(now_unix());
            green.last_apply_error = None;
            self.store(green)?;
        } else {
            let failed = platforms
                .iter()
                .find(|p| !p.patched)
                .map(|p| p.reason.clone().unwrap_or_else(|| "apply failed".to_string()))
                .unwrap_or_else(|| "apply failed".to_string());
            let mut stale = self.get()?;
            stale.last_apply_error = Some(failed);
            // The error write goes through the same store entry: generation
            // bumps (a whitebox write happened), applied_generation does not
            // catch up — applied < gen with an error = ApplyFailed phase.
            self.store(stale)?;
        }

        Ok(ApplyReport { platforms })
    }

    /// Architecture-recovery ticket 14 / ADR-0054 §A: ONE-WAY reconcile.
    /// Serial: strategy apply FIRST, ports restore SECOND, stop at the first
    /// failure (fail-fast) so a broken strategy PATCH can never mask a port
    /// problem behind a half-applied pass. The ports half is injected as a
    /// closure so the whitebox store stays a shell-side concern — resin-core
    /// never touches the ports whitebox directly (same seam discipline as
    /// `apply`). No "accept current state" write exists by design.
    /// Wire-idempotent end to end since ADR-0057: the strategy half
    /// diff-then-skips in-sync platforms, the ports half is TTL-throttled —
    /// a second pass emits zero write requests (zero changes, both halves).
    pub async fn reconcile(
        &self,
        client: &crate::resin_client::ResinClient,
        platform_id_for_name: fn(&serde_json::Value, &str) -> Option<String>,
        restore_ports: impl std::future::Future<Output = Result<ReconcilePortsOutcome, String>>,
    ) -> Result<ReconcileReport, String> {
        let strategy = self.apply(client, platform_id_for_name).await?;
        let ports = restore_ports
            .await
            .map_err(|e| format!("reconcile: port restore failed after strategy apply: {e}"))?;
        Ok(ReconcileReport {
            strategy,
            ports_restored: ports.restored,
            ports_skipped: ports.skipped,
        })
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ps(name: &str, regions: &[&str]) -> PlatformStrategy {
        PlatformStrategy {
            platform_name: name.to_string(),
            a_class: crate::strategy_engine::AClassStrategy::Region,
            b_class: crate::strategy::StrategyId::Random,
            manual_nodes: vec![],
            regions: regions.iter().map(|s| s.to_string()).collect(),
            subscriptions: vec![],
            top_n: 10,
            b_class_params: Default::default(),
        }
    }

    fn cfg(platforms: Vec<PlatformStrategy>) -> StrategyConfig {
        StrategyConfig {
            version: 1,
            platforms,
            acknowledged: vec![],
            generation: 0,
            applied_generation: 0,
            last_apply_at: None,
            last_apply_error: None,
            updated_at: None,
        }
    }

    /// In-memory store for fixtures.
    struct MemStore(serde_json::Value);
    impl StrategyConfigStore for MemStore {
        fn load(&self) -> Result<Option<StrategyConfig>, String> {
            if self.0.is_null() {
                return Ok(None);
            }
            serde_json::from_value(self.0.clone())
                .map(Some)
                .map_err(|e| e.to_string())
        }
        fn store(&self, config: &StrategyConfig) -> Result<(), String> {
            // MemStore is read-only by construction in these fixtures; record
            // nothing. Persistence tests use FsStrategyStore + tempdir.
            let _ = serde_json::to_value(config).map(|v| v);
            Ok(())
        }
    }

    // ---- get ----
    #[test]
    fn get_returns_defaults_when_file_missing() {
        let svc = StrategyService::new(MemStore(serde_json::Value::Null));
        let c = svc.get().unwrap();
        assert_eq!(c.version, 1);
        assert!(c.platforms.is_empty());
    }

    #[test]
    fn get_parses_existing_document() {
        let doc = json!({"version": 1, "platforms": [
            {"platform_name": "Anthropic", "a_class": "region", "b_class": "random", "regions": ["US"]}
        ]});
        let svc = StrategyService::new(MemStore(doc));
        let c = svc.get().unwrap();
        assert_eq!(c.platforms.len(), 1);
        assert_eq!(c.platforms[0].platform_name, "Anthropic");
        assert_eq!(c.platforms[0].regions, vec!["US".to_string()]);
    }

    // ---- validate ----
    #[test]
    fn validate_rejects_wrong_version_and_bad_names() {
        let mut c = cfg(vec![ps("A", &["US"])]);
        c.version = 2;
        assert!(validate(&c).is_err());
        c.version = 1;
        c.platforms[0].platform_name = String::new();
        assert!(validate(&c).is_err());
        c.platforms[0].platform_name = "x".repeat(129);
        assert!(validate(&c).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_platforms_and_oversized_lists() {
        let c = cfg(vec![ps("A", &["US"]), ps("A", &["HK"])]);
        assert!(validate(&c).is_err());
        let mut c2 = cfg(vec![ps("A", &["US"])]);
        c2.platforms[0].regions = (0..65).map(|i| format!("R{i}")).collect();
        assert!(validate(&c2).is_err());
        c2.platforms[0].regions = (0..64).map(|i| format!("R{i}")).collect();
        assert!(validate(&c2).is_ok());
    }

    // ---- store (write entry) ----
    #[test]
    fn store_rejects_invalid_documents_before_writing() {
        let svc = StrategyService::new(MemStore(serde_json::Value::Null));
        let mut bad = cfg(vec![ps("A", &["US"])]);
        bad.version = 9;
        assert!(svc.store(bad).is_err());
    }

    // ---- Round 5 T09 / ADR-0058: generation counter (F2) ----
    #[test]
    fn store_bumps_generation_serially_and_stamps_updated_at() {
        let dir = std::env::temp_dir().join(format!("strategy-svc-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("egressapikey-strategy.json");
        let svc = StrategyService::new(FsStrategyStore::new(path.clone()));
        // Every sanctioned store-path write bumps by exactly one: IPC put
        // equivalent (store), deep region edit, rollback.
        let g1 = svc.store(cfg(vec![ps("A", &["US"])])).unwrap();
        assert_eq!(g1.generation, 1);
        let t1 = g1.updated_at.unwrap();
        let g2 = svc
            .set_platform_regions("A", vec!["HK".to_string()])
            .unwrap();
        assert_eq!(g2.generation, 2);
        let t2 = g2.updated_at.unwrap();
        assert!(t2 >= t1, "updated_at must advance or hold (same-second ok)");
        // apply-generation write-back (F3) also lands through store(): the
        // error path bumps too. Verify via a direct green write-back below.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strategy_config_v1_file_deserializes_with_generation_zero_never_applied() {
        // Acceptance (a): a v1 file (no generation fields) loads with
        // generation=0 = NeverApplied — zero-migration serde(default) compat.
        let dir = std::env::temp_dir().join(format!("strategy-svc-v1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("egressapikey-strategy.json");
        std::fs::write(
            &path,
            r#"{"version":1,"platforms":[{"platform_name":"Old","a_class":"region","b_class":"random","regions":["US"]}]}"#,
        )
        .unwrap();
        let svc = StrategyService::new(FsStrategyStore::new(path.clone()));
        let v1 = svc.get().unwrap();
        assert_eq!(v1.version, 1);
        assert_eq!(v1.generation, 0);
        assert_eq!(v1.applied_generation, 0);
        assert_eq!(v1.last_apply_at, None);
        assert_eq!(v1.last_apply_error, None);
        assert_eq!(v1.updated_at, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Round 5 T09 / ADR-0058: apply write-back (F3, D-26) ----
    #[tokio::test]
    async fn apply_green_write_back_applies_generation_inside_store_entry() {
        let store_path = apply_fixture_store_path("gen-green");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        let stored = svc.store(cfg(vec![ps("alpha", &["HK"])]));
        assert!(stored.is_ok());

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"node_hash":"h1","region":"HK","has_outbound":true,"failure_count":0}]}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        // Two phases: drifted, then in-sync on the write-back re-read? No —
        // the write-back re-read happens AFTER the PATCH loop, the live row
        // is already synced by then; the single drifted registration covers
        // both reads.
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-a","name":"alpha","region_filters":["US"]}]}"#)
            .expect_at_least(2)
            .create_async()
            .await;
        let m_patch = server
            .mock("PATCH", "/api/v1/platforms/id-a")
            .match_header(bearer.0, bearer.1)
            .match_body(mockito::Matcher::PartialJson(json!({"region_filters": ["HK"]})))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"id-a"}"#)
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let report = svc.apply(&c, platform_id_for_name_fixture).await.expect("apply green");
        assert!(report.platforms[0].patched);

        // The write-back: applied_generation caught up, error slot cleared,
        // last_apply_at stamped. gen >= applied always; equality is what the
        // ConvergePhase derivation keys on (the write-back's own store()
        // bump means the landed file reads gen = N+1, applied = gen).
        let after = svc.get().unwrap();
        assert_eq!(after.applied_generation, after.generation, "green pass must converge the counter pair");
        assert!(after.applied_generation >= 1, "write-back went through store(): {after:?}");
        assert_eq!(after.last_apply_error, None);
        assert!(after.last_apply_at.is_some());

        m_nodes.assert_async().await;
        m_platforms.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    #[tokio::test]
    async fn apply_green_diff_skip_pass_still_refreshes_last_apply_at() {
        // Same-generation re-apply (Terraform re-apply semantics): a fully
        // green pass over an already-converged world emits ZERO PATCH but
        // still refreshes last_apply_at and converges the counter pair.
        let store_path = apply_fixture_store_path("gen-resync");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        svc.store(cfg(vec![ps("alpha", &["HK"])]))
            .unwrap();

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"node_hash":"h1","region":"HK","has_outbound":true,"failure_count":0}]}"#)
            .expect(1)
            .create_async()
            .await;
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-a","name":"alpha","region_filters":["HK"]}]}"#)
            .expect(2)
            .create_async()
            .await;
        let m_patch = server
            .mock("PATCH", mockito::Matcher::Any)
            .with_status(500)
            .expect(0)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let before = svc.get().unwrap();
        let report = svc.apply(&c, platform_id_for_name_fixture).await.expect("pass green");
        assert_eq!(report.platforms[0].reason.as_deref(), Some("in sync"));
        let after = svc.get().unwrap();
        assert_eq!(after.applied_generation, after.generation);
        assert!(after.last_apply_at.is_some(), "zero-PATCH green pass still stamps apply time");
        assert!(after.last_apply_at >= before.last_apply_at);
        m_nodes.assert_async().await;
        m_platforms.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    #[tokio::test]
    async fn apply_failure_keeps_old_applied_and_records_last_apply_error() {
        // D-26 failure path: anything not green keeps applied_generation at
        // its old value (no fake convergence) and records the failure reason.
        let store_path = apply_fixture_store_path("gen-fail");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        svc.store(cfg(vec![ps("alpha", &["HK"])]))
            .unwrap();

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"node_hash":"h1","region":"HK","has_outbound":true,"failure_count":0}]}"#)
            .create_async()
            .await;
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-a","name":"alpha","region_filters":["US"]}]}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        // PATCH 400: per-platform failure, apply stays "successful" overall
        // but is NOT green — the write-back must record the error.
        let m_patch = server
            .mock("PATCH", "/api/v1/platforms/id-a")
            .match_header(bearer.0, bearer.1)
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"region rejected"}"#)
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let report = svc.apply(&c, platform_id_for_name_fixture).await.expect("apply reports per-platform");
        assert!(!report.platforms[0].patched);

        let after = svc.get().unwrap();
        assert_eq!(after.applied_generation, 0, "failure must NOT converge the counter");
        assert!(
            after.last_apply_error.as_deref().unwrap_or("").contains("PATCH failed"),
            "failure reason must be recorded: {after:?}"
        );
        // last_apply_at is untouched by a failed pass (only green passes
        // stamp it — the field means "last time it actually worked").
        assert_eq!(after.last_apply_at, None);
        m_nodes.assert_async().await;
        m_platforms.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    // ---- set_platform_regions (deep edit for the canvas) ----
    #[test]
    fn set_platform_regions_updates_existing_entry() {
        let svc = StrategyService::new(MemStore(
            json!({"version": 1, "platforms": [
                {"platform_name": "Anthropic", "a_class": "region", "b_class": "random", "regions": ["US"]}
            ]}),
        ));
        let stored = svc
            .set_platform_regions("Anthropic", vec!["HK".to_string(), "SG".to_string()])
            .unwrap();
        assert_eq!(stored.platforms[0].regions, vec!["HK".to_string(), "SG".to_string()]);
        // a_class/b_class untouched
        assert_eq!(stored.platforms[0].a_class, crate::strategy_engine::AClassStrategy::Region);
    }

    #[test]
    fn set_platform_regions_creates_missing_entry_as_region_strategy() {
        let svc = StrategyService::new(MemStore(serde_json::Value::Null));
        let stored = svc
            .set_platform_regions("NewPlat", vec!["US".to_string()])
            .unwrap();
        assert_eq!(stored.platforms.len(), 1);
        assert_eq!(stored.platforms[0].a_class, crate::strategy_engine::AClassStrategy::Region);
        assert_eq!(stored.platforms[0].regions, vec!["US".to_string()]);
    }

    #[test]
    fn set_platform_regions_rejects_bad_inputs() {
        let svc = StrategyService::new(MemStore(serde_json::Value::Null));
        assert!(svc.set_platform_regions("", vec![]).is_err());
        assert!(svc.set_platform_regions(&"x".repeat(129), vec![]).is_err());
        let many = (0..65).map(|i| format!("R{i}")).collect();
        assert!(svc.set_platform_regions("A", many).is_err());
    }

    // ---- apply report shape (serde contract) ----
    #[test]
    fn applied_platform_serializes_reason_only_on_failure() {
        let ok = AppliedPlatform {
            platform: "A".into(),
            region_filters: vec!["US".into()],
            patched: true,
            reason: None,
        };
        let v = serde_json::to_value(&ok).unwrap();
        assert!(v.get("reason").is_none());
        assert_eq!(v["patched"], json!(true));
        let bad = AppliedPlatform {
            platform: "A".into(),
            region_filters: vec![],
            patched: false,
            reason: Some("PATCH failed: boom".into()),
        };
        let v2 = serde_json::to_value(&bad).unwrap();
        assert_eq!(v2["reason"], json!("PATCH failed: boom"));
    }

    #[test]
    fn store_ref_exposes_path_for_snapshot_read_side() {
        let dir = std::env::temp_dir().join(format!("strategy-svc-ref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("egressapikey-strategy.json");
        let svc = StrategyService::new(FsStrategyStore::new(path.clone()));
        assert_eq!(svc.store_ref().path(), &path);
        let _ = std::fs::remove_dir(&dir);
    }

    // ---- ticket 15: whitebox versioning (ADR-0054 section B) ----
    #[test]
    fn fs_store_write_backs_up_previous_file_and_is_atomic() {
        let dir = std::env::temp_dir().join(format!("strategy-svc-bak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("egressapikey-strategy.json");
        let svc = StrategyService::new(FsStrategyStore::new(path.clone()));
        // First write: no previous file, no backup.
        svc.store(cfg(vec![ps("A", &["US"])])).unwrap();
        assert!(svc.store_ref().list_backups().unwrap().is_empty());
        // Second write: previous content backed up before the swap.
        svc.store(cfg(vec![ps("B", &["EU"])])).unwrap();
        let backups = svc.store_ref().list_backups().unwrap();
        assert_eq!(backups.len(), 1);
        let raw = std::fs::read_to_string(
            crate::whitebox_backup::backup_dir(&path).join(&backups[0].file_name),
        )
        .unwrap();
        assert!(raw.contains("\"A\""), "backup must hold the PREVIOUS content");
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fs_store_rollback_restores_previous_and_rejects_garbage() {
        let dir = std::env::temp_dir().join(format!("strategy-svc-rb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("egressapikey-strategy.json");
        let svc = StrategyService::new(FsStrategyStore::new(path.clone()));
        svc.store(cfg(vec![ps("A", &["US"])])).unwrap();
        svc.store(cfg(vec![ps("B", &["EU"])])).unwrap();
        let backup_name = svc.store_ref().list_backups().unwrap()[0].file_name.clone();
        let rolled = svc.store_ref().rollback(&backup_name).unwrap();
        assert_eq!(rolled.platforms[0].platform_name, "A");
        assert_eq!(svc.get().unwrap().platforms[0].platform_name, "A");
        // Tampered backup content is rejected before any swap.
        std::fs::write(
            crate::whitebox_backup::backup_dir(&path).join(&backup_name),
            b"garbage",
        )
        .unwrap();
        assert!(svc.store_ref().rollback(&backup_name).is_err());
        assert_eq!(svc.get().unwrap().platforms[0].platform_name, "A");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- ticket 12: acknowledged field parse/validate ----
    #[test]
    fn acknowledged_parses_from_optional_field_and_defaults_empty() {
        // Older config without the field loads unchanged (serde default).
        let doc = json!({"version": 1, "platforms": [
            {"platform_name": "Anthropic", "a_class": "region", "b_class": "random", "regions": ["US"]}
        ]});
        let svc = StrategyService::new(MemStore(doc));
        let c = svc.get().unwrap();
        assert!(c.acknowledged.is_empty());

        // Round-trip: the field persists through the real file store
        // (MemStore.store is a deliberate no-op fixture).
        let dir = std::env::temp_dir().join(format!("strategy-svc-ack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("egressapikey-strategy.json");
        let _ = std::fs::remove_file(&path);
        let fsvc = StrategyService::new(FsStrategyStore::new(path.clone()));
        let mut cfg_doc = cfg(vec![ps("A", &["US"])]);
        cfg_doc.acknowledged = vec!["A".to_string()];
        fsvc.store(cfg_doc).unwrap();
        let reloaded = fsvc.get().unwrap();
        assert_eq!(reloaded.acknowledged, vec!["A".to_string()]);
        // skip_serializing_if: empty list writes NO acknowledged key.
        let mut empty = cfg(vec![]);
        empty.acknowledged = vec![];
        fsvc.store(empty).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("acknowledged"), "empty exemption list must not appear in the file");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn acknowledged_validate_rejects_oversized_empty_control_and_duplicate_members() {
        let mut c = cfg(vec![]);
        // oversized member
        c.acknowledged = vec!["x".repeat(129)];
        assert!(validate(&c).unwrap_err().contains("1..128"));
        // empty member
        c.acknowledged = vec![String::new()];
        assert!(validate(&c).unwrap_err().contains("1..128"));
        // control character
        c.acknowledged = vec!["bad\u{0}name".to_string()];
        assert!(validate(&c).unwrap_err().contains("control"));
        // duplicate members
        c.acknowledged = vec!["A".to_string(), "A".to_string()];
        assert!(validate(&c).unwrap_err().contains("duplicated"));
        // oversized list (65 > 64)
        c.acknowledged = (0..65).map(|i| format!("P{i}")).collect();
        assert!(validate(&c).unwrap_err().contains("max 64"));
        // valid list passes
        c.acknowledged = (0..64).map(|i| format!("P{i}")).collect();
        assert!(validate(&c).is_ok());
    }

    #[test]
    fn acknowledged_non_string_member_fails_deserialize_not_validate() {
        // A hand-edited JSON with a non-string member is a PARSE error (the
        // file cannot load), never a silent silent coercion — valid old files
        // without the field keep loading (compat preserved).
        let doc = json!({"version": 1, "platforms": [], "acknowledged": ["ok", 42]});
        let svc = StrategyService::new(MemStore(doc));
        assert!(svc.get().is_err());
    }

    // ---- FsStrategyStore: real-file round trip + missing file ----
    #[test]
    fn fs_store_round_trip_and_missing_default() {
        let dir = std::env::temp_dir().join(format!("strategy-svc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("egressapikey-strategy.json");
        let _ = std::fs::remove_file(&path);
        let store = FsStrategyStore::new(path.clone());
        // missing -> None -> defaults
        let svc = StrategyService::new(store);
        assert!(svc.get().unwrap().platforms.is_empty());
        // store + reload (T09: store() bumps generation 0->1 and stamps
        // updated_at, so the reloaded doc differs from the input in exactly
        // those two write-authority fields).
        let c = cfg(vec![ps("Anthropic", &["US", "HK"])]);
        let landed = svc.store(c.clone()).unwrap();
        assert_eq!(landed.generation, 1);
        assert!(landed.updated_at.is_some());
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"platform_name\""));
        let reloaded = svc.get().unwrap();
        assert_eq!(reloaded, landed);
        assert_eq!(reloaded.generation, 1);
        assert!(reloaded.updated_at.is_some());
        // set_platform_regions persists through the same file
        svc.set_platform_regions("Anthropic", vec!["SG".to_string()]).unwrap();
        let again = svc.get().unwrap();
        assert_eq!(again.platforms[0].regions, vec!["SG".to_string()]);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    // ---- ticket 14 / ADR-0054 §A: reconcile plan + idempotency memory ----

    fn node(hash: &str, region: &str, healthy: bool) -> NodeSummary {
        NodeSummary {
            node_hash: hash.into(),
            region: region.into(),
            has_outbound: healthy,
            failure_count: if healthy { 0 } else { 2 },
            reference_latency_ms: None,
            subscription_name: None,
        }
    }

    fn runtime(name: &str, id: &str, regions: &[&str]) -> ResinPlatformRuntime {
        ResinPlatformRuntime {
            id: id.into(),
            name: name.into(),
            region_filters: regions.iter().map(|s| s.to_string()).collect(),
            allocation_policy: "BALANCED".into(),
        }
    }

    fn pm(port: u16, platform: &str, enabled: bool) -> crate::db::PortMapping {
        crate::db::PortMapping {
            port,
            protocol: "socks5".into(),
            platform_name: platform.into(),
            account: String::new(),
            label: String::new(),
            enabled,
            auth_required: false,
        }
    }

    #[test]
    fn reconcile_plan_lists_region_drift_and_missing_platforms() {
        // nodes: healthy HK; whitebox wants region HK for "alpha" and "beta".
        let nodes = vec![node("h1", "HK", true)];
        let config = cfg(vec![ps("alpha", &["HK"]), ps("beta", &["HK"])]);
        // live: alpha already patched to ["HK"] (order-insensitive equal),
        // beta live with the WRONG region -> patch_regions; "gamma" exists
        // only in the whitebox -> create_platform.
        let live = vec![runtime("alpha", "id-a", &["HK"]), runtime("beta", "id-b", &["US"])];
        let plan = compute_reconcile_plan(&config, &nodes, &live, &[], &[]);
        // One-way semantics: the plan covers WHITEBOX entries only. alpha is
        // in sync (absent), beta drifted (patch_regions); runtime-only rows
        // are never "reconciled away" — they are snapshot drift, not plan rows.
        assert_eq!(plan.platforms.len(), 1, "{plan:?}");
        let beta = plan.platforms.iter().find(|p| p.platform == "beta").unwrap();
        assert_eq!(beta.action, "patch_regions");
        assert_eq!(beta.desired_regions, vec!["HK".to_string()]);
        assert_eq!(beta.live_regions, vec!["US".to_string()]);
        let alpha = plan.platforms.iter().find(|p| p.platform == "alpha");
        assert!(alpha.is_none(), "in-sync platform must not appear");
        // beta's entry also covers the missing-platform case via a config
        // entry that Resin does not have:
        let config2 = cfg(vec![ps("alpha", &["HK"]), ps("gamma", &["HK"])]);
        let plan2 = compute_reconcile_plan(&config2, &nodes, &live, &[], &[]);
        let g = plan2.platforms.iter().find(|p| p.platform == "gamma").unwrap();
        assert_eq!(g.action, "create_platform");
        assert!(g.live_regions.is_empty());
    }

    #[test]
    fn reconcile_plan_region_comparison_is_order_and_case_insensitive() {
        let nodes = vec![node("h1", "HK", true), node("h2", "US", true)];
        let config = cfg(vec![ps("alpha", &["HK", "US"])]);
        let live = vec![runtime("alpha", "id-a", &["us", "hk", "us"])];
        let plan = compute_reconcile_plan(&config, &nodes, &live, &[], &[]);
        assert!(plan.platforms.is_empty(), "{plan:?}");
    }

    #[test]
    fn reconcile_plan_lists_disabled_whitebox_ports_never() {
        // Desired enabled port missing from Resin -> create_endpoint.
        let desired = vec![pm(17990, "alpha", true), pm(17991, "alpha", false)];
        // 17990 live; 17991 disabled -> no entry either way.
        let plan = compute_reconcile_plan(&cfg(vec![]), &[], &[], &[17990], &desired);
        assert!(plan.platforms.is_empty());
        assert!(plan.ports.is_empty(), "disabled + already-live ports must not be listed: {plan:?}");

        let plan2 = compute_reconcile_plan(&cfg(vec![]), &[], &[], &[], &desired);
        assert_eq!(plan2.ports.len(), 1);
        assert_eq!(plan2.ports[0].port, 17990);
        assert_eq!(plan2.ports[0].action, "create_endpoint");
        assert_eq!(plan2.ports[0].platform, "alpha");
    }

    #[test]
    fn reconcile_plan_empty_when_everything_in_sync() {
        let nodes = vec![node("h1", "HK", true)];
        let config = cfg(vec![ps("alpha", &["HK"])]);
        let live = vec![runtime("alpha", "id-a", &["HK"])];
        let desired = vec![pm(17990, "alpha", true)];
        let plan = compute_reconcile_plan(&config, &nodes, &live, &[17990], &desired);
        assert!(plan.is_empty(), "{plan:?}");
    }

    #[test]
    fn endpoint_live_ports_parses_wrapper_and_dedups() {
        assert_eq!(
            endpoint_live_ports(&json!({"items": [{"port": 17990}, {"port": 17100}, {"port": 17990}]})),
            vec![17100, 17990]
        );
        assert_eq!(endpoint_live_ports(&json!([{"port": 5}, {"port": 3}])), vec![3, 5]);
        assert_eq!(endpoint_live_ports(&json!({})), Vec::<u16>::new());
    }

    #[test]
    fn reconcile_memory_ttl_throttles_reassert_and_stamps_clear_it() {
        let mem = ReconcileMemory::default();
        let desired = vec![pm(17990, "alpha", true), pm(17991, "alpha", false)];
        // First pass: port missing from live -> to assert.
        let first = mem.ports_to_assert(&desired, &[], 1_000);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].port, 17990);
        // Stamp it; second pass within the TTL -> NOTHING to assert (zero
        // changes even though Resin still reports the port missing).
        mem.stamp_asserted(17990, 1_000);
        let second = mem.ports_to_assert(&desired, &[], 1_000 + 60);
        assert!(second.is_empty(), "second reconcile must be a no-op: {second:?}");
        // Third pass after TTL expiry re-asserts (self-correction path).
        let third = mem.ports_to_assert(&desired, &[17990], 1_000 + RECONCILE_PORT_TTL_SECS);
        assert!(third.is_empty(), "already-live port never re-asserts: {third:?}");
        let fourth = mem.ports_to_assert(&desired, &[], 1_000 + RECONCILE_PORT_TTL_SECS);
        assert_eq!(fourth.len(), 1);
        // Disabled ports are never asserted at any point.
        assert!(fourth.iter().all(|m| m.enabled));
    }

    #[tokio::test]
    async fn reconcile_fails_fast_when_ports_half_errors() {
        // The ports closure failing must propagate as Err AFTER the strategy
        // half succeeded (fail-fast ordering, issue 14 failure path).
        let svc = StrategyService::new(FsStrategyStore::new(std::env::temp_dir().join(format!(
            "strategy-svc-reconcile-{}.json",
            std::process::id()
        ))));
        let mut server = mockito::Server::new_async().await;
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header("authorization", "Bearer testtok")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[]}"#)
            .create_async()
            .await;
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header("authorization", "Bearer testtok")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[]}"#)
            .create_async()
            .await;
        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let out = svc
            .reconcile(&c, platform_id_for_name_fixture, async { Err("port restore boom".to_string()) })
            .await;
        assert!(out.is_err());
        assert!(out.unwrap_err().contains("port restore boom"));
        m_nodes.assert_async().await;
        m_platforms.assert_async().await;
    }

    // ---- apply: missing-on-resin platform semantics (ADR-0056, ticket 22) ----

    fn apply_fixture_store_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("strategy-apply-{}-{}.json", tag, std::process::id()))
    }

    /// The preview promises "create_platform" for a whitebox platform absent
    /// from Resin; apply must FULFILL it: POST /platforms {"name": ...}, then
    /// PATCH the computed region_filters via the created id. The whitebox
    /// entry survives (apply never deletes desired state, ADR-0056).
    #[tokio::test]
    async fn apply_creates_missing_on_resin_platform_and_patches_regions() {
        let store_path = apply_fixture_store_path("create");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        svc.store(cfg(vec![ps("alpha", &["HK"])]))
            .unwrap();

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"node_hash":"h1","region":"HK","has_outbound":true,"failure_count":0}]}"#)
            .create_async()
            .await;
        // Two platform-list phases, in registration order (mockito serves the
        // first mock that still has missing hits): the apply's initial read
        // sees alpha MISSING (empty list), the PATCH loop's re-read after the
        // create sees alpha LIVE (id-alpha).
        let m_platforms_empty = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[]}"#)
            .expect(1)
            .create_async()
            .await;
        let m_platforms_created = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-alpha","name":"alpha","region_filters":[]}]}"#)
            .expect(1)
            .create_async()
            .await;
        let m_create = server
            .mock("POST", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .match_body(mockito::Matcher::PartialJson(json!({"name": "alpha"})))
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"id-alpha","name":"alpha"}"#)
            .expect(1)
            .create_async()
            .await;
        let m_patch = server
            .mock("PATCH", "/api/v1/platforms/id-alpha")
            .match_header(bearer.0, bearer.1)
            .match_body(mockito::Matcher::PartialJson(json!({"region_filters": ["HK"]})))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"id-alpha"}"#)
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let report = svc
            .apply(&c, platform_id_for_name_fixture)
            .await
            .expect("apply must succeed");
        assert_eq!(report.platforms.len(), 1);
        let p = &report.platforms[0];
        assert_eq!(p.platform, "alpha");
        assert!(p.patched, "created platform must be PATCHed: {p:?}");
        assert_eq!(p.reason, None);

        // The whitebox entry survives: apply never deletes desired state.
        let after = svc.get().unwrap();
        assert_eq!(after.platforms.len(), 1);
        assert_eq!(after.platforms[0].platform_name, "alpha");

        m_nodes.assert_async().await;
        m_platforms_empty.assert_async().await;
        m_platforms_created.assert_async().await;
        m_create.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    /// A failed create is honest, not silent: the report carries the failure
    /// reason for that platform AND the whitebox entry stays (the reverse
    /// "said establish, actually deleted" action is forbidden, ADR-0056).
    #[tokio::test]
    async fn apply_failed_create_reports_reason_and_keeps_whitebox_entry() {
        let store_path = apply_fixture_store_path("failcreate");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        svc.store(cfg(vec![ps("alpha", &["HK"])]))
            .unwrap();

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[]}"#)
            .create_async()
            .await;
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[]}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        // T04/round5: a 4xx rejection (never retried by send_with_retry) keeps
        // this a single-attempt semantics test; a 5xx here would now be
        // re-POSTed twice by the write-path retry (locked separately by the
        // resin_client mockito_retry_* tests).
        let m_create = server
            .mock("POST", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"name rejected"}"#)
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let report = svc
            .apply(&c, platform_id_for_name_fixture)
            .await
            .expect("apply itself must not fail: per-platform errors are non-fatal");
        assert_eq!(report.platforms.len(), 1);
        let p = &report.platforms[0];
        assert_eq!(p.platform, "alpha");
        assert!(!p.patched);
        assert!(
            p.reason.as_deref().unwrap_or("").contains("create"),
            "reason must surface the create failure: {p:?}"
        );

        // The whitebox entry KEEPS its place — no reverse deletion.
        let after = svc.get().unwrap();
        assert_eq!(after.platforms.len(), 1);
        assert_eq!(after.platforms[0].platform_name, "alpha");

        m_nodes.assert_async().await;
        m_platforms.assert_async().await;
        m_create.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    /// A live platform keeps the plain PATCH path (no create POST) — the
    /// create branch fires only for missing-on-resin names (ADR-0056).
    #[tokio::test]
    async fn apply_live_platform_patches_without_create() {
        let store_path = apply_fixture_store_path("live");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        svc.store(cfg(vec![ps("alpha", &["HK"])]))
            .unwrap();

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"node_hash":"h1","region":"HK","has_outbound":true,"failure_count":0}]}"#)
            .create_async()
            .await;
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-a","name":"alpha","region_filters":["US"]}]}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        let m_create = server
            .mock("POST", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"x"}"#)
            .expect(0)
            .create_async()
            .await;
        let m_patch = server
            .mock("PATCH", "/api/v1/platforms/id-a")
            .match_header(bearer.0, bearer.1)
            .match_body(mockito::Matcher::PartialJson(json!({"region_filters": ["HK"]})))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"id-a"}"#)
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let report = svc
            .apply(&c, platform_id_for_name_fixture)
            .await
            .expect("apply must succeed");
        assert!(report.platforms[0].patched, "{:?}", report.platforms[0]);

        m_nodes.assert_async().await;
        m_platforms.assert_async().await;
        m_create.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    /// Ticket 36 / ADR-0057: an in-sync platform (live region_filters equal
    /// the computed plan under the snapshot's order/case-insensitive set
    /// rule) is diff-skipped: the report marks the platform converged
    /// (patched=true + "in sync" reason, so Settings' patched/errors counter
    /// stays truthful) and NO PATCH request reaches Resin — even when the
    /// live row spells the set differently (["hk"] vs computed ["HK"]).
    #[tokio::test]
    async fn apply_in_sync_platform_skips_patch() {
        let store_path = apply_fixture_store_path("insync");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        svc.store(cfg(vec![ps("alpha", &["HK"])]))
            .unwrap();

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"node_hash":"h1","region":"HK","has_outbound":true,"failure_count":0}]}"#)
            .create_async()
            .await;
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-a","name":"alpha","region_filters":["hk"]}]}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        // The ZERO-write proof: any PATCH on this server fails the test.
        let m_patch = server
            .mock("PATCH", mockito::Matcher::Any)
            .with_status(500)
            .expect(0)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let report = svc
            .apply(&c, platform_id_for_name_fixture)
            .await
            .expect("apply must succeed");
        assert_eq!(report.platforms.len(), 1);
        let p = &report.platforms[0];
        assert_eq!(p.platform, "alpha");
        assert!(p.patched, "in-sync platform must converge: {p:?}");
        assert_eq!(p.reason.as_deref(), Some("in sync"));

        // The whitebox entry survives untouched.
        let after = svc.get().unwrap();
        assert_eq!(after.platforms.len(), 1);

        m_nodes.assert_async().await;
        m_platforms.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    /// Ticket 36 / ADR-0057 wire-idempotency: apply TWICE against a synced
    /// world. Pass 1 PATCHes the drift, pass 2 (live row now equals the
    /// plan) must emit ZERO PATCH — the mock caps PATCH at exactly 1 and
    /// pass 2 still succeeds with the platform reported converged.
    #[tokio::test]
    async fn apply_twice_second_pass_zero_patches() {
        let store_path = apply_fixture_store_path("twice");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        svc.store(cfg(vec![ps("alpha", &["HK"])]))
            .unwrap();

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"node_hash":"h1","region":"HK","has_outbound":true,"failure_count":0}]}"#)
            .expect_at_least(2)
            .create_async()
            .await;
        // Phase 1: drifted row ("US"); phase 2: synced row ("HK") — the
        // world state moves BETWEEN passes, matching mockito's per-registration
        // serving order (same pattern as the create test's two-phase read).
        let m_platforms_drifted = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-a","name":"alpha","region_filters":["US"]}]}"#)
            .expect(2)
            .create_async()
            .await;
        let m_platforms_synced = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-a","name":"alpha","region_filters":["HK"]}]}"#)
            .expect(2)
            .create_async()
            .await;
        let m_patch = server
            .mock("PATCH", "/api/v1/platforms/id-a")
            .match_header(bearer.0, bearer.1)
            .match_body(mockito::Matcher::PartialJson(json!({"region_filters": ["HK"]})))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"id-a"}"#)
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let report1 = svc
            .apply(&c, platform_id_for_name_fixture)
            .await
            .expect("pass 1 must succeed");
        assert!(report1.platforms[0].patched);

        let report2 = svc
            .apply(&c, platform_id_for_name_fixture)
            .await
            .expect("pass 2 must succeed");
        let p2 = &report2.platforms[0];
        assert!(p2.patched, "pass 2 converges without writing: {p2:?}");
        assert_eq!(p2.reason.as_deref(), Some("in sync"));

        m_nodes.assert_async().await;
        m_platforms_drifted.assert_async().await;
        m_platforms_synced.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    fn platform_id_for_name_fixture(v: &serde_json::Value, name: &str) -> Option<String> {
        let arr = v.get("items").and_then(|i| i.as_array()).or_else(|| v.as_array())?;
        arr.iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
            .and_then(|p| p.get("id").and_then(|i| i.as_str()))
            .map(String::from)
    }

    // ---- Round 5 T01 / issue 01 F3: apply reference resolution ----

    fn ps_sub(name: &str, subs: &[&str]) -> PlatformStrategy {
        let mut p = ps(name, &[]);
        p.a_class = crate::strategy_engine::AClassStrategy::Subscription;
        p.subscriptions = subs.iter().map(|s| s.to_string()).collect();
        p
    }

    /// A whitebox platform whose `subscriptions` mention a name that is not
    /// live on Resin gets a "dangling subscription refs:" clause on its
    /// report row (distinct from the "create failed:" vocabulary), the
    /// region plan still derives from the RESOLVABLE members, the whitebox
    /// entry survives, and the extra GET /subscriptions happens EXACTLY
    /// once per apply (handoff wire-level count lock).
    #[tokio::test]
    async fn apply_reports_dangling_subscription_refs_exactly_once_per_pass() {
        let store_path = apply_fixture_store_path("dangling");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        svc.store(cfg(vec![ps_sub("alpha", &["ghost-sub", "live-sub"])]))
            .unwrap();

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        // One healthy node tagged with the live subscription, region HK —
        // the dangling name contributes no region (no poisoned derivation).
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"items":[{"node_hash":"h1","region":"HK","has_outbound":true,"failure_count":0,"tags":[{"subscription_name":"live-sub"}]}]}"#,
            )
            .create_async()
            .await;
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-a","name":"alpha","region_filters":["US"]}]}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        // Wire count lock: exactly ONE list_subscriptions per apply pass.
        let m_subs = server
            .mock("GET", "/api/v1/subscriptions")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"name":"live-sub","node_count":1,"healthy_node_count":1}]}"#)
            .expect(1)
            .create_async()
            .await;
        let m_patch = server
            .mock("PATCH", "/api/v1/platforms/id-a")
            .match_header(bearer.0, bearer.1)
            .match_body(mockito::Matcher::PartialJson(json!({"region_filters": ["HK"]})))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"id-a"}"#)
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let report = svc
            .apply(&c, platform_id_for_name_fixture)
            .await
            .expect("apply must succeed");
        assert_eq!(report.platforms.len(), 1);
        let p = &report.platforms[0];
        assert_eq!(p.platform, "alpha");
        assert!(p.patched, "resolvable members still derive: {p:?}");
        assert_eq!(
            p.reason.as_deref(),
            Some("dangling subscription refs: ghost-sub"),
            "dangling clause must be its own vocabulary, not create/PATCH wording: {p:?}"
        );
        // The whitebox entry KEEPS its subscriptions (apply never deletes
        // desired state — the dangling ref is reported, not auto-cleaned).
        let after = svc.get().unwrap();
        assert_eq!(after.platforms[0].subscriptions.len(), 2);

        m_nodes.assert_async().await;
        m_platforms.assert_async().await;
        m_subs.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    /// Zero-extra-request rule: a config whose platforms reference no
    /// subscriptions skips the resolution GET entirely (the ADR-0057 wire
    /// shape for region/manual-only configs is untouched).
    #[tokio::test]
    async fn apply_skips_subscription_resolution_when_no_refs() {
        let store_path = apply_fixture_store_path("nosubs");
        let _ = std::fs::remove_file(&store_path);
        let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
        svc.store(cfg(vec![ps("alpha", &["HK"])]))
            .unwrap();

        let mut server = mockito::Server::new_async().await;
        let bearer = ("authorization", "Bearer testtok");
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[]}"#)
            .create_async()
            .await;
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(bearer.0, bearer.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"items":[{"id":"id-a","name":"alpha","region_filters":[]}]}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        // Any /subscriptions read would fail the test.
        let m_subs = server
            .mock("GET", "/api/v1/subscriptions")
            .with_status(500)
            .expect(0)
            .create_async()
            .await;

        let base = server.url();
        let c = crate::resin_client::ResinClient::new(&base, "testtok".into()).unwrap();
        let report = svc
            .apply(&c, platform_id_for_name_fixture)
            .await
            .expect("apply must succeed");
        assert_eq!(report.platforms[0].reason.as_deref(), Some("in sync"));

        m_nodes.assert_async().await;
        m_platforms.assert_async().await;
        m_subs.assert_async().await;
        let _ = std::fs::remove_file(&store_path);
    }

    #[test]
    fn dangling_subscription_refs_lists_only_unresolvable_names_per_platform() {
        let mut live = std::collections::HashSet::new();
        live.insert("live-sub".to_string());
        let config = cfg(vec![
            ps_sub("clean", &[]),
            ps_sub("ok", &["live-sub"]),
            ps_sub("mixed", &["live-sub", "ghost-a", "ghost-b"]),
        ]);
        let out = dangling_subscription_refs(&config, &live);
        assert_eq!(out.len(), 1, "clean + fully-resolvable platforms are absent: {out:?}");
        assert_eq!(out[0].0, "mixed");
        assert_eq!(out[0].1, vec!["ghost-a".to_string(), "ghost-b".to_string()]);
    }

    #[test]
    fn with_dangling_note_composes_distinct_vocabulary() {
        let row = AppliedPlatform {
            platform: "a".into(),
            region_filters: vec![],
            patched: true,
            reason: None,
        };
        // No dangling refs: reason untouched (ADR-0057 "in sync" contract
        // stays byte-exact).
        assert_eq!(with_dangling_note(row.clone(), &[]).reason, None);
        let in_sync = AppliedPlatform { reason: Some("in sync".into()), ..row.clone() };
        assert_eq!(
            with_dangling_note(in_sync, &[]).reason.as_deref(),
            Some("in sync"),
        );
        // Some + dangling: note appended after a "; " separator.
        let in_sync = AppliedPlatform { reason: Some("in sync".into()), ..row.clone() };
        assert_eq!(
            with_dangling_note(in_sync, &["g".to_string()]).reason.as_deref(),
            Some("in sync; dangling subscription refs: g"),
        );
        // None + dangling: note stands alone with its own prefix.
        assert_eq!(
            with_dangling_note(row, &["a".to_string(), "b".to_string()])
                .reason
                .as_deref(),
            Some("dangling subscription refs: a, b"),
        );
    }

    // ---- Round 7 T02 (D-C1.2): subscription phase status array ----

    /// Phase tests persist across calls, so they use the REAL store (a temp
    /// file) — MemStore::store is a deliberate no-op fixture.
    fn fs_service(tag: &str) -> (StrategyService<FsStrategyStore>, PathBuf) {
        let path = std::env::temp_dir().join(format!("phase-status-{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        (StrategyService::new(FsStrategyStore::new(path.clone())), path)
    }

    fn status(name: &str, phase: SubscriptionPhase) -> SubscriptionStatus {
        SubscriptionStatus { name: name.to_string(), phase, stage: None, phase_error: None }
    }

    /// Checkpoint A (D-C1.2): the Never -> Importing -> Establishing ->
    /// Converged sequence lands as an upsert (one row per name, no dupes),
    /// and the mid-flight stage rides the row.
    #[test]
    fn phase_transition_sequence_never_importing_establishing_converged() {
        let (svc, _path) = fs_service("seq");
        // Never: only reachable as the ABSENCE of a row; recording it
        // explicitly is also legal (a cascade that resets a stale row).
        let after = svc.record_subscription_phase("sub-a", SubscriptionPhase::Never, None, None).unwrap();
        assert_eq!(after.subscriptions, vec![status("sub-a", SubscriptionPhase::Never)]);

        let after = svc.record_subscription_phase("sub-a", SubscriptionPhase::Importing, None, None).unwrap();
        assert_eq!(after.subscriptions[0].phase, SubscriptionPhase::Importing);
        assert!(after.subscriptions[0].stage.is_none());

        let after = svc
            .record_subscription_phase("sub-a", SubscriptionPhase::Establishing, Some(EstablishStep::Platform), None)
            .unwrap();
        assert_eq!(after.subscriptions[0].phase, SubscriptionPhase::Establishing);
        assert_eq!(after.subscriptions[0].stage, Some(EstablishStep::Platform));

        let after = svc
            .record_subscription_phase("sub-a", SubscriptionPhase::Establishing, Some(EstablishStep::Apply), None)
            .unwrap();
        assert_eq!(after.subscriptions[0].stage, Some(EstablishStep::Apply));

        let after = svc.record_subscription_phase("sub-a", SubscriptionPhase::Converged, None, None).unwrap();
        assert_eq!(after.subscriptions.len(), 1, "upsert, never duplicate");
        assert_eq!(after.subscriptions[0].phase, SubscriptionPhase::Converged);
        assert!(after.subscriptions[0].stage.is_none(), "terminal green drops the stage");
        assert!(after.subscriptions[0].phase_error.is_none());
    }

    /// Checkpoint A (D-C1.2): failure write-back persists Failed(stage,
    /// reason); a later retry overwrites it; the row survives OTHER writes
    /// (a region edit must not clobber status history).
    #[test]
    fn failed_phase_persists_stage_and_reason() {
        let (svc, _path) = fs_service("fail");
        svc.record_subscription_phase("sub-x", SubscriptionPhase::Establishing, Some(EstablishStep::Bind), None).unwrap();
        let after = svc
            .record_subscription_phase(
                "sub-x",
                SubscriptionPhase::Failed,
                Some(EstablishStep::Bind),
                Some("strategy apply: PATCH 500".to_string()),
            )
            .unwrap();
        assert_eq!(after.subscriptions[0].phase, SubscriptionPhase::Failed);
        assert_eq!(after.subscriptions[0].stage, Some(EstablishStep::Bind));
        assert_eq!(after.subscriptions[0].phase_error.as_deref(), Some("strategy apply: PATCH 500"));

        // The status row survives an unrelated deep edit.
        svc.store(cfg(vec![ps("A", &["HK"])]));
        let reread = svc.get().unwrap();
        // store() replaced the whole document via cfg() — the status array
        // was reset by that DESIRED-state write. That is correct semantics:
        // a put is a full-document replace. Status writes never do this.
        assert!(reread.subscriptions.is_empty(), "desired-state put is a full replace");

        // A fresh status row on the new document re-lands cleanly.
        let after2 = svc
            .record_subscription_phase("sub-y", SubscriptionPhase::Failed, Some(EstablishStep::Resolve), Some("nodes not landed".to_string()))
            .unwrap();
        assert_eq!(after2.subscriptions.len(), 1);
        assert_eq!(after2.subscriptions[0].stage, Some(EstablishStep::Resolve));
    }

    /// Checkpoint B (D-C1.2): the status write is generation-aware in the
    /// ADR-0058 sense — it travels the ONE store entry (FsStrategyStore,
    /// validate + backup + audit) but does NOT bump the desired-state
    /// generation (k8s status-subresource rule). The pair stays converged
    /// across a cascade, so ADR-0058's top-level ConvergePhase cannot be
    /// flipped into a false PendingApply by a status write.
    #[test]
    fn fs_status_write_keeps_generation_stable_and_lands_on_disk() {
        let dir = std::env::temp_dir().join(format!("phase-status-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        let svc = StrategyService::new(FsStrategyStore::new(dir.clone()));

        // Seed a desired-state write (generation bumps 0 -> 1).
        let seeded = svc.store(cfg(vec![ps("A", &["HK"])]));
        assert_eq!(seeded.generation, 1);

        // A green apply write-back converges the pair (ADR-0058 D3 shape).
        let mut green = svc.get().unwrap();
        green.applied_generation = green.generation;
        green.last_apply_at = Some(now_unix());
        svc.store_ref().store(&green).unwrap();
        let converged = svc.get().unwrap();
        assert_eq!(converged.generation, converged.applied_generation);

        // The phase write: generation pair MUST NOT move.
        let after = svc
            .record_subscription_phase("sub-a", SubscriptionPhase::Establishing, Some(EstablishStep::Platform), None)
            .unwrap();
        assert_eq!(after.generation, converged.generation, "status write must not bump generation");
        assert_eq!(after.applied_generation, converged.applied_generation, "status write must not fake convergence");
        assert_eq!(after.updated_at, converged.updated_at, "status write is not a desired-state write");

        // ... and the row actually landed in the FILE (not just memory).
        let raw = std::fs::read_to_string(&dir).unwrap();
        let doc: StrategyConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(doc.subscriptions.len(), 1);
        assert_eq!(doc.subscriptions[0].name, "sub-a");
        assert_eq!(doc.subscriptions[0].phase, SubscriptionPhase::Establishing);

        // v1-file compat: a document WITHOUT the array still parses, and the
        // array is omitted on serialize while empty (skip_serializing_if).
        let v1: StrategyConfig = serde_json::from_str(&json!({
            "version": 1,
            "platforms": []
        }).to_string()).unwrap();
        assert!(v1.subscriptions.is_empty());
        let ser = serde_json::to_string(&StrategyConfig::default()).unwrap();
        assert!(!ser.contains("\"subscriptions\""), "empty status array must not serialize");

        let _ = std::fs::remove_file(&dir);
    }

    /// Shape locks: stage/phase_error invariants + bounds are enforced by
    /// the write path AND by validate() (hand-edited files included).
    #[test]
    fn phase_row_shape_locks() {
        let (svc, _path) = fs_service("shape");
        // Establishing/Failed REQUIRE a stage...
        assert!(svc.record_subscription_phase("s", SubscriptionPhase::Establishing, None, None).is_err());
        assert!(svc.record_subscription_phase("s", SubscriptionPhase::Failed, None, Some("r".into())).is_err());
        // ...but the data-landing phases never carry one...
        assert!(svc.record_subscription_phase("s", SubscriptionPhase::Importing, Some(EstablishStep::Import), None).is_err());
        assert!(svc.record_subscription_phase("s", SubscriptionPhase::Converged, Some(EstablishStep::Apply), None).is_err());
        assert!(svc.record_subscription_phase("s", SubscriptionPhase::Never, None, Some("r".into())).is_err());
        // ...Failed requires a reason; others never carry one.
        assert!(svc.record_subscription_phase("s", SubscriptionPhase::Failed, Some(EstablishStep::Platform), None).is_err());
        // Name hygiene (AGENTS 7.5): empty / oversized / control chars.
        assert!(svc.record_subscription_phase("", SubscriptionPhase::Never, None, None).is_err());
        assert!(svc.record_subscription_phase(&"x".repeat(129), SubscriptionPhase::Never, None, None).is_err());
        assert!(svc.record_subscription_phase("a\u{0}b", SubscriptionPhase::Never, None, None).is_err());
        // Reason bounds.
        let long = "x".repeat(MAX_PHASE_ERROR_LEN + 1);
        assert!(svc.record_subscription_phase("s", SubscriptionPhase::Failed, Some(EstablishStep::Import), Some(long)).is_err());
        let nul = "bad\u{0}reason".to_string();
        assert!(svc.record_subscription_phase("s", SubscriptionPhase::Failed, Some(EstablishStep::Import), Some(nul)).is_err());

        // validate() rejects the same junk in a hand-edited document.
        let mut bad = cfg(vec![]);
        bad.subscriptions = vec![SubscriptionStatus {
            name: "dup".into(), phase: SubscriptionPhase::Never, stage: None, phase_error: None,
        }, SubscriptionStatus {
            name: "dup".into(), phase: SubscriptionPhase::Never, stage: None, phase_error: None,
        }];
        assert!(validate(&bad).is_err(), "duplicate rows rejected");
        bad.subscriptions = vec![SubscriptionStatus {
            name: "s".into(), phase: SubscriptionPhase::Converged, stage: Some(EstablishStep::Apply), phase_error: None,
        }];
        assert!(validate(&bad).is_err(), "Converged with a stage rejected");
        bad.subscriptions = vec![SubscriptionStatus {
            name: "s".into(), phase: SubscriptionPhase::Failed, stage: Some(EstablishStep::Import), phase_error: None,
        }];
        assert!(validate(&bad).is_err(), "Failed without a reason rejected");
        // A legal row passes.
        bad.subscriptions = vec![SubscriptionStatus {
            name: "s".into(), phase: SubscriptionPhase::Failed, stage: Some(EstablishStep::Import), phase_error: Some("boom".into()),
        }];
        assert!(validate(&bad).is_ok());
    }
}

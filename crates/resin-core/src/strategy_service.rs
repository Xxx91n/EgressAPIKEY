//! StrategyService — the ONE deep module owning the strategyConfig pipeline
//! (architecture-recovery ticket 10; ADR-0052).
//!
//! Vocabulary (see ADR-0052 for the full three-vocabulary map):
//! - `strategy.rs` owns the B-class catalog (`StrategyId` 6 shell options +
//!   protocol weight table) — the UI-facing strategy vocabulary.
//! - `strategy_engine.rs` owns the A-class planner (`StrategyConfig`,
//!   `compute_plan`, `parse_nodes`) — the region-computation vocabulary.
//! - `src/lib/strategy.ts` (frontend) maps `StrategyId` to i18n keys and to
//!   Resin's `allocation_policy` enum — a pure display/mapping layer.
//! This module is the *ownership* layer: read (whitebox JSON), validate,
//! store (write-back), apply (compute plan + PATCH Resin + auto-clean the
//! whitebox file), snapshot (ticket 07 read-back), and a deep
//! `set_platform_regions` edit used by the topology canvas.
//!
//! ADR-0036 discipline: the whitebox file is the truth; `store` is the ONLY
//! writer in the shell (the former duplicate writers in the command layer are
//! gone). ADR-0039 SS2: every strategy field still travels to views through
//! the snapshot (unchanged shape).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;

use serde::Serialize;

use crate::strategy_engine::{compute_plan, parse_nodes, NodeSummary, PlatformStrategy, StrategyConfig};
use crate::whitebox_backup::{
    atomic_write_bytes, backup_before_write, backup_list, now_unix, read_backup_parsed,
    WhiteboxBackupEntry,
};
use crate::snapshot::ResinPlatformRuntime;
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
/// "连续两次执行第二次零变更"). The strategy half is naturally idempotent
/// (PATCHing the computed plan twice is a no-op); the ports half is not —
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
/// a filesystem (loop-me: fixtures exist for both deleted-platform and
/// all-alive auto-clean cases).
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
        std::fs::write(&self.path, json).map_err(|e| e.to_string())
    }
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
    validate_acknowledged(&config.acknowledged, "acknowledged")
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

/// Auto-clean: drop platform entries whose names no longer exist in Resin.
/// Returns the cleaned clone plus whether anything was dropped (the caller
/// persists only when `changed` is true — same behavior as the former
/// command-layer read-time write, now as a pure, tested function).
pub fn clean_stale(config: &StrategyConfig, live_names: &std::collections::HashSet<String>) -> (StrategyConfig, bool) {
    let before = config.platforms.len();
    let platforms: Vec<PlatformStrategy> = config
        .platforms
        .iter()
        .filter(|ps| live_names.contains(&ps.platform_name))
        .cloned()
        .collect();
    let cleaned = StrategyConfig {
        version: config.version,
        acknowledged: config.acknowledged.clone(),
        platforms,
    };
    let changed = cleaned.platforms.len() != before;
    (cleaned, changed)
}

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
    pub fn store(&self, config: &StrategyConfig) -> Result<(), String> {
        validate(config)?;
        self.store.store(config)
    }

    /// Compute the A-class plan without touching Resin (pure read used by
    /// tests and the B-class snapshot path).
    pub fn plan(&self, nodes: &[NodeSummary]) -> Result<HashMap<String, Vec<String>>, String> {
        let config = self.get()?;
        Ok(compute_plan(&config, nodes))
    }

    /// Persist back only when cleaning actually dropped entries.
    fn persist_cleaned_if_changed(
        &self,
        original: &StrategyConfig,
        cleaned: &StrategyConfig,
        changed: bool,
    ) {
        if !changed {
            return;
        }
        tracing::info!(
            before = original.platforms.len(),
            after = cleaned.platforms.len(),
            "strategy_apply: auto-cleaned stale platform entries from strategyConfig"
        );
        // Best-effort persistence: a failed clean-back must not fail the apply
        // (historical behavior; the next apply re-cleans).
        if let Err(e) = self.store.store(cleaned) {
            tracing::warn!(error = %e, "strategy_apply: persisting cleaned config failed");
        }
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
        self.store(&config)?;
        Ok(config)
    }

}

impl StrategyService<FsStrategyStore> {
    /// Full apply: read whitebox -> parse nodes -> auto-clean stale platforms
    /// (persist when changed) -> compute plan -> PATCH region_filters per
    /// platform. PATCH failures are reported per-platform, never fatal.
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
        let (cleaned, changed) = clean_stale(&config, &live_names);
        self.persist_cleaned_if_changed(&config, &cleaned, changed);

        let plan = compute_plan(&cleaned, &nodes);
        let mut platforms = Vec::new();
        for (platform_name, regions) in &plan {
            let platforms_v = client
                .list_platforms()
                .await
                .map_err(|e| e.to_string())?;
            if let Some(id) = platform_id_for_name(&platforms_v, platform_name) {
                let body = serde_json::json!({ "region_filters": regions });
                match client.update_platform(&id, body).await {
                    Ok(_) => platforms.push(AppliedPlatform {
                        platform: platform_name.clone(),
                        region_filters: regions.clone(),
                        patched: true,
                        reason: None,
                    }),
                    Err(e) => {
                        tracing::warn!(platform = %platform_name, error = %e.to_string(), "auto_strategy_apply: PATCH region_filters failed");
                        platforms.push(AppliedPlatform {
                            platform: platform_name.clone(),
                            region_filters: regions.clone(),
                            patched: false,
                            reason: Some(format!("PATCH failed: {e}")),
                        });
                    }
                }
            } else {
                platforms.push(AppliedPlatform {
                    platform: platform_name.clone(),
                    region_filters: regions.clone(),
                    patched: false,
                    reason: Some("platform not found".to_string()),
                });
            }
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
        StrategyConfig { version: 1, platforms, acknowledged: vec![] }
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

    // ---- clean_stale: the two required fixtures ----
    #[test]
    fn clean_stale_drops_deleted_platforms() {
        let c = cfg(vec![ps("Alive", &["US"]), ps("Deleted", &["HK"])]);
        let live: std::collections::HashSet<String> = ["Alive".to_string()].into_iter().collect();
        let (cleaned, changed) = clean_stale(&c, &live);
        assert!(changed);
        assert_eq!(cleaned.platforms.len(), 1);
        assert_eq!(cleaned.platforms[0].platform_name, "Alive");
    }

    #[test]
    fn clean_stale_keeps_everything_when_all_alive() {
        let c = cfg(vec![ps("A", &["US"]), ps("B", &["HK"])]);
        let live: std::collections::HashSet<String> = ["A".to_string(), "B".to_string()]
            .into_iter()
            .collect();
        let (cleaned, changed) = clean_stale(&c, &live);
        assert!(!changed);
        assert_eq!(cleaned.platforms.len(), 2);
        assert_eq!(cleaned, c);
    }

    // ---- store (write entry) ----
    #[test]
    fn store_rejects_invalid_documents_before_writing() {
        let svc = StrategyService::new(MemStore(serde_json::Value::Null));
        let mut bad = cfg(vec![ps("A", &["US"])]);
        bad.version = 9;
        assert!(svc.store(&bad).is_err());
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
        fsvc.store(&cfg_doc).unwrap();
        let reloaded = fsvc.get().unwrap();
        assert_eq!(reloaded.acknowledged, vec!["A".to_string()]);
        // skip_serializing_if: empty list writes NO acknowledged key.
        let mut empty = cfg(vec![]);
        empty.acknowledged = vec![];
        fsvc.store(&empty).unwrap();
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

    #[test]
    fn clean_stale_preserves_acknowledged_list() {
        let mut c = cfg(vec![ps("Alive", &["US"]), ps("Deleted", &["HK"])]);
        c.acknowledged = vec!["Alive".to_string(), "Deleted".to_string()];
        let live: std::collections::HashSet<String> = ["Alive".to_string()].into_iter().collect();
        let (cleaned, changed) = clean_stale(&c, &live);
        assert!(changed);
        // The exemption list itself is NOT auto-cleaned: it is a user-marked
        // exemption list, not strategy intent; surfacing is read-side only.
        assert_eq!(cleaned.acknowledged, vec!["Alive".to_string(), "Deleted".to_string()]);
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
        // store + reload
        let c = cfg(vec![ps("Anthropic", &["US", "HK"])]);
        svc.store(&c).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"platform_name\""));
        let reloaded = svc.get().unwrap();
        assert_eq!(reloaded, c);
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

    fn platform_id_for_name_fixture(v: &serde_json::Value, name: &str) -> Option<String> {
        let arr = v.get("items").and_then(|i| i.as_array()).or_else(|| v.as_array())?;
        arr.iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
            .and_then(|p| p.get("id").and_then(|i| i.as_str()))
            .map(String::from)
    }
}

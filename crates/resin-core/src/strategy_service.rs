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

use serde::Serialize;

use crate::strategy_engine::{compute_plan, parse_nodes, NodeSummary, PlatformStrategy, StrategyConfig};

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

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ApplyReport {
    pub platforms: Vec<AppliedPlatform>,
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
    Ok(())
}

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
        StrategyConfig { version: 1, platforms }
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
}

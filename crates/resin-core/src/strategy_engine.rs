//! Shell-side strategy engine (T4-4 / ADR-0022).
//!
//! Two strategy classes:
//! - A-class: which IPs enter a platform (manual / region / quality / subscription)
//! - B-class: how a platform's exit IP is selected (random / round_robin / low_latency)
//!
//! B-class catalog already lives in `strategy.rs` (maps to Resin allocation_policy).
//! This module owns:
//!   1. A-class filter strategies that poll /nodes and compute region_filters
//!      to PATCH onto each platform.
//!   2. A per-platform strategy config document (whitebox JSON).
//!   3. Liveness gate: Resin ProbeManager already does health probing + circuit
//!      breaker, so we only filter on `has_outbound && failure_count == 0`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::strategy::StrategyId;

/// A-class strategy: which IPs enter a platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AClassStrategy {
    /// Only manually-specified regions/codes in the config.
    Manual,
    /// All healthy nodes from specific regions.
    Region,
    /// Top-N nodes by quality score (lowest failure_count, lowest latency).
    Quality,
    /// All healthy nodes from specific subscription sources.
    Subscription,
}

impl AClassStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Region => "region",
            Self::Quality => "quality",
            Self::Subscription => "subscription",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "manual" => Ok(Self::Manual),
            "region" => Ok(Self::Region),
            "quality" => Ok(Self::Quality),
            "subscription" => Ok(Self::Subscription),
            other => Err(format!("unknown A-class strategy: {other}")),
        }
    }
}

/// T18-3 (ADR-0042 S3): B-class strategy parameters (shell-side whitebox only).
///
/// These are display-only parameters surfaced on the canvas platform card badge.
/// Resin v1.2.0 only accepts `allocation_policy` enum; the per-strategy params
/// are shell-side hints for the GUI (round-robin N, latency threshold ms, quality
/// score floor, bandwidth weight). Kept optional so older configs without these
/// fields still deserialize.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BClassParams {
    #[serde(default)]
    pub round_robin_n: Option<u32>,
    #[serde(default)]
    pub latency_threshold_ms: Option<u32>,
    #[serde(default)]
    pub quality_score: Option<u32>,
    #[serde(default)]
    pub bandwidth_weight: Option<u32>,
}

impl Default for BClassParams {
    fn default() -> Self {
        Self {
            round_robin_n: None,
            latency_threshold_ms: None,
            quality_score: None,
            bandwidth_weight: None,
        }
    }
}

/// Per-platform strategy config entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlatformStrategy {
    /// Platform name (matches Resin /platforms name field).
    pub platform_name: String,
    /// A-class: which IPs go into this platform.
    pub a_class: AClassStrategy,
    /// B-class: how to pick exit IP (maps to Resin allocation_policy).
    pub b_class: StrategyId,
    /// For manual strategy: list of selected node hashes.
    #[serde(default)]
    pub manual_nodes: Vec<String>,
    /// For region strategy: list of allowed region codes.
    #[serde(default)]
    pub regions: Vec<String>,
    /// For subscription strategy: list of allowed subscription names.
    #[serde(default)]
    pub subscriptions: Vec<String>,
    /// For quality strategy: max number of nodes to include.
    #[serde(default = "default_quality_top_n")]
    pub top_n: usize,
    /// T18-3: B-class strategy parameters for GUI badge interpolation.
    #[serde(default)]
    pub b_class_params: BClassParams,
}

fn default_quality_top_n() -> usize {
    10
}

/// Round 7 T02 (spec D-C1.2): user-facing establish-cascade phase for ONE
/// subscription. Persisted as a STATUS row in the strategy whitebox
/// (`SubscriptionStatus`, top-level `subscriptions` array — the per-platform
/// `PlatformStrategy::subscriptions` NAME REFS are a different field).
/// Wire tags mirror the `ConvergePhase` style: unit variants serialize as
/// their PascalCase variant name.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubscriptionPhase {
    /// No establish cascade has ever run for this subscription. The ABSENT
    /// status row also reads as `Never` — the state machine's identity
    /// element (a v1 file without the array is all-Never, zero migration).
    Never,
    /// The data-landing beats are running (create_subscription / resolve).
    Importing,
    /// The whitebox+apply beats are running; the sub-step rides the
    /// `stage` column (platform/bind/port/apply per spec D-C1.2).
    Establishing,
    /// Terminal green: every cascade step wrote or skipped.
    Converged,
    /// A cascade step failed persistently; `stage` says where and
    /// `phase_error` carries the KEP-1623-style reason.
    Failed,
    /// Reserved (T03 owns the first producer): a default-port conflict or
    /// similar needs explicit user consent before the cascade continues.
    NeedsApproval,
}

/// Cascade sub-step tag for `Establishing` / `Failed` status rows. Serde
/// tags are stable snake_case, matching the per-entry state_tag style.
/// `Import`/`Resolve` appear ONLY on `Failed` rows (data-landing failures);
/// `Establishing` uses the spec's platform/bind/port/apply sub-steps.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EstablishStep {
    /// create_subscription beat (the import POST itself).
    Import,
    /// resolve beat (nodes not landed yet — Resin fetcher still pulling).
    Resolve,
    /// establish-platform beat (whitebox entry + ADR-0056 create seam).
    Platform,
    /// strategy_config_put beat (the subscriptions-ref binding; fused into
    /// the platform write today, kept distinct for the wire contract).
    Bind,
    /// Default-port binding (T03 owns the producer; reserved tag).
    Port,
    /// strategy_apply beat (ADR-0057 diff-then-skip + ADR-0058 write-back).
    Apply,
}

/// One subscription's establish-phase STATUS row in the strategy whitebox
/// (`egressapikey-strategy.json` top-level `subscriptions` array). STATUS,
/// not spec: writes go through the ONE store entry (validate + versioned
/// backup + audit — the generation-aware path) but do NOT bump the
/// desired-state generation. k8s status-subresource rule: a status write is
/// not a write-authority change; bumping would flip ADR-0058's top-level
/// ConvergePhase into a false PendingApply after every successful cascade
/// (spec D-C1.6: existing ADR conclusions must not be broken).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriptionStatus {
    /// Subscription name — the row key (1..128 chars, unique in the array).
    pub name: String,
    pub phase: SubscriptionPhase,
    /// Present only while Establishing / Failed (see `EstablishStep`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<EstablishStep>,
    /// Present only while Failed — the reason (KEP-1623 style; ≤1024
    /// chars, NUL rejected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_error: Option<String>,
}

/// The whitebox strategy config document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StrategyConfig {
    pub version: u8,
    #[serde(default)]
    pub platforms: Vec<PlatformStrategy>,
    /// Ticket 12 / ADR-0054 §D: optional exemption list. Members are platform
    /// names the user has marked "known drift, don't notify". Parse-compat:
    /// absent = empty (older configs load unchanged). The list NEVER enters
    /// the three-state merge — it is surfaced read-side only. Validate caps:
    /// ≤64 members × 1..128 chars (no control chars), duplicates rejected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acknowledged: Vec<String>,
    /// Round 7 T02 (D-C1.2): per-subscription establish-phase STATUS rows.
    /// ABSENT in a v1 file = empty = every subscription reads phase `Never`
    /// (the same zero-migration serde-default story as `generation`).
    /// Writes go ONLY through `StrategyService::record_subscription_phase`
    /// (status subresource — no generation bump).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<SubscriptionStatus>,
    /// Round 5 T09 / ADR-0058: write-authority generation counter. Bumped by
    /// EVERY sanctioned store-path write (service `store`, deep region edit,
    /// rollback) after validate, before the file lands. Absent in a v1 file
    /// = 0 = "never applied" (k8s habit: a fresh boot is not a fake alarm).
    #[serde(default)]
    pub generation: u64,
    /// Observed generation: the value `generation` had when the last FULLY
    /// GREEN apply pass returned. Written back inside the same store entry by
    /// `StrategyService::apply` (R-B Q2); failures keep the old value and
    /// record `last_apply_error` instead.
    #[serde(default)]
    pub applied_generation: u64,
    /// Unix seconds of the last apply pass that returned green (refreshed
    /// even for a diff-then-skip zero-PATCH pass, Terraform re-apply style).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_apply_at: Option<u64>,
    /// KEP-1623 style failure record: reason+message of the last apply pass
    /// that did NOT return fully green. Cleared by the next green pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_apply_error: Option<String>,
    /// Unix seconds of the last whitebox write (any store-path write,
    /// including non-apply edits). Pure metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            version: 1,
            platforms: vec![],
            acknowledged: vec![],
            subscriptions: vec![],
            generation: 0,
            applied_generation: 0,
            last_apply_at: None,
            last_apply_error: None,
            updated_at: None,
        }
    }
}

/// A node summary extracted from Resin GET /api/v1/nodes items.
#[derive(Debug, Clone)]
pub struct NodeSummary {
    pub node_hash: String,
    pub region: String,
    pub has_outbound: bool,
    pub failure_count: i64,
    pub reference_latency_ms: Option<f64>,
    pub subscription_name: Option<String>,
}

/// Extract NodeSummary items from a Resin /nodes JSON response.
/// Accepts both items-wrapper and bare-array shapes.
pub fn parse_nodes(v: &serde_json::Value) -> Vec<NodeSummary> {
    let arr = if let Some(items) = v.get("items") {
        items.as_array()
    } else {
        v.as_array()
    };
    let Some(arr) = arr else { return vec![] };

    arr.iter()
        .filter_map(|n| {
            let node_hash = n.get("node_hash")?.as_str()?.to_string();
            let region = n.get("region").and_then(|r| r.as_str()).unwrap_or("").to_string();
            let has_outbound = n.get("has_outbound").and_then(|h| h.as_bool()).unwrap_or(false);
            let failure_count = n.get("failure_count").and_then(|f| f.as_i64()).unwrap_or(0);
            let reference_latency_ms = n
                .get("reference_latency_ms")
                .and_then(|l| l.as_f64());
            // tags[] -> subscription_name (snake_case from Resin API)
            let subscription_name = n
                .get("tags")
                .and_then(|t| t.as_array())
                .and_then(|tags| tags.first())
                .and_then(|tag| {
                    tag.get("subscription_name")
                        .or_else(|| tag.get("subscriptionName"))
                        .and_then(|s| s.as_str())
                        .map(String::from)
                });
            Some(NodeSummary {
                node_hash,
                region,
                has_outbound,
                failure_count,
                reference_latency_ms,
                subscription_name,
            })
        })
        .collect()
}

/// Liveness gate: only nodes that are healthy (has_outbound + no failures).
/// Resin ProbeManager already probes; this is the shell-side filter.
pub fn liveness_filter(nodes: &[NodeSummary]) -> Vec<&NodeSummary> {
    nodes
        .iter()
        .filter(|n| n.has_outbound && n.failure_count == 0)
        .collect()
}

/// Apply an A-class strategy to a list of healthy nodes and produce
/// the set of region codes that should be PATCHed into region_filters.
pub fn a_class_regions(strategy: &PlatformStrategy, healthy: &[&NodeSummary]) -> Vec<String> {
    match strategy.a_class {
        AClassStrategy::Manual => {
            // Map manual_nodes (node hashes) to regions by looking up each hash
            // in the healthy nodes list. GUI writes manual_nodes, NOT regions.
            let allowed: HashSet<&str> = strategy.manual_nodes.iter().map(|s| s.as_str()).collect();
            let mut result: Vec<String> = healthy
                .iter()
                .filter_map(|n| {
                    if allowed.contains(n.node_hash.as_str()) {
                        Some(n.region.clone())
                    } else {
                        None
                    }
                })
                .collect();
            result.sort();
            result.dedup();
            result
        }
        AClassStrategy::Region => {
            // Collect all unique regions from healthy nodes that match the config regions
            let allowed: HashSet<&str> = strategy.regions.iter().map(|s| s.as_str()).collect();
            let mut result: Vec<String> = healthy
                .iter()
                .filter_map(|n| {
                    if allowed.is_empty() || allowed.contains(n.region.as_str()) {
                        Some(n.region.clone())
                    } else {
                        None
                    }
                })
                .collect();
            result.sort();
            result.dedup();
            result
        }
        AClassStrategy::Quality => {
            // Sort by latency (None = worst), take top_n nodes, collect their regions
            let mut sorted: Vec<&&NodeSummary> = healthy.iter().collect();
            sorted.sort_by(|a, b| {
                let la = a.reference_latency_ms.unwrap_or(f64::MAX);
                let lb = b.reference_latency_ms.unwrap_or(f64::MAX);
                la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal)
            });
            let top = sorted.into_iter().take(strategy.top_n);
            let mut result: Vec<String> = top.map(|n| n.region.clone()).collect();
            result.sort();
            result.dedup();
            result
        }
        AClassStrategy::Subscription => {
            // Collect regions of healthy nodes from specified subscriptions
            let allowed: HashSet<&str> = strategy.subscriptions.iter().map(|s| s.as_str()).collect();
            let mut result: Vec<String> = healthy
                .iter()
                .filter_map(|n| {
                    let sub = n.subscription_name.as_deref().unwrap_or("");
                    if allowed.is_empty() || allowed.contains(sub) {
                        Some(n.region.clone())
                    } else {
                        None
                    }
                })
                .collect();
            result.sort();
            result.dedup();
            result
        }
    }
}

/// Compute the full strategy plan: for each platform in the config,
/// which region_filters should be PATCHed.
pub fn compute_plan(
    config: &StrategyConfig,
    nodes: &[NodeSummary],
) -> HashMap<String, Vec<String>> {
    let healthy = liveness_filter(nodes);
    let mut plan = HashMap::new();
    for ps in &config.platforms {
        let regions = a_class_regions(ps, &healthy);
        plan.insert(ps.platform_name.clone(), regions);
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk_node(hash: &str, region: &str, healthy: bool, latency: Option<f64>, sub: Option<&str>) -> NodeSummary {
        NodeSummary {
            node_hash: hash.into(),
            region: region.into(),
            has_outbound: healthy,
            failure_count: if healthy { 0 } else { 3 },
            reference_latency_ms: latency,
            subscription_name: sub.map(String::from),
        }
    }

    #[test]
    fn parse_nodes_handles_items_wrapper() {
        let v = json!({
            "items": [
                {"node_hash": "h1", "region": "HK", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "sub1", "tag": "ss"}]},
                {"node_hash": "h2", "region": "US", "has_outbound": false, "failure_count": 2, "tags": [{"subscription_name": "sub2", "tag": "vmess"}]},
            ]
        });
        let nodes = parse_nodes(&v);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].region, "HK");
        assert_eq!(nodes[0].subscription_name.as_deref(), Some("sub1"));
        assert_eq!(nodes[1].has_outbound, false);
    }

    #[test]
    fn liveness_filter_excludes_failed() {
        let nodes = vec![
            mk_node("h1", "HK", true, None, None),
            mk_node("h2", "US", false, None, None),
            mk_node("h3", "JP", true, None, None),
        ];
        let healthy = liveness_filter(&nodes);
        assert_eq!(healthy.len(), 2);
        assert!(healthy.iter().all(|n| n.node_hash != "h2"));
    }

    #[test]
    fn manual_strategy_maps_manual_nodes_to_regions() {
        // T11-4a: Manual mode maps manual_nodes (hashes) to regions via healthy nodes.
        // With empty manual_nodes, result is empty (no nodes selected).
        let nodes = vec![
            mk_node("h1", "HK", true, None, None),
            mk_node("h2", "US", true, None, None),
            mk_node("h3", "JP", true, None, None),
        ];
        let healthy: Vec<&NodeSummary> = nodes.iter().collect();
        let ps = PlatformStrategy {
            platform_name: "p1".into(),
            a_class: AClassStrategy::Manual,
            b_class: StrategyId::Random,
            regions: vec![],
            subscriptions: vec![],
            top_n: 10,
            manual_nodes: vec!["h1".into(), "h3".into()],
                    b_class_params: BClassParams::default(),
        };
        let regions = a_class_regions(&ps, &healthy);
        assert_eq!(regions, vec!["HK".to_string(), "JP".to_string()]);
    }

    #[test]
    fn manual_strategy_empty_nodes_returns_empty() {
        // T11-4a: Manual mode with no manual_nodes selected returns empty vec.
        let nodes = vec![
            mk_node("h1", "HK", true, None, None),
        ];
        let healthy: Vec<&NodeSummary> = nodes.iter().collect();
        let ps = PlatformStrategy {
            platform_name: "p1".into(),
            a_class: AClassStrategy::Manual,
            b_class: StrategyId::Random,
            regions: vec!["HK".into()],
            subscriptions: vec![],
            top_n: 10,
            manual_nodes: vec![],
                    b_class_params: BClassParams::default(),
        };
        let regions = a_class_regions(&ps, &healthy);
        assert_eq!(regions, Vec::<String>::new());
    }

    #[test]
    fn region_strategy_filters_by_config_regions() {
        let nodes = vec![
            mk_node("h1", "HK", true, Some(100.0), None),
            mk_node("h2", "US", true, Some(200.0), None),
            mk_node("h3", "JP", true, Some(150.0), None),
        ];
        let healthy: Vec<&NodeSummary> = nodes.iter().collect();
        let ps = PlatformStrategy {
            platform_name: "p1".into(),
            a_class: AClassStrategy::Region,
            b_class: StrategyId::Random,
            regions: vec!["HK".into(), "JP".into()],
            subscriptions: vec![],
            top_n: 10,
                    manual_nodes: vec![],
                    b_class_params: BClassParams::default(),
        };
        let regions = a_class_regions(&ps, &healthy);
        assert!(regions.contains(&"HK".to_string()));
        assert!(regions.contains(&"JP".to_string()));
        assert!(!regions.contains(&"US".to_string()));
    }

    #[test]
    fn quality_strategy_takes_top_n_by_latency() {
        let nodes = vec![
            mk_node("h1", "HK", true, Some(100.0), None),
            mk_node("h2", "US", true, Some(300.0), None),
            mk_node("h3", "JP", true, Some(150.0), None),
            mk_node("h4", "DE", true, Some(500.0), None),
        ];
        let healthy: Vec<&NodeSummary> = nodes.iter().collect();
        let ps = PlatformStrategy {
            platform_name: "p1".into(),
            a_class: AClassStrategy::Quality,
            b_class: StrategyId::Latency,
            regions: vec![],
            subscriptions: vec![],
            top_n: 2,
                    manual_nodes: vec![],
                    b_class_params: BClassParams::default(),
        };
        let regions = a_class_regions(&ps, &healthy);
        // Top 2 by latency: HK (100ms) + JP (150ms)
        assert!(regions.contains(&"HK".to_string()));
        assert!(regions.contains(&"JP".to_string()));
        assert!(!regions.contains(&"US".to_string()));
    }

    #[test]
    fn subscription_strategy_filters_by_sub_name() {
        let nodes = vec![
            mk_node("h1", "HK", true, Some(100.0), Some("alpha")),
            mk_node("h2", "US", true, Some(200.0), Some("beta")),
            mk_node("h3", "JP", true, Some(150.0), Some("alpha")),
        ];
        let healthy: Vec<&NodeSummary> = nodes.iter().collect();
        let ps = PlatformStrategy {
            platform_name: "p1".into(),
            a_class: AClassStrategy::Subscription,
            b_class: StrategyId::Random,
            regions: vec![],
            subscriptions: vec!["alpha".into()],
            top_n: 10,
                    manual_nodes: vec![],
                    b_class_params: BClassParams::default(),
        };
        let regions = a_class_regions(&ps, &healthy);
        assert!(regions.contains(&"HK".to_string()));
        assert!(regions.contains(&"JP".to_string()));
        assert!(!regions.contains(&"US".to_string()));
    }

    #[test]
    fn compute_plan_produces_per_platform_regions() {
        let nodes = vec![
            mk_node("h1", "HK", true, Some(100.0), Some("sub1")),
            mk_node("h2", "US", true, Some(200.0), Some("sub2")),
        ];
        let config = StrategyConfig {
            version: 1,
            acknowledged: vec![],
            generation: 0,
            applied_generation: 0,
            last_apply_at: None,
            last_apply_error: None,
            updated_at: None,
            platforms: vec![PlatformStrategy {
                platform_name: "p1".into(),
                a_class: AClassStrategy::Region,
                b_class: StrategyId::Random,
                regions: vec!["HK".into()],
                subscriptions: vec![],
                top_n: 10,
                        manual_nodes: vec![],
                    b_class_params: BClassParams::default(),
        }],
        };
        let plan = compute_plan(&config, &nodes);
        assert_eq!(plan.get("p1").unwrap(), &vec!["HK".to_string()]);
    }

    #[test]
    fn a_class_strategy_parse_roundtrips() {
        for s in [AClassStrategy::Manual, AClassStrategy::Region, AClassStrategy::Quality, AClassStrategy::Subscription] {
            assert_eq!(AClassStrategy::parse(s.as_str()).unwrap(), s);
        }
        assert!(AClassStrategy::parse("unknown").is_err());
    }

    #[test]
    fn parse_nodes_handles_empty_items() {
        let v = json!({"items": []});
        assert_eq!(parse_nodes(&v).len(), 0);
        assert_eq!(parse_nodes(&json!([])).len(), 0);
        assert_eq!(parse_nodes(&json!({})).len(), 0);
    }


    #[test]
    fn b_class_params_serde_roundtrip() {
        use serde_json::json;
        let ps = PlatformStrategy {
            platform_name: "p1".into(),
            a_class: AClassStrategy::Manual,
            b_class: crate::strategy::StrategyId::Sequential,
            manual_nodes: vec![],
            regions: vec![],
            subscriptions: vec![],
            top_n: 10,
            b_class_params: BClassParams {
                round_robin_n: Some(5),
                latency_threshold_ms: Some(200),
                quality_score: None,
                bandwidth_weight: Some(2),
            },
        };
        let v = serde_json::to_value(&ps).unwrap();
        assert_eq!(v["b_class_params"]["round_robin_n"], json!(5));
        assert_eq!(v["b_class_params"]["latency_threshold_ms"], json!(200));
        assert!(v["b_class_params"]["quality_score"].is_null());
        assert_eq!(v["b_class_params"]["bandwidth_weight"], json!(2));
        // Round-trip back
        let back: PlatformStrategy = serde_json::from_value(v).unwrap();
        assert_eq!(back.b_class_params.round_robin_n, Some(5));
        assert_eq!(back.b_class_params.bandwidth_weight, Some(2));
    }

    #[test]
    fn b_class_params_default_all_none() {
        let p = BClassParams::default();
        assert!(p.round_robin_n.is_none());
        assert!(p.latency_threshold_ms.is_none());
        assert!(p.quality_score.is_none());
        assert!(p.bandwidth_weight.is_none());
    }

    #[test]
    fn b_class_params_omitted_field_deserializes_to_none() {
        use serde_json::json;
        // Config written before T18-3 has no b_class_params field — must default.
        let raw = json!({
            "platform_name": "p1",
            "a_class": "manual",
            "b_class": "random",
            "manual_nodes": [],
            "regions": [],
            "subscriptions": [],
            "top_n": 10
        });
        let ps: PlatformStrategy = serde_json::from_value(raw).unwrap();
        assert_eq!(ps.b_class_params, BClassParams::default());
    }

}
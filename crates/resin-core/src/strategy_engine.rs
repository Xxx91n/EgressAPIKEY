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
}

fn default_quality_top_n() -> usize {
    10
}

/// The whitebox strategy config document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StrategyConfig {
    pub version: u8,
    #[serde(default)]
    pub platforms: Vec<PlatformStrategy>,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            version: 1,
            platforms: vec![],
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
        AClassStrategy::Manual => strategy.regions.clone(),
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
    fn manual_strategy_returns_config_regions() {
        let ps = PlatformStrategy {
            platform_name: "p1".into(),
            a_class: AClassStrategy::Manual,
            b_class: StrategyId::Random,
            regions: vec!["HK".into(), "JP".into()],
            subscriptions: vec![],
            top_n: 10,
                    manual_nodes: vec![],
        };
        let healthy: Vec<&NodeSummary> = vec![];
        let regions = a_class_regions(&ps, &healthy);
        assert_eq!(regions, vec!["HK", "JP"]);
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
            platforms: vec![PlatformStrategy {
                platform_name: "p1".into(),
                a_class: AClassStrategy::Region,
                b_class: StrategyId::Random,
                regions: vec!["HK".into()],
                subscriptions: vec![],
                top_n: 10,
                        manual_nodes: vec![],
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

}
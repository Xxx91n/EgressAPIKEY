//! Authoritative effective-config snapshot (architecture-recovery ticket 07).
//!
//! The read-back answer to "is my configuration actually in effect"
//! (CONTEXT.md: Authoritative Snapshot; ARCHITECTURE.md §Config Authority):
//! merge the L2 whitebox strategy config against the L3 Resin runtime and
//! report per platform whether the two layers AGREE, DISAGREE (both values
//! surfaced, never silently reconciled) or one side is MISSING. This module
//! is the ONLY sanctioned cross-store merge point; view layers consume the
//! snapshot and must not re-merge stores themselves.
//!
//! Pure data + pure functions: no Tauri, no I/O, fully unit-testable against
//! the three legislated fixtures (consistent / divergent / missing).
//!
//! Ticket 10 will move the strategy read/apply pipeline into a
//! StrategyService; this module keeps the merge in resin-core so that move
//! stays a pure relocation (AGENTS.md ADR-0036 write entry unchanged).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::strategy::StrategyId;
use crate::strategy_engine::StrategyConfig;

/// Per-platform agreement between the L2 whitebox strategy config and the
/// L3 Resin runtime platform row. Serde is camelCase so the TS discriminated
/// union narrows on `state` without string parsing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum StrategySnapshot {
    /// Whitebox and Resin runtime agree.
    Consistent {
        platform_name: String,
        /// Resin runtime row id (lease chips resolve platform_id -> name).
        platform_id: String,
        /// region_filters computed from the whitebox A-class plan.
        regions: Vec<String>,
        /// Resin allocation_policy observed on the runtime platform row.
        resin_allocation_policy: String,
        /// Shell B-class strategy id that maps onto that policy.
        b_class: String,
        /// A-class mode from the whitebox config.
        a_class: String,
        /// Whitebox manual_nodes (ADR-0039 SS2: ALL strategy fields merged).
        manual_nodes: Vec<String>,
        /// Whitebox subscription names (ADR-0039 SS2).
        subscriptions: Vec<String>,
    },
    /// Both sides readable but disagree (apply failed or was overridden).
    /// Both values are carried; the snapshot NEVER picks a winner.
    Divergent {
        platform_name: String,
        /// Resin runtime row id; empty when the platform is runtime-only.
        platform_id: String,
        /// Whitebox (L2 intent) regions.
        whitebox_regions: Vec<String>,
        /// Resin runtime (L3 effective) region_filters.
        resin_regions: Vec<String>,
        /// Resin allocation_policy observed on the runtime row.
        resin_allocation_policy: String,
        /// Shell B-class strategy id from the whitebox config.
        b_class: String,
        /// A-class mode from the whitebox config.
        a_class: String,
        /// Whitebox manual_nodes (ADR-0039 SS2).
        manual_nodes: Vec<String>,
        /// Whitebox subscription names (ADR-0039 SS2).
        subscriptions: Vec<String>,
    },
    /// The platform exists in the whitebox but has no Resin runtime row
    /// (create failed, sidecar restarted without restore, manual delete).
    MissingOnResin {
        platform_name: String,
        /// Empty: no Resin row exists for this platform.
        platform_id: String,
        regions: Vec<String>,
        a_class: String,
        b_class: String,
        /// Whitebox manual_nodes (ADR-0039 SS2).
        manual_nodes: Vec<String>,
        /// Whitebox subscription names (ADR-0039 SS2).
        subscriptions: Vec<String>,
    },
}

impl StrategySnapshot {
    pub fn platform_name(&self) -> &str {
        match self {
            Self::Consistent { platform_name, .. }
            | Self::Divergent { platform_name, .. }
            | Self::MissingOnResin { platform_name, .. } => platform_name,
        }
    }

    /// Stable machine-readable state tag for logs and tests.
    pub fn state_tag(&self) -> &'static str {
        match self {
            Self::Consistent { .. } => "consistent",
            Self::Divergent { .. } => "divergent",
            Self::MissingOnResin { .. } => "missing_on_resin",
        }
    }
}

/// Per-entry-port agreement between the L2 whitebox ports config
/// (`egressapikey-ports.json` + `egressapikey.db`) and the L3 Resin
/// runtime listener table (`GET /api/v1/endpoints`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum PortSnapshot {
    /// The port exists in whitebox + DB and Resin has a listener for it.
    Consistent {
        port: u16,
        platform_name: String,
        protocol: String,
        account: String,
        label: String,
        enabled: bool,
        auth_required: bool,
    },
    /// The port is enabled in the whitebox but has no Resin endpoint
    /// (create failed, 409 shadowed, sidecar restarted without restore).
    /// A disabled port with no Resin endpoint is consistent-by-intent, not
    /// divergent: disabled means "no listener" is the desired state.
    MissingOnResin {
        port: u16,
        platform_name: String,
        protocol: String,
        account: String,
        label: String,
        auth_required: bool,
    },
}

impl PortSnapshot {
    pub fn port(&self) -> u16 {
        match self {
            Self::Consistent { port, .. } | Self::MissingOnResin { port, .. } => *port,
        }
    }

    pub fn state_tag(&self) -> &'static str {
        match self {
            Self::Consistent { .. } => "consistent",
            Self::MissingOnResin { .. } => "missing_on_resin",
        }
    }
}

/// The full authoritative snapshot returned by the `authoritative_snapshot`
/// IPC command in ONE call. Pre-merged in Rust; views consume it as-is.
/// Top-level fields are camelCase (TS-side convention for the wrapper);
/// per-variant fields stay snake_case, matching every other IPC payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthoritativeSnapshot {
    /// Whitebox strategy config document version (`egressapikey-strategy.json`).
    pub strategy_version: u8,
    pub platforms: Vec<StrategySnapshot>,
    pub ports: Vec<PortSnapshot>,
    /// Whether the Resin runtime was reachable when the snapshot was taken.
    /// false => Resin-sourced fields are empty and every enabled entry is
    /// reported missing_on_resin; consumers must not treat that as divergence.
    pub resin_reachable: bool,
}

/// Raw L3 runtime view of one Resin platform row: the only fields the
/// snapshot needs. Built from `GET /api/v1/platforms` items.
#[derive(Debug, Clone, PartialEq)]
pub struct ResinPlatformRuntime {
    pub id: String,
    pub name: String,
    pub region_filters: Vec<String>,
    pub allocation_policy: String,
}

/// Parse Resin /platforms items into runtime views. Accepts both the
/// `{"items":[..]}` wrapper and a bare array; silently skips malformed rows
/// (a snapshot must never fail because one row lacks a name).
pub fn parse_resin_platforms(v: &serde_json::Value) -> Vec<ResinPlatformRuntime> {
    let arr = match v.get("items").and_then(|i| i.as_array()) {
        Some(a) => a,
        None => match v.as_array() {
            Some(a) => a,
            None => return vec![],
        },
    };
    arr.iter()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?.to_string();
            let region_filters = p
                .get("region_filters")
                .and_then(|r| r.as_array())
                .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let allocation_policy = p
                .get("allocation_policy")
                .and_then(|a| a.as_str())
                .unwrap_or("BALANCED")
                .to_string();
            Some(ResinPlatformRuntime {
                id: p.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
                name,
                region_filters,
                allocation_policy,
            })
        })
        .collect()
}

/// The three-state merge. This is the sanctioned merge point named by
/// ARCHITECTURE.md §Config Authority.
///
/// Semantics:
/// - Whitebox platform absent from Resin => missing_on_resin (but only
///   meaningful when resin_reachable; the caller decides what to emit when
///   Resin is down — this function is pure and always runs the full merge).
/// - Resin platform absent from the whitebox => it was created outside the
///   strategy pipeline; it still appears, tagged divergent with the whitebox
///   side empty, because the snapshot must surface ALL runtime platforms.
///   The alternative (dropping runtime-only rows) would hide drift.
/// - Comparison is on the ORDER-INSENSITIVE region set: region_filters is a
///   membership filter, so [US, HK] and [HK, US] are the same effective
///   configuration. Duplicates collapse (["US","US"] == ["US"]).
/// - Manual A-class entries compare computed regions (a_class_regions output)
///   against Resin: the whitebox intent for manual mode is the mapped region
///   set, not raw node hashes.
pub fn merge_strategies(
    config: &StrategyConfig,
    resin: &[ResinPlatformRuntime],
    computed_regions: &HashMap<String, Vec<String>>,
    whitebox_exists: bool,
) -> Vec<StrategySnapshot> {
    let mut resin_by_name: HashMap<&str, &ResinPlatformRuntime> = HashMap::new();
    for rp in resin {
        resin_by_name.insert(rp.name.as_str(), rp);
    }

    let mut out: Vec<StrategySnapshot> = Vec::with_capacity(config.platforms.len() + resin.len());
    let mut seen_resin: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ps in &config.platforms {
        let computed = computed_regions
            .get(&ps.platform_name)
            .cloned()
            .unwrap_or_else(|| ps.regions.clone());
        let b_class = ps.b_class.as_str().to_string();
        let a_class = ps.a_class.as_str().to_string();
        let manual_nodes = ps.manual_nodes.clone();
        let subscriptions = ps.subscriptions.clone();
        match resin_by_name.get(ps.platform_name.as_str()) {
            Some(rp) => {
                seen_resin.insert(ps.platform_name.clone());
                if same_region_set(&computed, &rp.region_filters) {
                    out.push(StrategySnapshot::Consistent {
                        platform_name: ps.platform_name.clone(),
                        platform_id: rp.id.clone(),
                        regions: normalized(computed),
                        resin_allocation_policy: rp.allocation_policy.clone(),
                        b_class: b_class.clone(),
                        a_class: a_class.clone(),
                        manual_nodes: manual_nodes.clone(),
                        subscriptions: subscriptions.clone(),
                    });
                } else {
                    out.push(StrategySnapshot::Divergent {
                        platform_name: ps.platform_name.clone(),
                        platform_id: rp.id.clone(),
                        whitebox_regions: normalized(computed),
                        resin_regions: normalized(rp.region_filters.clone()),
                        resin_allocation_policy: rp.allocation_policy.clone(),
                        b_class,
                        a_class,
                        manual_nodes,
                        subscriptions,
                    });
                }
            }
            None => {
                out.push(StrategySnapshot::MissingOnResin {
                    platform_name: ps.platform_name.clone(),
                    platform_id: String::new(),
                    regions: normalized(computed),
                    a_class,
                    b_class,
                    manual_nodes,
                    subscriptions,
                });
            }
        }
    }

    // Runtime-only platforms: created outside the strategy pipeline (GUI
    // platform_add, Resin API direct). Surface them as divergent with an
    // empty whitebox side so "apply missing" drift is visible, never dropped.
    // T22-4 display contract: while the whitebox file exists, such platforms
    // render with the subscription A-class default (selects all nodes); with
    // no whitebox file at all there is no strategy intent to default to.
    let runtime_only_a_class = if whitebox_exists { "subscription" } else { "" };
    for rp in resin {
        if !seen_resin.contains(&rp.name) {
            out.push(StrategySnapshot::Divergent {
                platform_name: rp.name.clone(),
                platform_id: rp.id.clone(),
                whitebox_regions: vec![],
                resin_regions: normalized(rp.region_filters.clone()),
                resin_allocation_policy: rp.allocation_policy.clone(),
                b_class: String::new(),
                a_class: runtime_only_a_class.to_string(),
                manual_nodes: vec![],
                subscriptions: vec![],
            });
        }
    }

    out
}

fn normalized(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

/// Region codes are case-insensitive labels (nodes report mixed case; the
/// canvas groups them lowercased), so membership comparison normalizes case.
fn same_region_set(a: &[String], b: &[String]) -> bool {
    let lower = |v: &[String]| -> Vec<String> { normalized(v.to_vec()).iter().map(|s| s.to_lowercase()).collect() };
    lower(a) == lower(b)
}

/// Merge the port triple (whitebox entry_ports + Resin endpoints list).
/// `resin_ports` is the set of ports Resin currently listens on, extracted
/// from `GET /api/v1/endpoints` by the caller (all endpoint kinds — the
/// shell does not distinguish default/custom here; a listener exists or not).
/// Disabled whitebox ports with no Resin endpoint are consistent by intent.
pub fn merge_ports(
    whitebox: &[crate::PortMapping],
    resin_ports: &[u16],
) -> Vec<PortSnapshot> {
    let resin_set: std::collections::HashSet<u16> = resin_ports.iter().copied().collect();
    let mut out = Vec::with_capacity(whitebox.len());
    for m in whitebox {
        if resin_set.contains(&m.port) {
            out.push(PortSnapshot::Consistent {
                port: m.port,
                platform_name: m.platform_name.clone(),
                protocol: m.protocol.clone(),
                account: m.account.clone(),
                label: m.label.clone(),
                enabled: m.enabled,
                auth_required: m.auth_required,
            });
        } else if m.enabled {
            out.push(PortSnapshot::MissingOnResin {
                port: m.port,
                platform_name: m.platform_name.clone(),
                protocol: m.protocol.clone(),
                account: m.account.clone(),
                label: m.label.clone(),
                auth_required: m.auth_required,
            });
        } else {
            out.push(PortSnapshot::Consistent {
                port: m.port,
                platform_name: m.platform_name.clone(),
                protocol: m.protocol.clone(),
                account: m.account.clone(),
                label: m.label.clone(),
                enabled: false,
                auth_required: m.auth_required,
            });
        }
    }
    out.sort_by_key(|p| p.port());
    out
}

/// Convenience: the B-class id that a whitebox entry maps onto in Resin terms.
/// Exposed for the command layer so the snapshot's b_class field always uses
/// the shell catalog vocabulary (ADR-0039 SS2 B badge contract).
pub fn b_class_of(ps: &crate::strategy_engine::PlatformStrategy) -> String {
    let _ = StrategyId::Random; // keep the import honest if the catalog moves
    ps.b_class.as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy_engine::{AClassStrategy, PlatformStrategy};
    use serde_json::json;

    fn strategy_entry(name: &str, regions: &[&str]) -> PlatformStrategy {
        PlatformStrategy {
            platform_name: name.into(),
            a_class: AClassStrategy::Region,
            b_class: StrategyId::Random,
            manual_nodes: vec![],
            regions: regions.iter().map(|s| s.to_string()).collect(),
            subscriptions: vec![],
            top_n: 10,
            b_class_params: Default::default(),
        }
    }

    fn runtime(name: &str, regions: &[&str], policy: &str) -> ResinPlatformRuntime {
        ResinPlatformRuntime {
            id: format!("id-{name}"),
            name: name.into(),
            region_filters: regions.iter().map(|s| s.to_string()).collect(),
            allocation_policy: policy.into(),
        }
    }

    // Fixture 1: whitebox and Resin agree.
    #[test]
    fn fixture_consistent_reports_agreement() {
        let cfg = StrategyConfig {
            version: 1,
            platforms: vec![strategy_entry("alpha", &["us", "hk"])],
        };
        let resin = vec![runtime("alpha", &["hk", "us"], "BALANCED")];
        let map = HashMap::new();
        let snap = merge_strategies(&cfg, &resin, &map, true);
        assert_eq!(snap.len(), 1);
        match &snap[0] {
            StrategySnapshot::Consistent {
                platform_name,
                platform_id,
                regions,
                resin_allocation_policy,
                b_class,
                a_class,
                manual_nodes,
                subscriptions,
            } => {
                assert_eq!(platform_name, "alpha");
                assert_eq!(platform_id, "id-alpha");
                assert_eq!(regions, &["hk".to_string(), "us".to_string()]);
                assert_eq!(resin_allocation_policy, "BALANCED");
                assert_eq!(b_class, "random");
                assert_eq!(a_class, "region");
                assert!(manual_nodes.is_empty());
                assert!(subscriptions.is_empty());
            }
            other => panic!("expected consistent, got {other:?}"),
        }
        assert_eq!(snap[0].state_tag(), "consistent");
    }

    // Fixture 2: both sides readable but disagree — both values surfaced.
    #[test]
    fn fixture_divergent_surfaces_both_sides() {
        let cfg = StrategyConfig {
            version: 1,
            platforms: vec![strategy_entry("alpha", &["jp"])],
        };
        let resin = vec![runtime("alpha", &["us"], "PREFER_LOW_LATENCY")];
        let snap = merge_strategies(&cfg, &resin, &HashMap::new(), true);
        match &snap[0] {
            StrategySnapshot::Divergent {
                whitebox_regions,
                resin_regions,
                resin_allocation_policy,
                ..
            } => {
                assert_eq!(whitebox_regions, &["jp".to_string()]);
                assert_eq!(resin_regions, &["us".to_string()]);
                assert_eq!(resin_allocation_policy, "PREFER_LOW_LATENCY");
            }
            other => panic!("expected divergent, got {other:?}"),
        }
        assert_eq!(snap[0].state_tag(), "divergent");
    }

    // Fixture 3: platform exists in whitebox but not in Resin.
    #[test]
    fn fixture_missing_on_resin_reported() {
        let cfg = StrategyConfig {
            version: 1,
            platforms: vec![strategy_entry("ghost", &["hk"])],
        };
        let resin = vec![runtime("alpha", &["hk"], "BALANCED")];
        let snap = merge_strategies(&cfg, &resin, &HashMap::new(), true);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].state_tag(), "missing_on_resin");
        assert_eq!(snap[0].platform_name(), "ghost");
        // Runtime-only platform still surfaces (divergent, empty whitebox).
        assert_eq!(snap[1].state_tag(), "divergent");
        assert_eq!(snap[1].platform_name(), "alpha");
    }

    #[test]
    fn region_comparison_is_order_and_duplicate_insensitive() {
        let cfg = StrategyConfig {
            version: 1,
            platforms: vec![strategy_entry("alpha", &["us", "hk", "us"])],
        };
        let resin = vec![runtime("alpha", &["HK", "US"], "BALANCED")];
        let snap = merge_strategies(&cfg, &resin, &HashMap::new(), true);
        assert_eq!(snap[0].state_tag(), "consistent");
    }

    #[test]
    fn empty_whitebox_plus_empty_resin_is_empty_snapshot() {
        let cfg = StrategyConfig::default();
        let snap = merge_strategies(&cfg, &[], &HashMap::new(), true);
        assert!(snap.is_empty());
    }

    #[test]
    fn computed_regions_override_raw_regions_for_manual_mode() {
        // compute_plan output (manual_nodes -> regions) is the whitebox intent.
        let cfg = StrategyConfig {
            version: 1,
            platforms: vec![strategy_entry("alpha", &[])],
        };
        let mut computed = HashMap::new();
        computed.insert("alpha".to_string(), vec!["sg".to_string()]);
        let resin = vec![runtime("alpha", &["sg"], "BALANCED")];
        let snap = merge_strategies(&cfg, &resin, &computed, true);
        assert_eq!(snap[0].state_tag(), "consistent");
    }

    #[test]
    fn parse_resin_platforms_accepts_wrapper_and_bare_array() {
        let wrapper = json!({"items": [
            {"name": "alpha", "region_filters": ["hk"], "allocation_policy": "BALANCED"},
            {"name": "beta", "region_filters": null}
        ]});
        let parsed = parse_resin_platforms(&wrapper);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].region_filters, vec!["hk"]);
        assert_eq!(parsed[1].allocation_policy, "BALANCED");
        let bare = json!([{"name": "gamma", "region_filters": ["us"]}]);
        assert_eq!(parse_resin_platforms(&bare).len(), 1);
        assert!(parse_resin_platforms(&json!(null)).is_empty());
        // Malformed row is skipped, not fatal.
        let mixed = json!({"items": [{"region_filters": ["hk"]}, {"name": "ok"}]});
        assert_eq!(parse_resin_platforms(&mixed).len(), 1);
    }

    fn port_mapping(port: u16, enabled: bool) -> crate::PortMapping {
        crate::PortMapping {
            port,
            protocol: "socks5".into(),
            platform_name: "alpha".into(),
            account: format!("port-{port}"),
            label: String::new(),
            enabled,
            auth_required: false,
        }
    }

    #[test]
    fn merge_ports_enabled_missing_disabled_consistent() {
        let whitebox = vec![port_mapping(17990, true), port_mapping(17991, false)];
        // 17990 has a Resin listener, 17991 does not.
        let snap = merge_ports(&whitebox, &[17990]);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].state_tag(), "consistent");
        assert_eq!(snap[0].port(), 17990);
        // Disabled + no Resin endpoint = consistent by intent.
        assert_eq!(snap[1].state_tag(), "consistent");
        assert_eq!(snap[1].port(), 17991);
    }

    #[test]
    fn merge_ports_reports_enabled_port_missing_on_resin() {
        let whitebox = vec![port_mapping(17990, true)];
        let snap = merge_ports(&whitebox, &[]);
        assert_eq!(snap[0].state_tag(), "missing_on_resin");
    }

    #[test]
    fn snapshot_serializes_discriminated_states_for_ts() {
        let cfg = StrategyConfig {
            version: 1,
            platforms: vec![strategy_entry("alpha", &["hk"]), strategy_entry("beta", &["us"])],
        };
        let resin = vec![runtime("alpha", &["hk"], "BALANCED")];
        let mut snap = AuthoritativeSnapshot {
            strategy_version: cfg.version,
            platforms: merge_strategies(&cfg, &resin, &HashMap::new(), true),
            ports: merge_ports(&[], &[]),
            resin_reachable: true,
        };
        snap.ports = merge_ports(&[port_mapping(17990, true)], &[17990]);
        let v = serde_json::to_value(&snap).expect("serialize");
        assert_eq!(v["platforms"][0]["state"], "consistent");
        assert_eq!(v["platforms"][1]["state"], "missingOnResin");
        assert_eq!(v["ports"][0]["state"], "consistent");
        assert_eq!(v["strategyVersion"], 1);
        assert_eq!(v["resinReachable"], true);
        // Round-trip stays equal (contract stability for the TS wrapper).
        let back: AuthoritativeSnapshot = serde_json::from_value(v).expect("deserialize");
        assert_eq!(back, snap);
    }

    #[test]
    fn b_class_uses_shell_catalog_vocabulary() {
        let ps = strategy_entry("alpha", &["hk"]);
        assert_eq!(b_class_of(&ps), "random");
    }
}

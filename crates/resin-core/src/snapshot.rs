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
        /// Read-side exemption flag from the strategy whitebox `acknowledged`
        /// array (ticket 12 / ADR-0054 §D). NEVER influences the merge; set
        /// after merging via stamp_platform_acknowledged.
        #[serde(default)]
        acknowledged: bool,
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
        /// Unix seconds when this platform FIRST entered divergent within the
        /// current process; None while consistent (ticket 12 / ADR-0054 §C).
        /// In-process memory only: cleared when the state returns to
        /// consistent, and reset on process restart.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        divergent_since: Option<u64>,
        /// Read-side exemption flag from the strategy whitebox `acknowledged`
        /// array (ticket 12 / ADR-0054 §D). NEVER influences the merge.
        #[serde(default)]
        acknowledged: bool,
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
        /// Unix seconds when this platform FIRST entered missing_on_resin
        /// within the current process (ticket 12 / ADR-0054 §C). In-process
        /// memory only; re-times after restart or a consistent spell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        divergent_since: Option<u64>,
        /// Read-side exemption flag from the strategy whitebox `acknowledged`
        /// array (ticket 12 / ADR-0054 §D). NEVER influences the merge.
        #[serde(default)]
        acknowledged: bool,
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

    /// Read-side exemption flag (ticket 12 / ADR-0054 §D). The notify-once
    /// rule (ADR-0054 §E, ticket 16) and the view's grey "known" degradation
    /// both consume this; the three-state merge itself never reads it.
    pub fn acknowledged(&self) -> bool {
        match self {
            Self::Consistent { acknowledged, .. }
            | Self::Divergent { acknowledged, .. }
            | Self::MissingOnResin { acknowledged, .. } => *acknowledged,
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
        /// Read-side exemption flag from the ports whitebox `acknowledged`
        /// array (ticket 12 / ADR-0054 §D). NEVER influences the merge; set
        /// after merging via stamp_port_acknowledged.
        #[serde(default)]
        acknowledged: bool,
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
        /// Unix seconds when this port FIRST entered missing_on_resin within
        /// the current process (ticket 12 / ADR-0054 §C). In-process memory
        /// only; re-times after restart or a consistent spell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        divergent_since: Option<u64>,
        /// Read-side exemption flag from the ports whitebox `acknowledged`
        /// array (ticket 12 / ADR-0054 §D). NEVER influences the merge.
        #[serde(default)]
        acknowledged: bool,
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

    /// Read-side exemption flag (ticket 12 / ADR-0054 §D); see the platform
    /// accessor for the consumers.
    pub fn acknowledged(&self) -> bool {
        match self {
            Self::Consistent { acknowledged, .. }
            | Self::MissingOnResin { acknowledged, .. } => *acknowledged,
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
    /// Ticket 17 / ADR-0055 D3: per-route three-state (whitebox
    /// process_routes vs the Resin process-group echo). Empty when the
    /// whitebox defines no routes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<ProcessRouteSnapshot>,
    /// Unix seconds when THIS snapshot was generated (ticket 12 / ADR-0054 §C).
    /// Pure metadata: stamped by the command layer, never participates in the
    /// three-state merge. Monotonic non-decreasing across consecutive calls.
    pub last_checked_at: u64,
}

/// Process-local first-drift memory for `divergentSince` (ticket 12 /
/// ADR-0054 §C). Keyed by a stable entity id (platform name / port number
/// string); the value is the Unix-second instant the entity FIRST entered a
/// drift state (divergent or missingOnResin). Semantics (issue 12):
/// - first observation of a drifting entity => record `now`;
/// - entity still drifting on later snapshots => keep the original instant;
/// - entity back to consistent => the entry is removed (re-drift re-times);
/// - process restart => the map is empty (in-memory only, never persisted),
///   so the next drift observation re-times from zero.
pub type DriftMemory = HashMap<String, u64>;

/// Pure advance of the drift memory for one snapshot's entries. `entries`
/// carries (entity_key, is_drifting). Returns the new memory; the input is
/// not mutated, so callers can diff or test without fixtures aliasing.
pub fn advance_drift_memory(
    prev: &DriftMemory,
    entries: &[(String, bool)],
    now: u64,
) -> DriftMemory {
    let mut next = prev.clone();
    for (key, drifting) in entries {
        if *drifting {
            next.entry(key.clone()).or_insert(now);
        } else {
            next.remove(key);
        }
    }
    next
}

/// Resolve the `divergent_since` value for one entity from a drift memory.
/// Drifting entities carry Some(first_drift_instant); consistent entities
/// always carry None (the field never leaks a stale instant).
pub fn divergent_since_for(memory: &DriftMemory, key: &str) -> Option<u64> {
    memory.get(key).copied()
}

/// Deciding whether an entity is acknowledged is a read-side presentation
/// concern: `acknowledged` NEVER participates in the three-state merge
/// (ticket 12 / ADR-0054 §D). This helper keeps that rule in one place: the
/// merge functions below must not call it — callers stamp the flag AFTER
/// the merge, on the merged output only.
pub fn is_acknowledged(acknowledged: &[String], entity_key: &str) -> bool {
    acknowledged.iter().any(|a| a == entity_key)
}

/// Stamp `acknowledged` onto merged platform entries (post-merge, read-side
/// only). Entity key = platform_name. The three-state tag is untouched.
pub fn stamp_platform_acknowledged(platforms: &mut [StrategySnapshot], acknowledged: &[String]) {
    if acknowledged.is_empty() {
        return;
    }
    for p in platforms.iter_mut() {
        let key = p.platform_name().to_string();
        let flag = is_acknowledged(acknowledged, &key);
        match p {
            StrategySnapshot::Consistent { acknowledged: a, .. }
            | StrategySnapshot::Divergent { acknowledged: a, .. }
            | StrategySnapshot::MissingOnResin { acknowledged: a, .. } => *a = flag,
        }
    }
}

/// Stamp `acknowledged` onto merged port entries (post-merge, read-side
/// only). Entity key = decimal port number, matching the ports whitebox
/// `acknowledged` vocabulary.
pub fn stamp_port_acknowledged(ports: &mut [PortSnapshot], acknowledged: &[String]) {
    if acknowledged.is_empty() {
        return;
    }
    for p in ports.iter_mut() {
        let key = p.port().to_string();
        let flag = is_acknowledged(acknowledged, &key);
        match p {
            PortSnapshot::Consistent { acknowledged: a, .. }
            | PortSnapshot::MissingOnResin { acknowledged: a, .. } => *a = flag,
        }
    }
}

/// Ticket 17 / ADR-0055 D3: per-route agreement between the L2 whitebox
/// `process_routes` family and the L3 Resin process-group echo. The live
/// side is the set of process names Resin currently routes (its own
/// process-group registry, or the endpoints echo when the registry route
/// is absent — the caller normalizes both into a name set).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ProcessRouteSnapshot {
    /// The rule exists in the whitebox and Resin reports the process group.
    Consistent {
        process: String,
        target_port: u16,
        /// Read-side exemption flag from the ports whitebox
        /// `route_acknowledged` array (ADR-0055 D6). NEVER influences the
        /// merge; stamped after merging.
        #[serde(default)]
        acknowledged: bool,
    },
    /// The rule exists in the whitebox but Resin does not report the
    /// process group (registry absent, group deleted, sidecar restarted
    /// without a route restore).
    MissingOnResin {
        process: String,
        target_port: u16,
        /// Unix seconds when this route FIRST entered missing_on_resin
        /// within the current process. In-process memory only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        divergent_since: Option<u64>,
        /// Read-side exemption flag (ADR-0055 D6).
        #[serde(default)]
        acknowledged: bool,
    },
}

impl ProcessRouteSnapshot {
    pub fn process(&self) -> &str {
        match self {
            Self::Consistent { process, .. } | Self::MissingOnResin { process, .. } => process,
        }
    }

    pub fn state_tag(&self) -> &'static str {
        match self {
            Self::Consistent { .. } => "consistent",
            Self::MissingOnResin { .. } => "missing_on_resin",
        }
    }

    pub fn acknowledged(&self) -> bool {
        match self {
            Self::Consistent { acknowledged, .. }
            | Self::MissingOnResin { acknowledged, .. } => *acknowledged,
        }
    }
}

/// Ticket 17 / ADR-0055 D3: merge the whitebox route family. Resin has no
/// per-process object, so the L3 side of a route IS its target port: a
/// route is consistent when its port has a live Resin listener,
/// missing_on_resin when the port is enabled-but-listenerless, and
/// consistent when the port is disabled or absent (inert-by-intent,
/// mirroring the ports family's disabled-port rule). All inputs are data
/// the snapshot pass already holds; this function is pure.
pub fn merge_routes(
    whitebox: &[crate::whitebox_config::ProcessRouteRule],
    desired_enabled_ports: &[u16],
    live_ports: &[u16],
) -> Vec<ProcessRouteSnapshot> {
    let live: std::collections::HashSet<u16> = live_ports.iter().copied().collect();
    let enabled: std::collections::HashSet<u16> = desired_enabled_ports.iter().copied().collect();
    let mut out: Vec<ProcessRouteSnapshot> = whitebox
        .iter()
        .map(|r| {
            if live.contains(&r.target_port) || !enabled.contains(&r.target_port) {
                ProcessRouteSnapshot::Consistent {
                    process: r.process.clone(),
                    target_port: r.target_port,
                    acknowledged: false,
                }
            } else {
                ProcessRouteSnapshot::MissingOnResin {
                    process: r.process.clone(),
                    target_port: r.target_port,
                    divergent_since: None,
                    acknowledged: false,
                }
            }
        })
        .collect();
    out.sort_by(|a, b| a.process().to_lowercase().cmp(&b.process().to_lowercase()));
    out
}

/// Stamp `acknowledged` onto merged route entries (post-merge, read-side
/// only). Entity key = process name (case-insensitive match against the
/// `route_acknowledged` vocabulary). The three-state tag is untouched.
pub fn stamp_route_acknowledged(routes: &mut [ProcessRouteSnapshot], acknowledged: &[String]) {
    if acknowledged.is_empty() {
        return;
    }
    let vocab: std::collections::HashSet<String> =
        acknowledged.iter().map(|a| a.trim().to_lowercase()).collect();
    for r in routes.iter_mut() {
        let flag = vocab.contains(&r.process().trim().to_lowercase());
        match r {
            ProcessRouteSnapshot::Consistent { acknowledged: a, .. }
            | ProcessRouteSnapshot::MissingOnResin { acknowledged: a, .. } => *a = flag,
        }
    }
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
                        acknowledged: false,
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
                        divergent_since: None,
                        acknowledged: false,
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
                    divergent_since: None,
                    acknowledged: false,
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
                divergent_since: None,
                acknowledged: false,
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
                acknowledged: false,
            });
        } else if m.enabled {
            out.push(PortSnapshot::MissingOnResin {
                port: m.port,
                platform_name: m.platform_name.clone(),
                protocol: m.protocol.clone(),
                account: m.account.clone(),
                label: m.label.clone(),
                auth_required: m.auth_required,
                divergent_since: None,
                acknowledged: false,
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
                acknowledged: false,
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
    ps.b_class.as_str().to_string()
}

#[cfg(test)]
mod tests {

    /// Ticket 16 / ADR-0054 §E: the acknowledged accessor reads the stamped
    /// read-side flag on every variant (the once-per-process notify rule
    /// filters on it; the three-state merge must stay untouched).
    #[test]
    fn acknowledged_accessor_reads_stamped_flag_all_variants() {
        use crate::snapshot::StrategySnapshot;
        let div = StrategySnapshot::Divergent {
            platform_name: "p".into(),
            platform_id: "id".into(),
            whitebox_regions: vec!["us".into()],
            resin_regions: vec!["hk".into()],
            resin_allocation_policy: "BALANCED".into(),
            b_class: "random".into(),
            a_class: "region".into(),
            manual_nodes: vec![],
            subscriptions: vec![],
            divergent_since: None,
            acknowledged: true,
        };
        assert!(div.acknowledged());
        let mis = StrategySnapshot::MissingOnResin {
            platform_name: "q".into(),
            platform_id: String::new(),
            regions: vec![],
            a_class: "region".into(),
            b_class: "random".into(),
            manual_nodes: vec![],
            subscriptions: vec![],
            divergent_since: None,
            acknowledged: false,
        };
        assert!(!mis.acknowledged());
        let con = StrategySnapshot::Consistent {
            platform_name: "r".into(),
            platform_id: "id".into(),
            regions: vec!["us".into()],
            resin_allocation_policy: "BALANCED".into(),
            b_class: "random".into(),
            a_class: "region".into(),
            manual_nodes: vec![],
            subscriptions: vec![],
            acknowledged: true,
        };
        assert!(con.acknowledged());

        use crate::snapshot::PortSnapshot;
        let pmis = PortSnapshot::MissingOnResin {
            port: 17990,
            platform_name: "p".into(),
            protocol: "socks5".into(),
            account: "a".into(),
            label: String::new(),
            auth_required: false,
            divergent_since: None,
            acknowledged: true,
        };
        assert!(pmis.acknowledged());
        let pcon = PortSnapshot::Consistent {
            port: 17991,
            platform_name: "p".into(),
            protocol: "socks5".into(),
            account: "a".into(),
            label: String::new(),
            enabled: true,
            auth_required: false,
            acknowledged: false,
        };
        assert!(!pcon.acknowledged());
    }

    use super::*;
    use crate::strategy::StrategyId;
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
            acknowledged: vec![],
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
                acknowledged,
                ..
            } => {
                assert_eq!(platform_name, "alpha");
                assert_eq!(platform_id, "id-alpha");
                assert_eq!(regions, &["hk".to_string(), "us".to_string()]);
                assert_eq!(resin_allocation_policy, "BALANCED");
                assert_eq!(b_class, "random");
                assert_eq!(a_class, "region");
                assert!(manual_nodes.is_empty());
                assert!(subscriptions.is_empty());
                assert!(!acknowledged, "pre-stamp entries default to unacknowledged");
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
            acknowledged: vec![],
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
            acknowledged: vec![],
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
            acknowledged: vec![],
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
            acknowledged: vec![],
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
            acknowledged: vec![],
            platforms: vec![strategy_entry("alpha", &["hk"]), strategy_entry("beta", &["us"])],
        };
        let resin = vec![runtime("alpha", &["hk"], "BALANCED")];
        let mut snap = AuthoritativeSnapshot {
            strategy_version: cfg.version,
            platforms: merge_strategies(&cfg, &resin, &HashMap::new(), true),
            ports: merge_ports(&[], &[]),
            routes: vec![],
            resin_reachable: true,
            last_checked_at: 1_700_000_000,
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

    // ---- ticket 17 / ADR-0055: routes merge ----

    fn wb_rule(process: &str, port: u16) -> crate::whitebox_config::ProcessRouteRule {
        crate::whitebox_config::ProcessRouteRule {
            process: process.to_string(),
            target_port: port,
        }
    }

    #[test]
    fn merge_routes_target_port_semantics() {
        let rules = vec![wb_rule("Live.exe", 17990), wb_rule("Drift.exe", 17991), wb_rule("Inert.exe", 17992)];
        // 17990 live; 17991 enabled-but-listenerless => missing; 17992 not enabled => inert-consistent
        let snap = merge_routes(&rules, &[17990, 17991], &[17990]);
        assert_eq!(snap.len(), 3);
        // sort is case-insensitive by name: Drift < Inert < Live
        assert_eq!(snap[0].state_tag(), "missing_on_resin");
        assert_eq!(snap[0].process(), "Drift.exe");
        // explicit per-rule assertions:
        let by_name = |name: &str| snap.iter().find(|r| r.process() == name).unwrap();
        assert_eq!(by_name("Live.exe").state_tag(), "consistent");
        assert_eq!(by_name("Drift.exe").state_tag(), "missing_on_resin");
        assert_eq!(by_name("Inert.exe").state_tag(), "consistent");
        // wire shape: tagged enum with camelCase state (v[0] = Drift.exe, missing)
        let v = serde_json::to_value(&snap).unwrap();
        assert_eq!(v[0]["state"], "missingOnResin");
        assert_eq!(v[0]["target_port"], 17991);
    }

    #[test]
    fn merge_routes_case_insensitive_order_insensitive() {
        let rules = vec![wb_rule("APP.exe", 17990)];
        let snap = merge_routes(&rules, &[17990], &[17990]);
        assert_eq!(snap[0].state_tag(), "consistent");
        // input order never leaks into output order
        let rules2 = vec![wb_rule("z.exe", 17990), wb_rule("a.exe", 17991)];
        let snap2 = merge_routes(&rules2, &[17990, 17991], &[17990, 17991]);
        assert_eq!(snap2[0].process(), "a.exe");
    }

    #[test]
    fn route_acknowledged_stamp_never_touches_state() {
        let rules = vec![wb_rule("a.exe", 17991)];
        let mut snap = merge_routes(&rules, &[17991], &[]);
        assert_eq!(snap[0].state_tag(), "missing_on_resin");
        stamp_route_acknowledged(&mut snap, &["A.EXE".to_string()]);
        assert!(snap[0].acknowledged());
        assert_eq!(snap[0].state_tag(), "missing_on_resin");
        // empty vocab leaves defaults
        let mut snap2 = merge_routes(&rules, &[17991], &[]);
        stamp_route_acknowledged(&mut snap2, &[]);
        assert!(!snap2[0].acknowledged());
    }

    #[test]
    fn routes_serializes_only_when_present() {
        let snap = AuthoritativeSnapshot {
            strategy_version: 1,
            platforms: vec![],
            ports: vec![],
            routes: vec![],
            resin_reachable: true,
            last_checked_at: 1,
        };
        let v = serde_json::to_value(&snap).unwrap();
        assert!(v.get("routes").is_none(), "empty routes must be omitted");
        let with_routes = AuthoritativeSnapshot {
            routes: vec![ProcessRouteSnapshot::Consistent {
                process: "a.exe".into(),
                target_port: 17990,
                acknowledged: false,
            }],
            ..snap
        };
        let v2 = serde_json::to_value(&with_routes).unwrap();
        assert!(v2.get("routes").is_some());
    }

    // ---- ticket 12: lastCheckedAt + divergentSince + acknowledged ----

    #[test]
    fn last_checked_at_serializes_camel_case_for_ts() {
        let snap = AuthoritativeSnapshot {
            strategy_version: 1,
            platforms: vec![],
            ports: vec![],
            routes: vec![],
            resin_reachable: false,
            last_checked_at: 1_756_521_600,
        };
        let v = serde_json::to_value(&snap).expect("serialize");
        assert_eq!(v["lastCheckedAt"], 1_756_521_600i64);
        let back: AuthoritativeSnapshot = serde_json::from_value(v).expect("deserialize");
        assert_eq!(back, snap);
    }

    #[test]
    fn acknowledged_never_changes_the_three_state_merge() {
        // The acknowledged exemption flag is stamped AFTER the merge and must
        // not move any entry between consistent/divergent/missingOnResin
        // (issue 12 acceptance: "acknowledged 不改变三态").
        let cfg = StrategyConfig {
            version: 1,
            acknowledged: vec![],
            platforms: vec![strategy_entry("alpha", &["jp"])],
        };
        let resin = vec![runtime("alpha", &["us"], "BALANCED")];
        let mut merged = merge_strategies(&cfg, &resin, &HashMap::new(), true);
        let before_tag = merged[0].state_tag();
        stamp_platform_acknowledged(&mut merged, &["alpha".to_string()]);
        assert_eq!(merged[0].state_tag(), before_tag, "stamping acknowledged must not change the state tag");
        match &merged[0] {
            StrategySnapshot::Divergent { whitebox_regions, resin_regions, acknowledged, divergent_since, .. } => {
                assert_eq!(whitebox_regions, &["jp".to_string()]);
                assert_eq!(resin_regions, &["us".to_string()]);
                assert!(acknowledged);
                assert_eq!(*divergent_since, None);
            }
            other => panic!("expected divergent, got {other:?}"),
        }
        // Ports: acknowledged stamping also leaves the port state untouched.
        let mut ports = merge_ports(&[port_mapping(17990, true)], &[]);
        assert_eq!(ports[0].state_tag(), "missing_on_resin");
        stamp_port_acknowledged(&mut ports, &["17990".to_string()]);
        assert_eq!(ports[0].state_tag(), "missing_on_resin");
        match &ports[0] {
            PortSnapshot::MissingOnResin { acknowledged, .. } => assert!(acknowledged),
            other => panic!("expected missing port, got {other:?}"),
        }
    }

    #[test]
    fn divergent_since_semantics_hold_keep_clear() {
        // HOLD: an entity that stays drifting keeps its FIRST instant.
        let mut mem = advance_drift_memory(&DriftMemory::default(), &[("alpha".into(), true), ("17990".into(), true)], 100);
        assert_eq!(divergent_since_for(&mem, "alpha"), Some(100));
        mem = advance_drift_memory(&mem, &[("alpha".into(), true), ("17990".into(), true)], 250);
        assert_eq!(divergent_since_for(&mem, "alpha"), Some(100), "still drifting: the first instant is kept");
        assert_eq!(divergent_since_for(&mem, "17990"), Some(100));

        // CLEAR: back to consistent => removed; a later re-drift re-times.
        mem = advance_drift_memory(&mem, &[("alpha".into(), false), ("17990".into(), true)], 300);
        assert_eq!(divergent_since_for(&mem, "alpha"), None, "consistent entity carries no drift instant");
        mem = advance_drift_memory(&mem, &[("alpha".into(), true)], 400);
        assert_eq!(divergent_since_for(&mem, "alpha"), Some(400), "re-drift restarts the clock");
        assert_eq!(divergent_since_for(&mem, "17990"), Some(100), "unrelated entities keep their instants");

        // RESTART: a fresh process starts from an empty memory.
        let fresh = DriftMemory::default();
        assert_eq!(divergent_since_for(&fresh, "17990"), None);
        let after_restart = advance_drift_memory(&fresh, &[("17990".into(), true)], 900);
        assert_eq!(divergent_since_for(&after_restart, "17990"), Some(900), "first observation after restart re-times");
    }

    #[test]
    fn stamp_uses_platform_name_and_decimal_port_keys() {
        let cfg = StrategyConfig {
            version: 1,
            acknowledged: vec![],
            platforms: vec![strategy_entry("alpha", &["hk"]), strategy_entry("beta", &["us"])],
        };
        let resin = vec![runtime("alpha", &["hk"], "BALANCED")];
        let mut platforms = merge_strategies(&cfg, &resin, &HashMap::new(), true);
        stamp_platform_acknowledged(&mut platforms, &["beta".to_string()]);
        for p in &platforms {
            let ack = match p {
                StrategySnapshot::Consistent { acknowledged, .. }
                | StrategySnapshot::Divergent { acknowledged, .. }
                | StrategySnapshot::MissingOnResin { acknowledged, .. } => *acknowledged,
            };
            assert_eq!(ack, p.platform_name() == "beta");
        }
        // Re-stamping recomputes every entry from scratch: a list without
        // "beta" clears its flag (the command layer always stamps the full
        // snapshot from the current whitebox array, so this is idempotent).
        stamp_platform_acknowledged(&mut platforms, &["nonexistent".to_string()]);
        for p in &platforms {
            let ack = match p {
                StrategySnapshot::Consistent { acknowledged, .. }
                | StrategySnapshot::Divergent { acknowledged, .. }
                | StrategySnapshot::MissingOnResin { acknowledged, .. } => *acknowledged,
            };
            assert!(!ack, "non-matching list clears previous stamps");
        }

        let mut ports = merge_ports(&[port_mapping(17990, true), port_mapping(17991, false)], &[17990]);
        stamp_port_acknowledged(&mut ports, &["17991".to_string()]);
        for pp in &ports {
            let ack = match pp {
                PortSnapshot::Consistent { acknowledged, .. }
                | PortSnapshot::MissingOnResin { acknowledged, .. } => *acknowledged,
            };
            assert_eq!(ack, pp.port() == 17991);
        }
    }

    #[test]
    fn divergent_since_omitted_when_none_and_round_trips_when_set() {
        // None => the key is absent from the wire payload (skip_serializing_if).
        let missing = StrategySnapshot::MissingOnResin {
            platform_name: "ghost".into(),
            platform_id: String::new(),
            regions: vec!["hk".into()],
            a_class: "region".into(),
            b_class: "random".into(),
            manual_nodes: vec![],
            subscriptions: vec![],
            divergent_since: None,
            acknowledged: false,
        };
        let v = serde_json::to_value(&missing).unwrap();
        assert!(v.get("divergentSince").is_none());
        let back: StrategySnapshot = serde_json::from_value(v).unwrap();
        assert_eq!(back, missing);
        // Some => camelCase key round-trips.
        let stamped = StrategySnapshot::MissingOnResin {
            platform_name: "ghost".into(),
            platform_id: String::new(),
            regions: vec!["hk".into()],
            a_class: "region".into(),
            b_class: "random".into(),
            manual_nodes: vec![],
            subscriptions: vec![],
            divergent_since: Some(1_756_521_600),
            acknowledged: false,
        };
        let v2 = serde_json::to_value(&stamped).unwrap();
        // ADR-0051 wire contract: per-variant payload fields stay snake_case
        // (only the state TAG value is camelCase), so the wire key is
        // "divergent_since", not "divergentSince".
        assert_eq!(v2["divergent_since"], 1_756_521_600i64);
        let back2: StrategySnapshot = serde_json::from_value(v2).unwrap();
        assert_eq!(back2, stamped);
        assert_ne!(back2, missing);
    }
}

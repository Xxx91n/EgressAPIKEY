//! Modular strategy catalog (Phase 3 / NEW-4).
//!
//! Ponytail: do NOT invent a parallel node-picker. Resin v1.1.2 already owns
//! allocation via `allocation_policy`. This module is the shell-side catalog
//! that:
//!   1. Names the user-facing strategies (random / sequential / latency /
//!      quality / protocol_weight).
//!   2. Maps each name onto a real Resin enum value the platform PATCH accepts.
//!   3. Exposes the static protocol-weight table from docs/PROTOCOL_WEIGHT_RESEARCH.md
//!      so the GUI/IP-channel card can rank node protocols for SSE suitability.
//!
//! Bandwidth is intentionally mapped to PREFER_IDLE_IP until Resin grows a
//! native bandwidth policy — documented, not faked at runtime.

use serde::{Deserialize, Serialize};

/// User-facing strategy ids (GUI / whitebox). Stable snake_case strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyId {
    Random,
    Sequential,
    Latency,
    Quality,
    Bandwidth,
    ProtocolWeight,
}

impl StrategyId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Random => "random",
            Self::Sequential => "sequential",
            Self::Latency => "latency",
            Self::Quality => "quality",
            Self::Bandwidth => "bandwidth",
            Self::ProtocolWeight => "protocol_weight",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "random" => Ok(Self::Random),
            "sequential" => Ok(Self::Sequential),
            "latency" | "prefer_low_latency" => Ok(Self::Latency),
            "quality" | "prefer_idle_ip" | "idle" => Ok(Self::Quality),
            "bandwidth" => Ok(Self::Bandwidth),
            "protocol_weight" | "protocol" | "sse" => Ok(Self::ProtocolWeight),
            other => Err(format!("unknown strategy: {other}")),
        }
    }

    /// Map onto the live Resin v1.1.2 `allocation_policy` enum.
    /// protocol_weight has no native Resin value — falls back to BALANCED and
    /// the shell ranks display tags via [`protocol_weight`].
    pub fn to_resin_allocation_policy(self) -> &'static str {
        match self {
            Self::Random | Self::Sequential | Self::ProtocolWeight => "BALANCED",
            Self::Latency => "PREFER_LOW_LATENCY",
            Self::Quality | Self::Bandwidth => "PREFER_IDLE_IP",
        }
    }
}

/// Static SSE suitability weights (docs/PROTOCOL_WEIGHT_RESEARCH.md).
/// Higher is better for long-lived AI SSE / WebSocket streams.
pub fn protocol_weight(protocol: &str) -> f32 {
    let p = protocol.trim().to_ascii_lowercase();
    match p.as_str() {
        "http" | "https" | "socks5" | "socks" | "vmess" | "vless" | "trojan" => 1.0,
        "shadowsocks" | "ss" | "ssr" => 0.7,
        "hysteria2" | "hysteria" | "tuic" | "wireguard" | "wg" => 0.1,
        _ => 0.5,
    }
}

/// Catalog entry for GUI enumeration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StrategyInfo {
    pub id: String,
    pub resin_policy: String,
    pub protocol_aware: bool,
}

pub fn strategy_catalog() -> Vec<StrategyInfo> {
    [
        StrategyId::Random,
        StrategyId::Sequential,
        StrategyId::Latency,
        StrategyId::Quality,
        StrategyId::Bandwidth,
        StrategyId::ProtocolWeight,
    ]
    .into_iter()
    .map(|id| StrategyInfo {
        id: id.as_str().into(),
        resin_policy: id.to_resin_allocation_policy().into(),
        protocol_aware: matches!(id, StrategyId::ProtocolWeight),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_id_to_known_resin_policy() {
        for id in [
            StrategyId::Random,
            StrategyId::Sequential,
            StrategyId::Latency,
            StrategyId::Quality,
            StrategyId::Bandwidth,
            StrategyId::ProtocolWeight,
        ] {
            let p = id.to_resin_allocation_policy();
            assert!(
                matches!(p, "BALANCED" | "PREFER_LOW_LATENCY" | "PREFER_IDLE_IP"),
                "{p}"
            );
        }
    }

    #[test]
    fn parse_accepts_aliases() {
        assert_eq!(
            StrategyId::parse("prefer_low_latency").unwrap(),
            StrategyId::Latency
        );
        assert_eq!(StrategyId::parse("idle").unwrap(), StrategyId::Quality);
        assert!(StrategyId::parse("nope").is_err());
    }

    #[test]
    fn protocol_weight_ranks_sse_friendly_higher() {
        assert!(protocol_weight("vmess") > protocol_weight("shadowsocks"));
        assert!(protocol_weight("socks5") > protocol_weight("hysteria2"));
        assert_eq!(protocol_weight("http"), 1.0);
    }

    #[test]
    fn catalog_lists_six_strategies() {
        let c = strategy_catalog();
        assert_eq!(c.len(), 6);
        assert!(c
            .iter()
            .any(|s| s.id == "protocol_weight" && s.protocol_aware));
    }
}

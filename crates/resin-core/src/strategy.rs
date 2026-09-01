//! B-class strategy type — the shell-facing strategy vocabulary.
//!
//! Ponytail: do NOT invent a parallel node-picker. Resin v1.1.2 already owns
//! allocation via `allocation_policy`.
//!
//! The former catalog face (parse aliases, the Rust-side
//! strategy→allocation_policy mapping, the protocol_weight table,
//! strategy_catalog/StrategyInfo) had no consumer outside its own tests and
//! was deleted (architecture-recovery ticket 24, ADR-0050 direction). The one
//! surviving strategy↔allocation_policy mapping lives in the webview
//! (`src/lib/strategy.ts`, display + PATCH translation) — do not reintroduce
//! a Rust copy.

use serde::{Deserialize, Serialize};

/// User-facing strategy ids (GUI / whitebox). Stable snake_case strings.
/// Type of `strategy_engine::BClassParams::b_class`.
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
}

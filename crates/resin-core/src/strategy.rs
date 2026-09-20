//! B-class strategy type — the shell-facing strategy vocabulary.
//!
//! the vocabulary
//! CONVERGED from six display-only shell options onto Resin's three real
//! egress-selection policies. `StrategyId` now holds exactly the values
//! Resin supports — BALANCED / PREFER_LOW_LATENCY / PREFER_IDLE_IP — and the
//! former six names survive ONLY as accepted input spellings, so an existing
//! whitebox file keeps loading. That is the Expand step of the industrial
//! Expand -> Migrate -> Contract sequence (readers understand both spellings
//! before writers emit the new one); the one-time rewrite of the stored values
//! is `strategy_engine::migrate_b_class_values`, and the Contract step is the
//! removal of the six shell options from the UI and API surface.
//!
//! Ponytail: do NOT invent a parallel node-picker. Resin already owns
//! allocation via `allocation_policy`.
//!
//! The former catalog face (parse aliases, the Rust-side
//! strategy->allocation_policy mapping, the protocol_weight table,
//! strategy_catalog/StrategyInfo) had no consumer outside its own tests and
//! was deleted (ADR-0050 direction). With
//! the remaining many-to-one mapping collapsed to the IDENTITY: the
//! whitebox stores the Resin wire value verbatim, so the desired-vs-observed
//! comparison in `snapshot::merge_strategies` needs no second mapping table,
//! and the translation left in `src/lib/strategy.ts` is display-only.

use serde::{Deserialize, Serialize};

/// The legacy six-option shell catalog (pre-round-8). Accepted as INPUT only;
/// NEVER produced by serialization. Kept as a named list so the one-time
/// migration and its tests share ONE source of truth for "what counts as a
/// legacy token".
pub const LEGACY_B_CLASS_TOKENS: [&str; 6] = [
    "random",
    "sequential",
    "latency",
    "quality",
    "bandwidth",
    "protocol_weight",
];

/// User-facing strategy ids == Resin's `allocation_policy` enum (CONTEXT.md:
/// Egress IP Policy). Stable UPPER_SNAKE wire strings: the whitebox `b_class`
/// field stores the Resin value verbatim, so the snapshot compares desired
/// against observed with no translation at all.
///
/// Deserialization is TOLERANT at the read boundary (the six withdrawn shell
/// options are mapped many-to-one onto the three real policies by the SAME
/// table the webview used before the convergence —
/// `src/lib/strategy.ts::strategyToResinPolicy` — so a not-yet-migrated
/// whitebox keeps describing the policy the shell actually PATCHed), while an
/// unknown token is REJECTED rather than silently coerced: at the persisted
/// truth boundary a closed value set is what makes drift detectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StrategyId {
    #[serde(rename = "BALANCED")]
    Balanced,
    #[serde(rename = "PREFER_LOW_LATENCY")]
    PreferLowLatency,
    #[serde(rename = "PREFER_IDLE_IP")]
    PreferIdleIp,
}

impl<'de> Deserialize<'de> for StrategyId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown b_class value {raw:?}: expected one of                  BALANCED/PREFER_LOW_LATENCY/PREFER_IDLE_IP, or one of the                  withdrawn shell options ({})",
                LEGACY_B_CLASS_TOKENS.join(", ")
            ))
        })
    }
}

impl StrategyId {
    /// Canonical wire value — the exact string Resin accepts and reports for
    /// `allocation_policy`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "BALANCED",
            Self::PreferLowLatency => "PREFER_LOW_LATENCY",
            Self::PreferIdleIp => "PREFER_IDLE_IP",
        }
    }

    /// The legislated many-to-one convergence table: a canonical value or one
    /// of the six withdrawn shell options -> the real policy. Case- and
    /// whitespace-insensitive (hand-edited whitebox files). `None` for
    /// anything else — an unknown token is never silently coerced.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "BALANCED" => Some(Self::Balanced),
            "PREFER_LOW_LATENCY" => Some(Self::PreferLowLatency),
            "PREFER_IDLE_IP" => Some(Self::PreferIdleIp),
            // Withdrawn shell catalog (pre-round-8 whitebox files).
            "RANDOM" | "BANDWIDTH" | "PROTOCOL_WEIGHT" => Some(Self::Balanced),
            "LATENCY" => Some(Self::PreferLowLatency),
            "SEQUENTIAL" | "QUALITY" => Some(Self::PreferIdleIp),
            _ => None,
        }
    }

    /// True when `raw` is one of the six withdrawn shell options, i.e. the
    /// value still needs the one-time rewrite to its canonical spelling.
    pub fn is_legacy_token(raw: &str) -> bool {
        let token = raw.trim().to_ascii_lowercase();
        LEGACY_B_CLASS_TOKENS.contains(&token.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The convergence table, asserted row by row against the historical
    /// webview mapping (`strategyToResinPolicy`) it replaces.
    #[test]
    fn convergence_table_matches_the_withdrawn_shell_mapping() {
        for legacy in ["random", "bandwidth", "protocol_weight"] {
            assert_eq!(
                StrategyId::parse(legacy),
                Some(StrategyId::Balanced),
                "{legacy}"
            );
        }
        assert_eq!(
            StrategyId::parse("latency"),
            Some(StrategyId::PreferLowLatency)
        );
        for legacy in ["sequential", "quality"] {
            assert_eq!(
                StrategyId::parse(legacy),
                Some(StrategyId::PreferIdleIp),
                "{legacy}"
            );
        }
        for id in [
            StrategyId::Balanced,
            StrategyId::PreferLowLatency,
            StrategyId::PreferIdleIp,
        ] {
            assert_eq!(StrategyId::parse(id.as_str()), Some(id), "{}", id.as_str());
        }
        // Case/whitespace tolerance for hand-edited files.
        assert_eq!(StrategyId::parse(" balanced "), Some(StrategyId::Balanced));
        assert_eq!(
            StrategyId::parse("prefer_idle_ip"),
            Some(StrategyId::PreferIdleIp)
        );
        // Unknown tokens are never coerced.
        assert_eq!(StrategyId::parse("p2c"), None);
        assert_eq!(StrategyId::parse(""), None);
    }

    #[test]
    fn canonical_values_are_the_resin_wire_spelling() {
        assert_eq!(StrategyId::Balanced.as_str(), "BALANCED");
        assert_eq!(StrategyId::PreferLowLatency.as_str(), "PREFER_LOW_LATENCY");
        assert_eq!(StrategyId::PreferIdleIp.as_str(), "PREFER_IDLE_IP");
    }

    /// Legacy tokens deserialize (Expand step) but ALWAYS serialize back as the
    /// canonical value, so ONE read+write cycle migrates the stored spelling.
    #[test]
    fn legacy_tokens_deserialize_but_serialize_canonical() {
        for (legacy, expected) in [
            ("random", "BALANCED"),
            ("bandwidth", "BALANCED"),
            ("protocol_weight", "BALANCED"),
            ("latency", "PREFER_LOW_LATENCY"),
            ("sequential", "PREFER_IDLE_IP"),
            ("quality", "PREFER_IDLE_IP"),
        ] {
            let parsed: StrategyId =
                serde_json::from_value(serde_json::Value::String(legacy.to_string())).unwrap();
            assert_eq!(parsed.as_str(), expected, "{legacy}");
            // Serialization ALWAYS emits the canonical wire spelling.
            assert_eq!(
                serde_json::to_value(parsed).unwrap(),
                serde_json::Value::String(expected.to_string())
            );
        }
    }

    /// An unknown value fails the parse with a diagnostic naming the legal set
    /// — the closed-enum discipline that keeps drift detectable.
    #[test]
    fn unknown_token_is_rejected_not_coerced() {
        let err =
            serde_json::from_value::<StrategyId>(serde_json::Value::String("p2c".to_string()))
                .unwrap_err()
                .to_string();
        assert!(err.contains("unknown b_class value"), "{err}");
        assert!(err.contains("PREFER_LOW_LATENCY"), "{err}");
    }

    #[test]
    fn is_legacy_token_flags_exactly_the_withdrawn_catalog() {
        for token in LEGACY_B_CLASS_TOKENS {
            assert!(StrategyId::is_legacy_token(token), "{token}");
        }
        for canonical in [
            "BALANCED",
            "PREFER_LOW_LATENCY",
            "PREFER_IDLE_IP",
            "balanced",
            "p2c",
            "",
        ] {
            assert!(!StrategyId::is_legacy_token(canonical), "{canonical}");
        }
    }
}

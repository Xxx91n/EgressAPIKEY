//! Config export/import document (Round 5 T07 / ADR-0061).
//!
//! `config_export` reads the two L2 whitebox documents verbatim (ADR-0036:
//! the whitebox files are the truth — it never reads Resin) and wraps them
//! in one JSON container. `config_import` parses that container, validates
//! each whitebox document through its OWN validate entry
//! (`strategy_service::validate` / `whitebox_config::validate`), and returns
//! the two typed documents for the shell to persist through the sanctioned
//! write entries (`StrategyService::store` / `WhiteboxConfigStore::apply`).
//!
//! This module is PURE: it never touches the filesystem or Resin, so it is
//! unit-testable without a sidecar.

use serde_json::Value;

use crate::strategy_engine::StrategyConfig;
use crate::whitebox_config::WhiteboxConfig;

/// Version of the config-export container format. Bumped independently of the
/// inner whitebox document versions (which carry their own `version` field).
pub const CONFIG_EXPORT_FORMAT_VERSION: u8 = 1;

/// The two typed whitebox documents a valid import carries.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigImportDoc {
    pub strategy: StrategyConfig,
    pub ports: WhiteboxConfig,
}

/// Build the export container from the two whitebox documents.
///
/// The whitebox documents are embedded verbatim (serde round-trip of the
/// in-memory typed values) so a re-import can only ever carry the exact
/// validated shapes the stores already produce — no field re-mapping, no
/// Resin-derived assembly (that is the deferred "Include Resin derived"
/// option, out of scope this round).
pub fn build_export_doc(
    strategy: &StrategyConfig,
    ports: &WhiteboxConfig,
    exported_at: &str,
) -> Value {
    serde_json::json!({
        "format": "egressapikey-config",
        "version": CONFIG_EXPORT_FORMAT_VERSION,
        "exported_at": exported_at,
        "strategy": strategy,
        "ports": ports,
    })
}

/// Parse + validate an import document. Returns `Err` (never writes) on any
/// structural or schema violation, so the command can surface a clear
/// `IpcError` without mutating state.
///
/// Schema-version compat: the inner whitebox documents are deserialized
/// through serde — unknown fields are ignored, so T09's v2 additions (e.g.
/// `generation`) cannot break parsing — and then re-validated through their
/// own validate entry, which is where the version gate lives. Today
/// `strategy_service::validate` accepts version 1 only; when T09 bumps the
/// strategy schema to v2 and relaxes that gate, this import path keeps
/// working unchanged because it only forwards the typed document. A document
/// whose version is not supported fails cleanly (never a silent mis-parse).
pub fn parse_import_doc(v: &Value) -> Result<ConfigImportDoc, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "config_import: document must be a JSON object".to_string())?;

    let container_version = obj
        .get("version")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "config_import: missing 'version' (number)".to_string())?;
    if container_version != CONFIG_EXPORT_FORMAT_VERSION as u64 {
        return Err(format!(
            "config_import: unsupported export version {container_version} (expected {CONFIG_EXPORT_FORMAT_VERSION})"
        ));
    }

    let strategy_raw = obj
        .get("strategy")
        .ok_or_else(|| "config_import: missing 'strategy' whitebox document".to_string())?;
    let ports_raw = obj
        .get("ports")
        .ok_or_else(|| "config_import: missing 'ports' whitebox document".to_string())?;

    let strategy: StrategyConfig = serde_json::from_value(strategy_raw.clone())
        .map_err(|e| format!("config_import: strategy whitebox invalid: {e}"))?;
    let ports: WhiteboxConfig = serde_json::from_value(ports_raw.clone())
        .map_err(|e| format!("config_import: ports whitebox invalid: {e}"))?;

    crate::strategy_service::validate(&strategy)
        .map_err(|e| format!("config_import: strategy whitebox rejected: {e}"))?;
    crate::whitebox_config::validate(&ports)
        .map_err(|e| format!("config_import: ports whitebox rejected: {e}"))?;

    Ok(ConfigImportDoc { strategy, ports })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_strategy() -> StrategyConfig {
        StrategyConfig {
            version: 1,
            platforms: vec![crate::strategy_engine::PlatformStrategy {
                platform_name: "Anthropic".into(),
                a_class: crate::strategy_engine::AClassStrategy::Region,
                b_class: crate::strategy::StrategyId::Random,
                manual_nodes: vec![],
                regions: vec!["US".into()],
                subscriptions: vec![],
                top_n: 10,
                b_class_params: Default::default(),
            }],
            acknowledged: vec![],
        }
    }

    fn sample_ports() -> WhiteboxConfig {
        WhiteboxConfig::from_ports(vec![crate::db::PortMapping {
            port: 17990,
            protocol: "socks5".into(),
            platform_name: "Anthropic".into(),
            account: String::new(),
            label: String::new(),
            enabled: true,
            auth_required: false,
        }])
    }

    #[test]
    fn export_doc_round_trips_through_parse() {
        let doc = build_export_doc(&sample_strategy(), &sample_ports(), "2026-09-03T00:00:00Z");
        let parsed = parse_import_doc(&doc).unwrap();
        assert_eq!(parsed.strategy, sample_strategy());
        assert_eq!(parsed.ports, sample_ports());
    }

    #[test]
    fn export_doc_embeds_whitebox_documents_verbatim() {
        let doc = build_export_doc(&sample_strategy(), &sample_ports(), "t");
        assert_eq!(doc["format"], json!("egressapikey-config"));
        assert_eq!(doc["version"], json!(CONFIG_EXPORT_FORMAT_VERSION));
        // The whitebox documents are the raw strategy/ports shapes, not the
        // former Resin-derived {platforms, subscriptions} assembly.
        assert!(doc["strategy"]["platforms"].is_array());
        assert!(doc["ports"]["entry_ports"].is_array());
        assert!(doc["platforms"].is_null());
        assert!(doc["subscriptions"].is_null());
    }

    #[test]
    fn parse_rejects_non_object_and_missing_version() {
        assert!(parse_import_doc(&json!([])).unwrap_err().contains("object"));
        let no_version = json!({"strategy": sample_strategy(), "ports": sample_ports()});
        assert!(parse_import_doc(&no_version).unwrap_err().contains("version"));
    }

    #[test]
    fn parse_rejects_unsupported_container_version() {
        let doc = json!({"version": 2, "strategy": sample_strategy(), "ports": sample_ports()});
        assert!(parse_import_doc(&doc)
            .unwrap_err()
            .contains("unsupported export version 2"));
    }

    #[test]
    fn parse_rejects_missing_whitebox_documents() {
        let doc = json!({"version": 1, "strategy": sample_strategy()});
        assert!(parse_import_doc(&doc).unwrap_err().contains("ports"));
        let doc2 = json!({"version": 1, "ports": sample_ports()});
        assert!(parse_import_doc(&doc2).unwrap_err().contains("strategy"));
    }

    #[test]
    fn parse_rejects_invalid_strategy_schema_without_write() {
        // strategy version 2 = T09's future schema. serde parses it (the
        // unknown `generation` field is ignored), and validation rejects it
        // cleanly with a named version — this layer never writes.
        let mut s = sample_strategy();
        s.version = 2;
        let doc = json!({"version": 1, "strategy": s, "ports": sample_ports()});
        let err = parse_import_doc(&doc).unwrap_err();
        assert!(err.contains("strategy whitebox rejected"), "got: {err}");
    }

    #[test]
    fn parse_rejects_invalid_ports_schema() {
        let mut p = sample_ports();
        p.version = 2;
        let doc = json!({"version": 1, "strategy": sample_strategy(), "ports": p});
        let err = parse_import_doc(&doc).unwrap_err();
        assert!(err.contains("ports whitebox rejected"), "got: {err}");
    }

    #[test]
    fn parse_tolerates_t09_v2_unknown_fields_on_strategy() {
        // A v2-preview strategy doc with a `generation` field deserializes
        // without crashing (serde ignores unknown fields); the version gate is
        // the ONLY thing that rejects it. This locks the forward-compat
        // contract: when T09 relaxes validate() to accept v2, this import path
        // keeps working unchanged.
        let doc = json!({
            "version": 1,
            "strategy": { "version": 2, "generation": 3, "applied_generation": 3, "platforms": [] },
            "ports": sample_ports()
        });
        let err = parse_import_doc(&doc).unwrap_err();
        assert!(err.contains("strategy whitebox rejected"), "got: {err}");
        assert!(err.contains("version"), "got: {err}");
    }
}

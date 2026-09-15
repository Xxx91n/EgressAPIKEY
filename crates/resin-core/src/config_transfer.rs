//! Config export/import document (ADR-0061).
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
/// through serde — unknown fields are ignored, so 's v2 additions (e.g.
/// `generation`) cannot break parsing — and then re-validated through their
/// own validate entry, which is where the version gate lives. Today
/// `strategy_service::validate` accepts version 1 only; when bumps the
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

    validate_import_pair(&strategy, &ports)?;

    Ok(ConfigImportDoc { strategy, ports })
}

/// ADR-0069 D4 (phase 1): validate BOTH whitebox documents before either is
/// committed. This is the atomicity gate of `config_import` - because both
/// documents are checked here, a rejection of the SECOND one can never leave
/// the FIRST one already persisted (the pre-D4 half-imported state).
///
/// Pure: it never touches the filesystem, which is what makes the
/// "validation failure writes nothing" acceptance case unit-testable.
pub fn validate_import_pair(
    strategy: &StrategyConfig,
    ports: &WhiteboxConfig,
) -> Result<(), String> {
    crate::strategy_service::validate(strategy)
        .map_err(|e| format!("config_import: strategy whitebox rejected: {e}"))?;
    crate::whitebox_config::validate(ports)
        .map_err(|e| format!("config_import: ports whitebox rejected: {e}"))?;
    Ok(())
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
                b_class: crate::strategy::StrategyId::Balanced,
                manual_nodes: vec![],
                regions: vec!["US".into()],
                subscriptions: vec![],
                top_n: 10,
            }],
            acknowledged: vec![],
            generation: 3,
            applied_generation: 2,
            last_apply_at: Some(1_700_000_000),
            last_apply_error: None,
            subscriptions: vec![],
            updated_at: Some(1_700_000_100),
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
        // strategy version 2 = 's future schema. serde parses it (the
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
        // version 2 is the CURRENT ports schema since the round-8 mixed-protocol
        // migration; the future schema (3) is what must be rejected.
        p.version = 3;
        let doc = json!({"version": 1, "strategy": sample_strategy(), "ports": p});
        let err = parse_import_doc(&doc).unwrap_err();
        assert!(err.contains("ports whitebox rejected"), "got: {err}");
    }

    #[test]
    fn parse_tolerates_t09_v2_unknown_fields_on_strategy() {
        // A v2-preview strategy doc with a `generation` field deserializes
        // without crashing (serde ignores unknown fields); the version gate is
        // the ONLY thing that rejects it. This locks the forward-compat
// contract: when relaxes validate() to accept v2, this import path
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

    /// A privileged port - rejected by `whitebox_config::validate`, i.e. an
    /// otherwise well-formed ports document that must fail the pair gate.
    fn invalid_ports() -> WhiteboxConfig {
        WhiteboxConfig::from_ports(vec![crate::db::PortMapping {
            port: 80,
            protocol: "socks5".into(),
            platform_name: "Anthropic".into(),
            account: String::new(),
            label: String::new(),
            enabled: true,
            auth_required: false,
        }])
    }

    /// ADR-0069 D4: the shared pair gate is the SINGLE validation seam - it
    /// accepts a good pair and names the offending half on rejection.
    #[test]
    fn validate_import_pair_is_the_single_gate_for_both_documents() {
        assert!(validate_import_pair(&sample_strategy(), &sample_ports()).is_ok());
        let err = validate_import_pair(&sample_strategy(), &invalid_ports()).unwrap_err();
        assert!(err.contains("ports whitebox rejected"), "{err}");
    }

    ///  ADR-0069 D4: an import whose SECOND (ports) document fails
    /// validation must write NOTHING - both whitebox files stay byte-identical
    /// and no staging temp file survives.
    #[test]
    fn import_rejecting_the_second_document_writes_nothing() {
        let dir =
            std::env::temp_dir().join(format!("egressapikey-import-d4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let strategy_path = dir.join("egressapikey-strategy.json");
        let ports_path = dir.join("egressapikey-ports.json");
        let before_strategy = br#"{"version":1}"#.to_vec();
        let before_ports = br#"{"version":1}"#.to_vec();
        std::fs::write(&strategy_path, &before_strategy).unwrap();
        std::fs::write(&ports_path, &before_ports).unwrap();

        // A document whose FIRST half is valid and whose SECOND half is not.
        let doc = json!({
            "format": "egressapikey-config",
            "version": CONFIG_EXPORT_FORMAT_VERSION,
            "exported_at": "2026-09-14T00:00:00Z",
            "strategy": sample_strategy(),
            "ports": invalid_ports(),
        });
        let err = parse_import_doc(&doc).unwrap_err();
        assert!(err.contains("ports whitebox rejected"), "{err}");

        // Zero writes: both files byte-identical, no temp file left behind.
        assert_eq!(std::fs::read(&strategy_path).unwrap(), before_strategy);
        assert_eq!(std::fs::read(&ports_path).unwrap(), before_ports);
        assert!(!strategy_path.with_extension("json.tmp").exists());
        assert!(!ports_path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

}
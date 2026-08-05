//! resin-core headless binary entrypoint.
//!
//! This binary is a **stub**: it parses CLI args, optionally loads a JSON config,
//! emits an honest startup banner, and exits. It does NOT bind axum, spawn the
//! Resin Go sidecar, or touch the network. The desktop Tauri shell
//! (src-tauri/src/main.rs) owns the live gateway lifecycle (boot_resin +
//! interceptor) in production. Full GUI<->CLI parity (a VPS headless mode that
//! exposes the 33 IPC commands over HTTP) is tracked in ADR-0009 and ADR-0001;
//! it is a multi-crate refactor deferred to a grill VPS-phase decision.

use anyhow::Result;
use clap::Parser;
use resin_core::{sanitize_lanes, CoreConfig};
#[cfg(test)]
use resin_core::{MAX_LANES, MIN_LANES};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "resin-core",
    version,
    about = "Resin-pattern L7 gateway stub for AI API keys (headless placeholder)"
)]
struct Args {
    /// Path to a JSON config file. If absent or unreadable, falls back to
    /// CoreConfig::default() and logs a warning.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("resin-core is a stub binary; the desktop Tauri shell owns the live gateway lifecycle. Full GUI<->CLI parity is tracked in ADR-0009 + ADR-0001. This process does NOT listen on any port.");
    let args = Args::parse();
    let cfg = load_config(args.config.as_ref());
    tracing::info!(bind = %cfg.bind, lanes = cfg.lanes, mihomo_api = %cfg.mihomo_api, "resin-core stub starting (headless; no network listener)");
    Ok(())
}

/// Read a JSON config file into CoreConfig. Failures fall back to default.
fn load_config(path: Option<&PathBuf>) -> CoreConfig {
    let Some(p) = path else {
        return CoreConfig::default();
    };
    if !p.exists() {
        return CoreConfig::default();
    }
    match std::fs::read(p) {
        Ok(bytes) => match serde_json::from_slice::<CoreConfig>(&bytes) {
            Ok(mut c) => {
                c.lanes = sanitize_lanes(c.lanes);
                c
            }
            Err(_) => CoreConfig::default(),
        },
        Err(_) => CoreConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("resin-core-cfg-{tag}-{}.json", std::process::id()))
    }

    #[test]
    fn load_config_happy_path() {
        let p = tmp_path("happy");
        let json = r#"{"lanes":25,"bind":"127.0.0.1:9999","mihomo_api":"http://127.0.0.1:9090","mihomo_secret":null}"#;
        std::fs::write(&p, json).unwrap();
        let cfg = load_config(Some(&p));
        assert_eq!(cfg.lanes, 25);
        assert_eq!(cfg.bind, "127.0.0.1:9999");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_config_clamps_lanes() {
        let p = tmp_path("clamp");
        let json = r#"{"lanes":99999,"bind":"x","mihomo_api":"y","mihomo_secret":null}"#;
        std::fs::write(&p, json).unwrap();
        let cfg = load_config(Some(&p));
        assert_eq!(cfg.lanes, MAX_LANES);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_config_missing_file_falls_back_to_default() {
        let p = PathBuf::from("/nonexistent/resin-core-does-not-exist-9382.json");
        let cfg = load_config(Some(&p));
        assert_eq!(cfg.lanes, 10);
        assert_eq!(cfg.bind, "127.0.0.1:7897");
    }

    #[test]
    fn load_config_none_uses_default() {
        let cfg = load_config(None);
        assert_eq!(cfg.lanes, 10);
        assert!(cfg.lanes >= MIN_LANES && cfg.lanes <= MAX_LANES);
    }

    #[test]
    fn load_config_bad_json_falls_back_to_default() {
        let p = tmp_path("bad");
        std::fs::write(&p, "this is not json {").unwrap();
        let cfg = load_config(Some(&p));
        assert_eq!(cfg.lanes, 10);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_config_does_not_touch_network() {
        let cfg = load_config(None);
        assert_eq!(cfg.bind, "127.0.0.1:7897");
    }
}

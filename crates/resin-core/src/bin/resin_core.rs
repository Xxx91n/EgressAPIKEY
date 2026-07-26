//! resin-core headless binary entrypoint.
//!
//! Used by the Tauri sidecar and the pure-backend release target. Parses a
//! config path, sets up tracing, and runs the gateway. The mihomo sidecar is
//! a separate OS process managed by the Tauri shell; this binary owns the L7
//! gateway and the lease/tdewma state.

use anyhow::Result;
use clap::Parser;
use resin_core::CoreConfig;

#[derive(Parser, Debug)]
#[command(name = "resin-core", version, about = "Resin-pattern L7 gateway for AI API keys")]
struct Args {
    /// Path to a JSON config file. Defaults to built-in CoreConfig::default().
    #[arg(long)]
    config: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let _args = Args::parse();
    let cfg = CoreConfig::default();
    tracing::info!(bind=%cfg.bind, lanes=cfg.lanes, "resin-core starting (headless stub)");
    // P1.f: bind axum gateway on cfg.bind with lanes; proxy upstream through mihomo sidecar.
    // The current release ships this binary so the CI backend target compiles; the Tauri
    // shell orchestrates the full gateway lifecycle in production.
    Ok(())
}

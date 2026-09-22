//! bench-forwarder: minimal Mode-A (shell) entry-port listener for the perf
//! baseline harness. Binds ONE entry port via PortForwarder::shell against an
//! in-memory DbPool, injects Platform.Account:proxy_token, and relays proxy
//! bytes to the engine's consolidated port - the exact path a desktop Mode A
//! client takes. Driven entirely by env vars so scripts/bench/run-bench.mjs
//! can spawn it next to the Resin sidecar:
//!
//!   BENCH_FWD_PORT          entry port to bind (client-facing, no creds)
//!   BENCH_FWD_ENGINE_PORT   engine consolidated port (admin+proxy, Mode B port)
//!   BENCH_FWD_PLATFORM      platform name for the injected identity
//!   BENCH_FWD_ACCOUNT       account segment (default "bench")
//!   BENCH_FWD_PROXY_TOKEN   proxy token half of the injected credential
//!   BENCH_FWD_PROTOCOL      declared entry protocol (default "mixed")
//!
//! Build: cargo build --release -p resin-core --features bench --bin bench-forwarder
//! The bin is feature-gated (required-features = ["bench"]) so normal builds
//! never compile it.

use resin_core::db::{DbPool, PortMapping};
use resin_core::port_forwarder::PortForwarder;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let port: u16 = env_or("BENCH_FWD_PORT", "")
        .parse()
        .map_err(|_| "BENCH_FWD_PORT must be a u16".to_string())?;
    let engine_port: u16 = env_or("BENCH_FWD_ENGINE_PORT", "")
        .parse()
        .map_err(|_| "BENCH_FWD_ENGINE_PORT must be a u16".to_string())?;
    let platform = env_or("BENCH_FWD_PLATFORM", "bench");
    let account = env_or("BENCH_FWD_ACCOUNT", "bench");
    let token = env_or("BENCH_FWD_PROXY_TOKEN", "");
    let protocol = env_or("BENCH_FWD_PROTOCOL", "mixed");

    let db = DbPool::open_in_memory()?;
    let forwarder = PortForwarder::shell(db, "127.0.0.1", engine_port, token);
    let rows = vec![PortMapping {
        port,
        protocol,
        platform_name: platform,
        account,
        label: "bench".to_string(),
        enabled: true,
        auth_required: false,
    }];
    forwarder.reload(&rows).await?;

    // Readiness contract for the harness: one line once the listener is bound.
    // The bench polls the TCP port too; this line is for the run log.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if forwarder.running_ports().contains(&port) {
            println!("bench-forwarder listening on 127.0.0.1:{port}");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    // Park forever; the harness kills the child when the run ends.
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

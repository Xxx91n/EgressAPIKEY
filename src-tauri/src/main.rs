//! Binary entrypoint for the ai-api-route desktop shell. P2 will replace this
//! with the tauri::Builder pipeline; the placeholder keeps the workspace
//! buildable and the binary runnable for smoke tests.

fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let _ = ai_api_route_app::init();
    println!("ai-api-route shell (placeholder) - resin-core exits cleanly");
}

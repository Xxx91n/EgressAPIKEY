//! Ticket 09 (tauri-specta pilot): regenerate src/bindings.ts in a debug
//! cargo-test build and assert the pilot contract is present. See
//! src/specta_bindings.rs for why this lives in an integration test.

use egressapikey_app::specta_bindings;
use specta_typescript::Typescript;

#[test]
fn export_bindings_ts_and_assert_pilot_contract() {
    specta_bindings::builder::<tauri::test::MockRuntime>()
        .export(Typescript::new(), "../src/bindings.ts")
        .expect("failed to export bindings.ts");
    let ts = std::fs::read_to_string("../src/bindings.ts").expect("bindings.ts readable");
    for needle in [
        "set_log_level",
        "port_toggle",
        "PortMapping",
        "IpcError",
        "LogLevel",
    ] {
        assert!(ts.contains(needle), "bindings.ts missing: {needle}");
    }
}

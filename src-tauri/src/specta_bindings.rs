//! tauri-specta pilot export module.
//!
//! TYPE generation is decoupled from runtime registration on purpose:
//! tauri-specta rc.25 cannot partially replace tauri::generate_handler!
//! (an invoke_handler covers all-or-nothing commands), so the runtime
//! registry in main.rs stays untouched (65 commands) while this module
//! collects only the two pilot commands. The export itself runs from
//! the integration test tests/bindings_export.rs (a debug-profile cargo
//! test build, matching the ticket's 'debug build generates bindings'
//! clause); the generated file is committed to the repo.
//!
//! Why an integration test instead of a unit test here: the test must
//! monomorphize Builder, which pulls tao/muda objects with comctl32 v6
//! imports into whichever test binary contains it. tauri_build embeds the
//! SxS manifest only into bin targets, so the lib unit-test harness
//! would die at load time (STATUS_ENTRYPOINT_NOT_FOUND). The integration
//! test target receives the manifest via cargo:rustc-link-arg-tests
//! (see build.rs). The test monomorphizes with tauri::test::MockRuntime
//! (tauri's own headless test runtime), not Wry, so it never opens a
//! real window. A post-adoption invoke_handler would call
//! builder::<tauri::Wry>() as usual.
//!
//! 64-bit ints carry #[specta(type = u32)] field overrides where they
//! appear: specta-typescript 0.0.12 has no bigint
//! re-behavior knob and forbids u64/usize by default, so the override
//! is the only way to keep the generated shapes 'number' and drop-in
//! compatible with the hand-copied types they replace.

use crate::commands;
use tauri::Runtime;
use tauri_specta::{Builder, collect_commands};

/// Pilot set: one enum-param (set_log_level), one rich-error-path
/// mutation (port_toggle).
/// Generic over the runtime: the export test uses MockRuntime; a
/// post-adoption invoke_handler would use Wry.
pub fn builder<R: Runtime>() -> Builder<R> {
    Builder::<R>::new().commands(collect_commands![
        commands::set_log_level,
        commands::port_toggle,
    ])
}

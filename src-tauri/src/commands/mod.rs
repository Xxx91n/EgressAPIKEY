//! IPC commands exposed to the React frontend via `tauri::generate_handler!`.
//!
//! Path A (fork Resin Go sidecar): the webview talks to the Resin Go control
//! plane WRAPPED behind Rust IPC. Platform create/list/delete are FORWARDED
//! to Resin via ResinClient. Every command validates its inputs at the IPC
//! boundary (AGENTS 7.5) before touching resin-core or the sidecar client.
//!
//! Domain layout (architecture-recovery ticket 08): implementations live in
//! per-domain submodules - platform / strategy / ports / backup / settings /
//! diagnostics - with shared IPC-boundary helpers in `common`. This facade
//! re-exports the full command set so the `generate_handler!` registry in
//! main.rs, the AGENTS.md 7.6 manifest, and the TS-side `src/lib/ipc.ts`
//! surface stay byte-for-byte unchanged.

mod common;
pub use common::*;

mod platform;
pub use platform::*;

mod backup;
pub use backup::*;

mod strategy;
pub use strategy::*;

mod settings;
pub use settings::*;

mod diagnostics;
pub use diagnostics::*;

mod ports;
pub use ports::*;

#[cfg(test)]
mod tests;


// ---------------------------------------------------------------------------
// Subscriptions - FORWARDED to Resin via ResinClient (DESIGN.md /subscriptions).

// ---- Process routing (issue 3+10) ----
// Per-process -> lane routing rules stored server-side in tauri-plugin-store.
// The Rust side rejects lane collisions (the same target lane already bound
// to a different process in a live rule) BEFORE we record the rule. This is the
// user-visible "conflict detect + refuse + clear toast" surface. The auth
// boundary stays on the OS side (server-trusted store); per-request auth lives
// in the Resin sidecar proxy. We only own the routing rule registry here.

// Phase R1: the topology canvas hot-switch and the node-pool tab need the
// full platform schema (not just names) and the node list. These forward to
// Resin with the same input-validation discipline as the other commands.

// --- WebDAV backup (clash-verge-rev pattern: zip config + upload to WebDAV) ---
// Ponytail: no reqwest_dav crate — reqwest does HTTP PUT for WebDAV upload.
// The webview never sees the password; it passes through tauri-plugin-store.
// We validate the URL shape (http(s)://) and length-cap before issuing the PUT.

// Pure helper: returns Err(msg) if adding {process, target_port} would
// conflict with an existing rule (same target lane, different process).
// Extracted for unit testing without an AppHandle.

// ---------------------------------------------------------------------------
// Strategy Engine (T4-4 / ADR-0022) — whitebox per-platform strategy config.
// The shell polls Resin /nodes, applies A-class filters, PATCHes region_filters.
// B-class maps to Resin allocation_policy via the existing platform_update IPC.

// --- T18 Phase 1: Port health batch probe (ADR-0042 S1) ---------------------
//
// A single watch_port_health command drives the background Tokio task that
// probes every enabled entry-port concurrently (cap 10) and streams
// PortHealthSnapshot down the Tauri ipc::Channel. The shared paused flag is
// toggled by WindowEvent::Focused in main.rs (tab-hidden pause pattern).
// Disabled ports (PortMapping.enabled=false) are filtered out by the ports_fn
// closure before probing, matching S2's "whitebox enabled is truth source".

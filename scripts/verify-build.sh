#!/usr/bin/env bash
set -euo pipefail

# T9-8: CI/local test split — CI runs full workspace, local skips app crate
IS_CI="${CI:-false}"

# R11-14 CI gates (wave-b D-001, audit handover F1):
# 1) lockfile-diff — Cargo.lock out of sync with Cargo.toml fails red here
#    (cargo metadata --locked is read-only, produces no build artifacts).
# 2) cargo fmt --check — stops fmt residue from continuing to leak through.
echo "[verify] lockfile freshness (cargo metadata --locked)"
cargo metadata --locked --format-version 1 >/dev/null

echo "[verify] cargo fmt --check"
cargo fmt --all -- --check

echo "[verify] cargo build"
if [ "$IS_CI" = "true" ]; then
  if [ "$(uname -s 2>/dev/null)" = "Linux" ] || [ "$(uname -s 2>/dev/null)" = "Darwin" ]; then
    echo "[verify] NOTE: non-Windows CI - app crate LINK skipped (GUI jobs cover it); the headless bin is cargo-checked below"
    cargo build -p resin-core --quiet
    # f682940e-class guard (architecture-recovery ticket 03 / IMP-3): the
    # headless bin lives in src-tauri, which this branch never compiled, so a
    # bin that no longer builds still passed the push gate. `cargo check` needs
    # no linker, so it can run here; --all-targets also type-checks the test
    # modules (a plain check leaves cfg(test) off, which would hide a broken
    # regression lock).
    cargo check -p egressapikey-app --features headless --all-targets --quiet
  else
    cargo build --workspace --quiet
  fi
else
  cargo build -p resin-core --quiet
  cargo build -p egressapikey-app --features custom-protocol --quiet || echo "[verify] WARN: egressapikey-app build skipped (host linker issue)"
fi

echo "[verify] cargo test"
if [ "$IS_CI" = "true" ]; then
  if [ "$(uname -s 2>/dev/null)" = "Linux" ] || [ "$(uname -s 2>/dev/null)" = "Darwin" ]; then
    cargo test -p resin-core --quiet
    # round10 ticket 04 (A-005 / D-004 B'): run the app-crate lib tests
    # on CI so the restart_into_slot regression lock (sidecar.rs
    # #[cfg(test)]) produces real evidence. Scoped to --lib to avoid the
    # specta bindings integration test (tests/bindings_export.rs).
    cargo test -p egressapikey-app --lib --quiet
  else
    cargo test --workspace --quiet
  fi
else
  cargo test -p resin-core --lib --quiet
fi

echo "[verify] pnpm build (tsc + vite)"
npx tsc -b
npx vite build

# GUI half of the f682940e-class guard (architecture-recovery ticket 06 /
# IMP-6 #1+#2). Ticket 03 closed the headless-bin hole; this closes the GUI
# path that actually ships to users: custom-protocol is the ONLY feature that
# embeds dist/ into the GUI exe. It MUST run AFTER `vite build` - with
# custom-protocol on, tauri::generate_context! resolves frontendDist (../dist)
# at compile time and fails when the directory is missing, and dist/ is
# gitignored, so a fresh CI checkout has none.
if [ "$IS_CI" = "true" ]; then
  if [ "$(uname -s 2>/dev/null)" = "Linux" ] || [ "$(uname -s 2>/dev/null)" = "Darwin" ]; then
    echo "[verify] cargo check (GUI custom-protocol path)"
    cargo check -p egressapikey-app --features custom-protocol --quiet
  fi
fi

echo "[verify] pnpm test (vitest)"
npx vitest run

echo "[verify] i18n coverage"
node scripts/i18n-check.cjs
echo "[verify] ipc manifest guard"
node scripts/ipc-manifest-check.cjs
echo "[verify] vitest isolation guard (ticket 18)"
node scripts/vitest-isolation-guard.cjs
echo "[verify] license field consistency (ticket 01, spec D-06)"
node scripts/license-field-check.cjs
echo "[verify] bilingual README alignment (ticket 03, spec D-06)"
node scripts/readme-lang-check.cjs
echo "[verify] upstream router integrity (ticket 14, spec D-C3.9)"
node scripts/upstream-router-check.cjs
echo "[verify] contracts (mode-a contract gate, ADR-0068 D4 / round9 D-001)"
node scripts/mode-a-contract-check.cjs

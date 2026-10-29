#!/usr/bin/env bash
set -euo pipefail

# CI/local test split — CI runs full workspace, local skips app crate
IS_CI="${CI:-false}"

# CI gates:
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
    echo "[verify] NOTE: non-Windows CI - GUI app crate LINK skipped (GUI jobs cover it)"
    cargo build -p resin-core --quiet
    # Compile-blind-spot guard: the
    # headless bin lives in src-tauri, which this CI path never compiled, so a
    # bin that no longer builds still passed the push gate. `cargo check` needs
    # no linker, so it can run here; --all-targets also type-checks the test
    # modules (a plain check leaves cfg(test) off, which would hide a broken
    # regression lock).
    cargo check -p egressapikey-app --features headless --all-targets --quiet
    # The verify job must also produce the real headless binary so
    # the smoke script below exercises a live process (验收: 编译通过、启动
    # 并测活软件进程). The bin is a pure axum server - no webkit link deps -
    # and the backend release matrix already builds it on ubuntu-22.04.
    cargo build -p egressapikey-app --features headless --bin egressapikey-headless --quiet
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
    # run the app-crate lib tests
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

# The lock-wait instrument compiles only under
# --features db-lock-metrics (observation builds, never shipping). This
# pass keeps it live code rather than dead cfg text: --all-targets
# type-checks the feature-gated test modules too (the compile-blind-spot
# guard above), and the test run exercises the gated probe/holder tests once.
echo "[verify] db-lock-metrics feature pass (observation-only instrument)"
cargo check -p resin-core --features db-lock-metrics --all-targets --quiet
cargo test -p resin-core --features db-lock-metrics --lib --quiet

echo "[verify] pnpm build (tsc + vite)"
npx tsc -b
npx vite build

# This is the GUI half of the compile-blind-spot guard.
# The earlier pass closed the headless-bin hole; this closes the GUI
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
echo "[verify] license field consistency"
node scripts/license-field-check.cjs
echo "[verify] ipc command registry (definitions = generate_handler!)"
node scripts/ipc-manifest-check.cjs
echo "[verify] ipc command registry parser self-test"
node scripts/ipc-manifest-check.cjs --self-test
echo "[verify] vitest isolation guard"
node scripts/vitest-isolation-guard.cjs
echo "[verify] contracts (mode-a contract gate)"
node scripts/mode-a-contract-check.cjs

# replace_ports' BEGIN-IMMEDIATE requirement is a source
# gate, not a self-reading test - comments stripped, whitespace
# normalized, only the transaction_with_behavior(Immediate) call counts.
echo "[verify] db BEGIN IMMEDIATE source gate"
node scripts/db-immediate-txn-check.cjs

# The headless capability registry must cover every registered
# command exactly once and agree with the transport route table.
echo "[verify] headless capability registry contract"
node scripts/headless-capability-check.cjs

# Live-process smoke for the headless transport (验收闭环: 启动并
# 测活软件进程). The script resolves the bin under target/<triple>/debug or
# target/debug itself; it fails hard in CI when no binary exists and skips
# locally with an explicit WARN (local runs are not delivery evidence).
echo "[verify] headless live smoke"
node scripts/headless-smoke.cjs

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
 # f682940e-class guard: the
    # headless bin lives in src-tauri, which this branch never compiled, so a
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

# r12-wave-i D-003.1: the lock-wait instrument compiles only under
# --features db-lock-metrics (observation builds, never shipping). This
# pass keeps it live code rather than dead cfg text: --all-targets
# type-checks the feature-gated test modules too (f682940e-class guard),
# and the test run exercises the gated probe/holder tests once.
echo "[verify] db-lock-metrics feature pass (observation-only instrument)"
cargo check -p resin-core --features db-lock-metrics --all-targets --quiet
cargo test -p resin-core --features db-lock-metrics --lib --quiet

echo "[verify] pnpm build (tsc + vite)"
npx tsc -b
npx vite build

# GUI half of the f682940e-class guard (
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
echo "[verify] AGENTS.md size guard (32KiB budget)"
agents_bytes=$(wc -c < AGENTS.md)
if [ "$agents_bytes" -gt 32768 ]; then
  echo "FAIL: AGENTS.md is ${agents_bytes}B - over the 32KiB project_doc_max_bytes budget; move detail into docs/agents/ and keep a pointer"
  exit 1
fi
echo "[verify] AGENTS.md size OK: ${agents_bytes}B <= 32768B"
echo "[verify] strategy_service.rs line guard (4000-line budget)"
ssvc_lines=$(wc -l < crates/resin-core/src/strategy_service.rs)
if [ "$ssvc_lines" -gt 4000 ]; then
  echo "FAIL: strategy_service.rs is ${ssvc_lines} lines - over the 4000-line module budget; split write-surface or read-surface helpers into a sibling module before growing further"
  exit 1
fi
echo "[verify] strategy_service.rs size OK: ${ssvc_lines} lines <= 4000"
echo "[verify] ipc manifest guard"
node scripts/ipc-manifest-check.cjs
echo "[verify] vitest isolation guard"
node scripts/vitest-isolation-guard.cjs
echo "[verify] license field consistency (spec D-06)"
node scripts/license-field-check.cjs
echo "[verify] bilingual README alignment (spec D-06)"
node scripts/readme-lang-check.cjs
echo "[verify] upstream router integrity (spec D-C3.9)"
node scripts/upstream-router-check.cjs
echo "[verify] contracts (mode-a contract gate, ADR-0068)"
node scripts/mode-a-contract-check.cjs

# r12-wave-i D-003.2 + D-007.1: trigger-line instruments must declare their
# three fields at birth (obs-env / zero-read / evidence) - a machine gate,
# not a convention.
echo "[verify] instrument declaration gate"
node scripts/instrument-declaration-check.cjs

# The headless capability registry must cover every registered
# command exactly once and agree with the transport route table.
echo "[verify] headless capability registry contract"
node scripts/headless-capability-check.cjs

# C7 (r12-wave-b): markdownlint coverage in the PRE-MERGE gate. docs-lint.yml
# fires on pull_request + push-to-main only; GitButler branch merges never
# open a PR, so doc changes previously got their first lint on main. Run the
# same markdownlint-cli2 (pinned devDep 0.14.0) with the same config + glob
# set here so verify is the single pre-merge gate.
echo "[verify] markdownlint (same config+globs as docs-lint.yml)"
npx markdownlint-cli2 "AGENTS.md" "README.md" "README_CN.md" "CHANGELOG.md" "CONTRIBUTING.md" "SECURITY.md" "CODE_OF_CONDUCT.md" "PULL_REQUEST_TEMPLATE.md" ".github/ISSUE_TEMPLATE/*.md" "docs/**/*.md"

# Live-process smoke for the headless transport (验收闭环: 启动并
# 测活软件进程). The script resolves the bin under target/<triple>/debug or
# target/debug itself; it fails hard in CI when no binary exists and skips
# locally with an explicit WARN (local runs are not delivery evidence).
echo "[verify] headless live smoke"
node scripts/headless-smoke.cjs

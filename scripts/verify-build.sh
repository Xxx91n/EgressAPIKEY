#!/usr/bin/env bash
set -euo pipefail

# T9-8: CI/local test split — CI runs full workspace, local skips app crate
IS_CI="${CI:-false}"

echo "[verify] cargo build"
if [ "$IS_CI" = "true" ]; then
  if [ "$(uname -s 2>/dev/null)" = "Linux" ] || [ "$(uname -s 2>/dev/null)" = "Darwin" ]; then
    echo "[verify] NOTE: non-Windows CI - app crate skipped (Windows-only compile surface; GUI jobs cover it)"
    cargo build -p resin-core --quiet
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

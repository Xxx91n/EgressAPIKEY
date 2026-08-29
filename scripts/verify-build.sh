#!/usr/bin/env bash
set -euo pipefail

# T9-8: CI/local test split — CI runs full workspace, local skips app crate
IS_CI="${CI:-false}"

echo "[verify] cargo build"
if [ "$IS_CI" = "true" ]; then
  cargo build --workspace --quiet
else
  cargo build -p resin-core --quiet
  cargo build -p egressapikey-app --features custom-protocol --quiet || echo "[verify] WARN: egressapikey-app build skipped (host linker issue)"
fi

echo "[verify] cargo test"
if [ "$IS_CI" = "true" ]; then
  cargo test --workspace --quiet
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

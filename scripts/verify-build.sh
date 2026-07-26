#!/usr/bin/env bash
set -euo pipefail
echo "[verify] cargo build (workspace)"
cargo build --workspace --quiet
echo "[verify] cargo test (workspace)"
cargo test --workspace --quiet
echo "[verify] pnpm build (tsc + vite)"
npx tsc -b
npx vite build
echo "[verify] pnpm test (vitest)"
npx vitest run
echo "[verify] i18n coverage"
node scripts/i18n-check.cjs
echo "VERIFY OK"

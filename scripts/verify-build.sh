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
# G5: sidecar binary existence guard. The GUI runtime needs a vendored
# Resin Go binary at src-tauri/binaries/resin-<triple>. On the dev host this is
# fetched by scripts/fetch_resin.{sh,ps1}. On CI each GUI job fetches it BEFORE
# this script runs. We verify presence here so a GUI release never ships with
# an empty binary stub - a missing sidecar manifests at runtime only after the
# user launches the GUI and the first /healthz probe times out, which is the
# worst possible failure shape. We only require one binary (the dev host
# triple) because verify runs on ubuntu-latest where controllers only test
# the backend matrix, not the GUI triple.
BYTES=$(find src-tauri/binaries -type f \( -name 'resin-*' -o -name 'resin*.exe' \) -printf '%s\n' 2>/dev/null | head -n1)
if [ -z "$BYTES" ] || [ "$BYTES" -lt 1048576 ]; then
  echo "[verify] WARN: no Resin sidecar binary >1MB in src-tauri/binaries/ (skipping strict guard on the verify job; GUI jobs fetch it themselves)"
fi
echo "VERIFY OK"

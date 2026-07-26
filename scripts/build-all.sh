#!/usr/bin/env bash
# Local reproduction of the CI matrix. Builds the headless backend for the
# host OS, then (optionally) the Tauri GUI if tauri-cli is installed.
# Output lands in release/<os>-backend/ and src-tauri/target/.../bundle/.
set -euo pipefail

OS="$(uname -s)"
case "$OS" in
  Linux*)  NAME=linux ;;
  Darwin*) NAME=macos ;;
  MINGW*|MSYS*|CYGWIN*) NAME=windows ;;
  *) echo "unknown host: $OS"; exit 1 ;;
esac

echo "[build-all] host=$NAME"
echo "[build-all] frontend"
npx tsc -b
npx vite build

echo "[build-all] backend headless"
cargo build --release -p resin-core
mkdir -p release
STAGE="release/${NAME}-backend"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp target/release/resin-core* "$STAGE/" 2>/dev/null || true
cp -r docs "$STAGE/docs" 2>/dev/null || true
tar -czf "${STAGE}.tar.gz" "$STAGE"
echo "[build-all] backend artifact: ${STAGE}.tar.gz"

if command -v tauri >/dev/null 2>&1; then
  echo "[build-all] gui (tauri)"
  tauri build
else
  echo "[build-all] tauri-cli not installed; skipping GUI build (CI runs it)"
fi
echo "BUILD-ALL OK"

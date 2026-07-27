#!/usr/bin/env bash
# Local reproduction of the CI matrix. Builds the headless backend for the
# host OS, then the Tauri GUI (installer + portable binary) if tauri-cli is
# available. Output lands in:
#   release/<os>-backend/         - resin-core headless binary + docs
#   release/<os>-backend.tar.gz
#   release/<os>-gui/             - Tauri installer bundles (msi, nsis setup,
#                                   deb, AppImage, dmg) + portable binary
#   release/<os>-gui.tar.gz
set -euo pipefail

OS="$(uname -s)"
case "$OS" in
  Linux*)  NAME=linux ;;
  Darwin*) NAME=macos ;;
  MINGW*|MSYS*|CYGWIN*) NAME=windows ;;
  *) echo "unknown host: $OS"; exit 1 ;;
esac

echo "[build-all] host=$NAME"
echo "[build-all] frontend (tsc + vite)"
npx --no-install tsc -b
npx --no-install vite build

echo "[build-all] backend headless (resin-core)"
cargo build --release -p resin-core
mkdir -p release
STAGE="release/${NAME}-backend"
rm -rf "$STAGE"
mkdir -p "$STAGE"
BINS="$(find target -maxdepth 3 -type f \( -name 'resin-core' -o -name 'resin-core.exe' \) -path '*/release/*' 2>/dev/null || true)"
for BIN in $BINS; do cp "$BIN" "$STAGE/"; done
[ -n "$BINS" ] || { echo "resin-core binary not found after build"; exit 1; }
cp -r docs "$STAGE/docs" 2>/dev/null || true
tar -czf "${STAGE}.tar.gz" "$STAGE"
echo "[build-all] backend artifact: ${STAGE}.tar.gz"

# --- GUI bundle -------------------------------------------------------------
# Produces BOTH the installer bundle AND a portable (unbundled) GUI binary.
# Installer bundles land under src-tauri/target/.../release/bundle/... and
# are copied into release/<os>-gui/. The portable binary comes from
# `tauri build --no-bundle` (Tauri v2+): a single GUI executable the user
# can drop anywhere without installation.
#
# CRITICAL: every `tauri build` invocation MUST pass --features custom-protocol.
# Without it, tauri::generate_context!() compiles with dev:true and the webview
# loads devUrl (http://localhost:1420) → ERR_CONNECTION_REFUSED on any machine
# that has no Vite dev server. See AGENTS.md section 5 + src-tauri Cargo.toml
# [features] custom-protocol forwarding. This applies to BOTH the installer
# bundle and the --no-bundle portable binary — a missing feature on EITHER
# path ships a broken GUI.

TAURI_BIN=""
if command -v tauri >/dev/null 2>&1; then
  TAURI_BIN=tauri
elif [ -x node_modules/.bin/tauri ]; then
  TAURI_BIN="node_modules/.bin/tauri"
fi

if [ -n "$TAURI_BIN" ]; then
  echo "[build-all] gui - installer bundle ($TAURI_BIN build --features custom-protocol)"
  "$TAURI_BIN" build --features custom-protocol
  echo "[build-all] gui - portable binary ($TAURI_BIN build --no-bundle --features custom-protocol)"
  "$TAURI_BIN" build --no-bundle --features custom-protocol || echo "[build-all] WARNING: --no-bundle not supported; skipping portable"

  GUI_STAGE="release/${NAME}-gui"
  rm -rf "$GUI_STAGE"
  mkdir -p "$GUI_STAGE"

  BUNDLES="$(find src-tauri/target -maxdepth 6 -type f \( \
    -name '*.msi' -o -name '*-setup.exe' -o \
    -name '*.deb' -o -name '*.AppImage' -o \
    -name '*.dmg' \) 2>/dev/null || true)"
  for f in $BUNDLES; do
    cp "$f" "$GUI_STAGE/" 2>/dev/null || true
  done

  PORT_BIN="$(find src-tauri/target -maxdepth 4 -type f \( \
    -name 'ai-api-route' -o -name 'ai-api-route.exe' \) \
    -path '*/release/*' ! -path '*/bundle/*' 2>/dev/null | head -n1 || true)"
  if [ -n "$PORT_BIN" ]; then
    # On Windows/MINGW the source is ai-api-route.exe; keep the .exe so the
    # user can double-click the staged portable binary. On Linux/macOS the
    # source has no extension, so neither does the destination.
    PORT_DEST="${NAME}-portable-gui"
    case "$NAME" in windows) PORT_DEST="${NAME}-portable-gui.exe" ;; esac
    cp "$PORT_BIN" "$GUI_STAGE/$PORT_DEST"
    echo "[build-all] portable GUI binary staged: $GUI_STAGE/$PORT_DEST"
  else
    echo "[build-all] WARNING: portable GUI binary not found"
  fi

  if [ -n "$BUNDLES" ] && [ "$(ls -A "$GUI_STAGE" 2>/dev/null)" ]; then
    tar -czf "${GUI_STAGE}.tar.gz" "$GUI_STAGE"
    echo "[build-all] GUI artifact: ${GUI_STAGE}.tar.gz"
  else
    echo "[build-all] WARNING: no GUI installer bundles found"
  fi
else
 echo "[build-all] tauri-cli not installed; skipping GUI build (install @tauri-apps/cli to build GUI locally)"
  # Fallback: produce a portable GUI exe via plain cargo. CRITICAL: the
  # custom-protocol feature MUST be passed — without it tauri's
  # generate_context! compiles with dev:true and the webview loads devUrl
  # (http://localhost:1420), giving ERR_CONNECTION_REFUSED on a user
  # machine with no Vite dev server. See AGENTS.md section 5 + src-tauri
  # Cargo.toml [features] custom-protocol forwarding.
  echo "[build-all] fallback: cargo build --release --features custom-protocol (portable GUI only)"
  cargo build --release -p ai-api-route-app --features custom-protocol
  GUI_STAGE="release/${NAME}-gui"
  rm -rf "$GUI_STAGE"
  mkdir -p "$GUI_STAGE"
  PORT_DEST="${NAME}-portable-gui"
  case "$NAME" in windows) PORT_DEST="${NAME}-portable-gui.exe" ;; esac
  PORT_BIN="$(find src-tauri/target -maxdepth 4 -type f \( \
    -name 'ai-api-route' -o -name 'ai-api-route.exe' \) \
    -path '*/release/*' ! -path '*/bundle/*' 2>/dev/null | head -n1 || true)"
  if [ -n "$PORT_BIN" ]; then
    cp "$PORT_BIN" "$GUI_STAGE/$PORT_DEST"
    echo "[build-all] portable GUI binary staged (fallback): $GUI_STAGE/$PORT_DEST"
    tar -czf "${GUI_STAGE}.tar.gz" "$GUI_STAGE"
  else
    echo "[build-all] WARNING: portable GUI binary not found after fallback cargo build"
  fi
fi

echo "BUILD-ALL OK"

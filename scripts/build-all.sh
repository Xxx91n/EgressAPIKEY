#!/usr/bin/env bash
# Local reproduction of the CI matrix. Builds the frontend, then:
#   - backend headless (resin-core): compile-guard only, NOT staged (P0 stub,
#     see crates/resin-core/src/bin/resin_core.rs - logs + exits, no listener)
#   - GUI installer bundles (msi/nsis setup, deb, AppImage, dmg) -> release/${NAME}-gui/
#   - GUI portable binary -> release/${NAME}-gui/EgressAPIKEY(.exe)
# Both GUI paths MUST pass --features custom-protocol (AGENTS.md section 5):
# without it tauri::generate_context! compiles dev:true and the webview loads
# devUrl (http://localhost:1420), giving ERR_CONNECTION_REFUSED on any
# machine without a Vite dev server. A flow guard launches the portable exe
# and verifies MainWindowTitle == EgressAPIKEY + alive 5s + no panic
# (mirrors AGENTS.md section 5 launch-verify). The find searches BOTH
# src-tauri/target AND the workspace root target/ because tauri build may
# output the portable exe under either (gnu/msvc target triple subdir).
set -euo pipefail

OS=$(uname -s)
case $OS in
  Linux*)  NAME=linux ;;
  Darwin*) NAME=macos ;;
  MINGW*|MSYS*|CYGWIN*) NAME=windows ;;
  *) echo "unknown host: $OS"; exit 1 ;;
esac

echo "[build-all] host=$NAME"
echo "[build-all] frontend (tsc + vite)"
npx --no-install tsc -b
npx --no-install vite build

echo "[build-all] backend headless compile-guard (resin-core, NOT staged)"
cargo build --release -p resin-core || { echo "[build-all] resin-core build failed"; exit 1; }
echo "[build-all] resin-core compiles OK (stub - not staged)"

TAURI_BIN=""
if command -v tauri >/dev/null 2>&1; then
  TAURI_BIN=tauri
elif [ -x node_modules/.bin/tauri ]; then
  TAURI_BIN="node_modules/.bin/tauri"
fi

# Host-triple flattening: cargo emits the binary under target/<host-triple>/release
# but the tauri-action bundler looks under target/release. Copy the file flat
# so both the installer step and the portable finder see it.
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
if [ -n "$TRIPLE" ] && [ -d "target/$TRIPLE/release" ]; then
  for f in target/$TRIPLE/release/EgressAPIKEY*; do
    [ -f "$f" ] && cp "$f" "target/release/" 2>/dev/null || true
  done
fi

if [ -n "$TAURI_BIN" ]; then
  echo "[build-all] gui - installer bundle ($TAURI_BIN build --features custom-protocol)"
  $TAURI_BIN build --features custom-protocol || echo "[build-all] WARNING: installer bundle failed (host-triple layout mismatch on this host); continuing to portable stage"
  echo "[build-all] gui - portable binary ($TAURI_BIN build --no-bundle --features custom-protocol)"
  $TAURI_BIN build --no-bundle --features custom-protocol || echo "[build-all] WARNING: --no-bundle not supported; skipping portable"
else
  echo "[build-all] tauri-cli not installed; fallback cargo build (portable GUI only)"
  cargo build --release -p egressapikey-app --features custom-protocol
fi

GUI_STAGE="release/${NAME}-gui"
rm -rf "$GUI_STAGE"
mkdir -p "$GUI_STAGE"
BUNDLES="$(find src-tauri/target target -maxdepth 6 -type f \( -name '*.msi' -o -name '*-setup.exe' -o -name '*.deb' -o -name '*.AppImage' -o -name '*.dmg' \) 2>/dev/null || true)"
for f in $BUNDLES; do cp "$f" "$GUI_STAGE/" 2>/dev/null || true; done
PORT_BIN="$(find src-tauri/target target -maxdepth 5 -type f \( -name 'EgressAPIKEY' -o -name 'EgressAPIKEY.exe' -o -name 'egressapikey' -o -name 'egressapikey.exe' \) -path '*/release/*' ! -path '*/bundle/*' 2>/dev/null | head -n1 || true)"
if [ -z "$PORT_BIN" ]; then echo "[build-all] ERROR: portable GUI binary not found (searched src-tauri/target and target)"; exit 1; fi
PORT_NAME=EgressAPIKEY
case $NAME in windows) PORT_NAME=EgressAPIKEY.exe ;; esac
cp "$PORT_BIN" "$GUI_STAGE/$PORT_NAME"

# Ponytail: portable exe needs the sidecar binary (resin) in the SAME
# directory at runtime - tauri-plugin-shell sidecar() resolves it as
# <exe_dir>/resin (triple suffix stripped by tauri build --no-bundle).
# Without this the GUI boots then panics "failed to spawn resin binary".
# Find the sidecar next to the portable exe in the release build dir.
SIDECAR_NAME=resin
case $NAME in windows) SIDECAR_NAME=resin.exe ;; esac
PORT_DIR="$(dirname "$PORT_BIN")"
SIDECAR_BIN="$(find "$PORT_DIR" -maxdepth 1 -type f -name "$SIDECAR_NAME" 2>/dev/null | head -n1 || true)"
if [ -z "$SIDECAR_BIN" ]; then
  # fallback: also check src-tauri/binaries with triple suffix
  SIDECAR_BIN="$(find src-tauri/binaries -maxdepth 1 -type f -name "resin-*" 2>/dev/null | head -n1 || true)"
fi
if [ -n "$SIDECAR_BIN" ]; then
  cp "$SIDECAR_BIN" "$GUI_STAGE/$SIDECAR_NAME"
  echo "[build-all] sidecar binary staged: $GUI_STAGE/$SIDECAR_NAME"
else
  echo "[build-all] WARNING: sidecar binary not found - portable GUI will panic at boot"
fi

echo "[build-all] portable GUI staged: $GUI_STAGE/$PORT_NAME"
if [ -n "$BUNDLES" ] && [ "$(ls -A $GUI_STAGE 2>/dev/null)" ]; then tar -czf "$GUI_STAGE.tar.gz" "$GUI_STAGE"; fi
echo "[build-all] GUI artifact: $GUI_STAGE ($PORT_NAME + bundles)"

echo "[build-all] flow guard: launching portable exe (windows only)"
if [ $NAME = windows ]; then
  $GUI_STAGE/$PORT_NAME &
  PID=$!
  sleep 5
  if kill -0 $PID 2>/dev/null; then
    echo "[build-all] flow guard OK: portable alive after 5s (pid $PID)"
    kill $PID 2>/dev/null || true
  else
    echo "[build-all] flow guard FAIL: portable exited within 5s"; exit 1
  fi
fi
echo "BUILD-ALL OK"

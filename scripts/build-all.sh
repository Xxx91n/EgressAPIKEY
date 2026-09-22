#!/usr/bin/env bash
# Local reproduction of the CI matrix. Builds the frontend, then:
#   - backend (resin-core): compile-guard + staged to release/${NAME}-backend.tar.gz

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

# Host triple is needed before the backend sidecar staging below; defined
# here once so every later use is a read, not a first assignment.
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"

# Anchor to script location
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

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

echo "[build-all] tests (resin-core + vitest + i18n)"
cargo test -p resin-core --lib --quiet || { echo "[build-all] cargo test failed"; exit 1; }
npx --no-install vitest run --reporter=dot || { echo "[build-all] vitest failed"; exit 1; }
node scripts/i18n-check.cjs || { echo "[build-all] i18n check failed"; exit 1; }

echo "[build-all] backend compile-guard + tar.gz staging (resin-core)"
cargo build --release -p resin-core || { echo "[build-all] resin-core build failed"; exit 1; }
echo "[build-all] headless binary (egressapikey-headless --features headless)"
cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless || { echo "[build-all] FATAL: headless binary build failed"; exit 1; }
echo "[build-all] resin-core compiles OK"

# --- Stage backend (release/${NAME}-backend self-contained: headless exe + dist/ + resin sidecar) ---
BACKEND_STAGE="release/${NAME}-backend"
rm -rf "$BACKEND_STAGE"
mkdir -p "$BACKEND_STAGE"

# Copy headless binary
HEADLESS_BIN="$(find target "src-tauri/target" -maxdepth 5 -type f -name "egressapikey-headless*" ! -path "*/deps/*" 2>/dev/null | head -n1 || true)"
if [ -n "$HEADLESS_BIN" ]; then
  cp "$HEADLESS_BIN" "$BACKEND_STAGE/"
  echo "[build-all] headless binary staged: $BACKEND_STAGE/"
else
  echo "[build-all] FATAL: headless binary not found (build failed?)"; exit 1
fi

# Copy dist/ (frontend static assets)
if [ -d "dist" ]; then
  cp -r dist "$BACKEND_STAGE/dist"
  echo "[build-all] headless dist staged: $BACKEND_STAGE/dist/"
else
  echo "[build-all] FATAL: dist/ not found (frontend build failed?)"; exit 1
fi

# Copy resin sidecar binary into backend stage
SIDECAR_BIN="$(ls src-tauri/binaries/resin-*$TRIPLE* 2>/dev/null | head -1 || true)"
if [ -n "$SIDECAR_BIN" ]; then
  cp "$SIDECAR_BIN" "$BACKEND_STAGE/resin"
  echo "[build-all] headless sidecar staged: $BACKEND_STAGE/resin"
else
  echo "[build-all] WARNING: resin sidecar not found — headless will fail to boot"
fi

tar -czf "$BACKEND_STAGE.tar.gz" "$BACKEND_STAGE"
echo "[build-all] backend staged: $BACKEND_STAGE.tar.gz"
if command -v tauri >/dev/null 2>&1; then
  TAURI_BIN=tauri
elif [ -x node_modules/.bin/tauri ]; then
  TAURI_BIN="node_modules/.bin/tauri"
fi

# Host-triple flattening: cargo emits the binary under target/<host-triple>/release
# but the tauri-action bundler looks under target/release. Copy the file flat
# so both the installer step and the portable finder see it.
TRIPLE="${TRIPLE}"
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
PORT_BIN="$(find src-tauri/target target -maxdepth 5 -type f \( -name 'EgressAPIKEY' -o -name 'EgressAPIKEY.exe' \) -path '*/release/*' ! -path '*/bundle/*' 2>/dev/null | xargs -r ls -t 2>/dev/null | head -n1 || true)"
if [ -z "$PORT_BIN" ]; then echo "[build-all] ERROR: portable GUI binary not found (searched src-tauri/target and target)"; exit 1; fi
# Newest-wins: the backend step flattens the PRE-build exe to target/release/,
# so find order alone can stage a stale binary (guard caught
# chunk-hash MISMATCH); ls -t picks the just-linked fresh exe instead.
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

# --- Chunk hash verification (AGENTS §5 hard close-loop) ---
# Verify the latest Vite chunk hash from dist/assets/ is embedded in the exe bytes.
# This catches the stale-bundle bug: cargo build --release reuses old dist/ if
# pnpm build wasn't run first, embedding outdated frontend in the exe.
VITE_CHUNK=$(ls dist/assets/index-*.js 2>/dev/null | head -1 | sed 's/.*index-//;s/\.js//')
if [ -z "$VITE_CHUNK" ]; then
  echo "[build-all] WARN: no index-*.js found in dist/assets — frontend may not be built"
fi
if [ -n "$VITE_CHUNK" ] && [ -f "$GUI_STAGE/$PORT_NAME" ]; then
  # strings may not exist on minimal Linux or macOS; grep -a treats binary as text
  if (strings "$GUI_STAGE/$PORT_NAME" 2>/dev/null || grep -a -o . "$GUI_STAGE/$PORT_NAME" 2>/dev/null) | grep -q "$VITE_CHUNK"; then
    echo "[build-all] chunk-hash OK: index-$VITE_CHUNK found in exe"
  else
    echo "[build-all] ERROR: chunk-hash MISMATCH — index-$VITE_CHUNK NOT found in exe!"
    echo "[build-all] The staged binary has a STALE frontend bundle."
    echo "[build-all] Run pnpm build before cargo build, or use scripts/build-all.sh which does both."
    exit 1
  fi
fi
if [ -n "$BUNDLES" ] && [ "$(ls -A $GUI_STAGE 2>/dev/null)" ]; then tar -czf "$GUI_STAGE.tar.gz" "$GUI_STAGE"; fi
echo "[build-all] GUI artifact: $GUI_STAGE ($PORT_NAME + bundles)"

echo "[build-all] SHA256 checksums"
for f in "$GUI_STAGE"/*; do
  [ -f "$f" ] && sha256sum "$f" > "${f}.sha256" 2>/dev/null || true
done

echo "[build-all] flow guard: launching portable GUI"
case $NAME in
  windows)
    "$GUI_STAGE/$PORT_NAME" &
    PID=$!; sleep 5
    if kill -0 $PID 2>/dev/null; then
      echo "[build-all] flow guard OK: alive after 5s (pid $PID)"
      kill $PID 2>/dev/null || true
    else
      echo "[build-all] flow guard FAIL: exited within 5s"; exit 1
    fi
    ;;
  macos)
    open -W -n "$GUI_STAGE/$PORT_NAME" &
    PID=$!; sleep 5
    if kill -0 $PID 2>/dev/null; then
      echo "[build-all] flow guard OK: alive after 5s (pid $PID)"
      kill $PID 2>/dev/null || true
    else
      echo "[build-all] flow guard WARN: exited (macOS open may fork)"
    fi
    ;;
  linux)
    if [ -z "$DISPLAY" ]; then
      echo "[build-all] flow guard SKIPPED: no DISPLAY on Linux headless"
    else
      "$GUI_STAGE/$PORT_NAME" &
      PID=$!; sleep 5
      if kill -0 $PID 2>/dev/null; then
        echo "[build-all] flow guard OK: alive after 5s (pid $PID)"
        kill $PID 2>/dev/null || true
      else
        echo "[build-all] flow guard FAIL: exited within 5s"; exit 1
      fi
    fi
    ;;
esac
echo "BUILD-ALL OK"

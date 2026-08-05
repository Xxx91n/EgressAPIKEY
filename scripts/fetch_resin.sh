#!/usr/bin/env bash
# Fetch the Resin Go sidecar binary for the current host triple and drop it
# into src-tauri/binaries/resin-<triple> so Tauri can bundle it as a sidecar.
# See docs/MEMORY_REUSE_DECISION.md path A.
set -euo pipefail
REPO="Resinat/Resin"
REL="v1.2.0"
BU="https://github.com/${REPO}/releases/download/${REL}"
triple="$(rustc -vV | awk '/^host:/ {print $2}')"
case "$triple" in
  *-windows-msvc|*-windows-gnu) asset="resin-windows-amd64.zip"; ext=".exe" ;;
  aarch64-apple-darwin) asset="resin-darwin-arm64.tar.gz"; ext="" ;;
  x86_64-apple-darwin) asset="resin-darwin-amd64.tar.gz"; ext="" ;;
  aarch64-*-linux-*) asset="resin-linux-arm64.tar.gz"; ext="" ;;
  *-linux-*) asset="resin-linux-amd64.tar.gz"; ext="" ;;
  *) echo "Resin sidecar unsupported triple: $triple" >&2; exit 2 ;;
esac
destdir="src-tauri/binaries"
mkdir -p "$destdir"
tmpdir="$(mktemp -d)"
trap "rm -rf $tmpdir" EXIT
echo "downloading $BU/$asset"
if command -v curl >/dev/null 2>&1; then curl -fsSL "$BU/$asset" -o "$tmpdir/asset"; else wget -q -O "$tmpdir/asset" "$BU/$asset"; fi
if [[ "$asset" == *.zip ]]; then
  if command -v unzip >/dev/null 2>&1; then
    unzip -q "$tmpdir/asset" -d "$tmpdir/expanded"
  else
    # Windows / any host without unzip: fall back to PowerShell Expand-Archive.
    powershell -NoProfile -Command "Expand-Archive -Path '$tmpdir/asset' -DestinationPath '$tmpdir/expanded' -Force"
  fi
  srcbin="$(find "$tmpdir/expanded" -type f -name 'resin*.exe' | head -n1)"
else
  mkdir -p "$tmpdir/expanded"
  tar -xzf "$tmpdir/asset" -C "$tmpdir/expanded"
  srcbin="$(find "$tmpdir/expanded" -type f -name "resin" | head -n1)"
fi
cp "$srcbin" "$destdir/resin-${triple}${ext}"
echo "fetched resin sidecar -> $destdir/resin-${triple}${ext}"

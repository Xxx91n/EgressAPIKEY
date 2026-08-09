# Local build + stage pipeline for Windows (ADR-0019 T2-9)
# PowerShell equivalent of build-all.sh: builds frontend + backend + GUI,
# stages artifacts to release/windows-gui/, generates SHA256 checksums,
# and reads the Resin sidecar version from the upstream manifest.
# Usage: pwsh -NoProfile -File scripts/build-all.ps1
$ErrorActionPreference="Stop"
$ProgressPreference="SilentlyContinue"

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

# --- Read version from manifest ---
$MANIFEST="$PSScriptRoot\..\docs\RESIN_UPSTREAM_MANIFEST.yaml"
if(-not (Test-Path $MANIFEST)) { Write-Error "manifest not found: $MANIFEST"; exit 2 }
$manifestContent = Get-Content $MANIFEST -Raw
$versionMatch = [regex]::Match($manifestContent, 'version:\s*"([^"]+)"')
if(-not $versionMatch.Success) { Write-Error "cannot parse version from manifest"; exit 2 }
$RESIN_VERSION=$versionMatch.Groups[1].Value
Write-Host "[build-all] Resin upstream version: $RESIN_VERSION"

# --- 1. Frontend (tsc + vite) ---
Write-Host "[build-all] frontend (tsc + vite)"
npx --no-install tsc -b
if($LASTEXITCODE -ne 0) { Write-Error "tsc -b failed"; exit 1 }
npx --no-install vite build
if($LASTEXITCODE -ne 0) { Write-Error "vite build failed"; exit 1 }

# --- 2. Backend headless compile-guard ---
Write-Host "[build-all] backend headless compile-guard (resin-core)"
cargo build --release -p resin-core
if($LASTEXITCODE -ne 0) { Write-Error "resin-core build failed"; exit 1 }
Write-Host "[build-all] resin-core compiles OK"

# --- 3. Host triple ---
$triple = (& rustc -vV | Select-String "^host:" | ForEach-Object { ($_ -split "\s+")[1] })
Write-Host "[build-all] host triple: $triple"

# --- 4. GUI build (portable + installer) ---
$TAURI_BIN = $null
if(Get-Command tauri -ErrorAction SilentlyContinue) { $TAURI_BIN = "tauri" }
elseif(Test-Path "node_modules/.bin/tauri") { $TAURI_BIN = "node_modules/.bin/tauri" }

if($TAURI_BIN) {
  Write-Host "[build-all] GUI installer bundle ($TAURI_BIN build --features custom-protocol)"
  & $TAURI_BIN build --features custom-protocol 2>&1 | ForEach-Object { Write-Host $_ }
  if($LASTEXITCODE -ne 0) { Write-Host "[build-all] WARNING: installer bundle failed (host-triple layout mismatch); continuing to portable stage" }
  Write-Host "[build-all] GUI portable binary ($TAURI_BIN build --no-bundle --features custom-protocol)"
  & $TAURI_BIN build --no-bundle --features custom-protocol 2>&1 | ForEach-Object { Write-Host $_ }
  if($LASTEXITCODE -ne 0) {
    Write-Host "[build-all] WARNING: --no-bundle not supported on this tauri-cli; running cargo fallback build"
    cargo build --release -p egressapikey-app --features custom-protocol
    if($LASTEXITCODE -ne 0) { Write-Error "[build-all] cargo fallback build failed"; exit 1 }
  }
} else {
  Write-Host "[build-all] tauri-cli not installed; fallback cargo build (portable GUI only)"
  cargo build --release -p egressapikey-app --features custom-protocol
  if($LASTEXITCODE -ne 0) { Write-Error "cargo build failed"; exit 1 }
}

# --- 5. Stage artifacts ---
$GUI_STAGE = "release/windows-gui"
if(Test-Path $GUI_STAGE) { Remove-Item $GUI_STAGE -Recurse -Force }
New-Item -ItemType Directory -Force -Path $GUI_STAGE | Out-Null

# Copy installer bundles (msi, setup.exe)
$bundles = Get-ChildItem -Path "src-tauri/target","target" -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Extension -in ".msi",".exe" -and $_.FullName -match "release" -and $_.FullName -notmatch "bundle" -and $_.FullName -notmatch "[\\/]deps[\\/]" -and $_.Name -in @("EgressAPIKEY.exe", "EgressAPIKEY-setup.exe", "EgressAPIKEY_*.msi") }
foreach($f in $bundles) { Copy-Item $f.FullName "$GUI_STAGE/" -Force; Write-Host "[build-all] staged bundle: $($f.Name)" }

# Find and copy portable GUI binary
$portBin = Get-ChildItem -Path "src-tauri/target","target" -Recurse -File -ErrorAction SilentlyContinue | Where-Object { ($_.Name -eq "EgressAPIKEY.exe") -and $_.FullName -match "release" -and $_.FullName -notmatch "bundle" -and $_.FullName -notmatch "[\\/]deps[\\/]" } | Select-Object -First 1
if(-not $portBin) { Write-Error "[build-all] ERROR: portable GUI binary not found"; exit 1 }
Copy-Item $portBin.FullName "$GUI_STAGE/EgressAPIKEY.exe" -Force
Write-Host "[build-all] portable GUI staged: $GUI_STAGE/EgressAPIKEY.exe"

# Copy sidecar binary (resin) next to portable exe
$sidecarSrc = Get-ChildItem -Path "src-tauri/binaries" -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match "^resin-" } | Select-Object -First 1
if($sidecarSrc) {
  Copy-Item $sidecarSrc.FullName "$GUI_STAGE/resin.exe" -Force
  Write-Host "[build-all] sidecar staged: $GUI_STAGE/resin.exe"
} else {
  Write-Host "[build-all] WARNING: sidecar binary not found - portable GUI will panic at boot"
}

# --- 6. SHA256 checksums ---
$checksumFile = "$GUI_STAGE/SHA256.txt"
$hashes = @()
foreach($f in Get-ChildItem $GUI_STAGE -File) {
  $hash = (Get-FileHash $f.FullName -Algorithm SHA256).Hash
  $hashes += "$hash  $($f.Name)"
  Write-Host "[build-all] SHA256 $($f.Name): $hash"
}
$hashes | Set-Content $checksumFile -Encoding UTF8
Write-Host "[build-all] checksums written to $checksumFile"

# --- 7. Flow guard: launch portable exe, verify alive 5s ---
Write-Host "[build-all] flow guard: launching portable exe"
$proc = Start-Process -FilePath "$GUI_STAGE/EgressAPIKEY.exe" -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 5
if(-not $proc.HasExited) {
  Write-Host "[build-all] flow guard OK: portable alive after 5s (pid $($proc.Id))"
  Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
} else {
  Write-Error "[build-all] flow guard FAIL: portable exited within 5s (exit code $($proc.ExitCode))"
  exit 1
}

Write-Host "BUILD-ALL OK"

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

# --- 2. Backend headless build (resin-core + headless binary) ---
Write-Host "[build-all] backend headless build (resin-core)"
cargo build --release -p resin-core
if($LASTEXITCODE -ne 0) { Write-Error "resin-core build failed"; exit 1 }
Write-Host "[build-all] resin-core compiles OK"
Write-Host "[build-all] headless binary (egressapikey-headless)"
cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless
if($LASTEXITCODE -ne 0) { Write-Error "[build-all] FATAL: headless binary build failed"; exit 1 }

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


# --- Chunk hash verification (AGENTS §5 hard close-loop) ---
# Verify the latest Vite chunk hash from dist/assets/ is embedded in the exe bytes.
# This catches the stale-bundle bug: cargo build --release reuses old dist/ if
# pnpm build wasn't run first, embedding outdated frontend in the exe.
$chunkFiles = Get-ChildItem "dist/assets/index-*.js" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($chunkFiles) {
    $chunkHash = $chunkFiles.BaseName -replace 'index-', ''
    $exeBytes = [System.IO.File]::ReadAllBytes("$GUI_STAGE/EgressAPIKEY.exe")
    $exeStr = [System.Text.Encoding]::ASCII.GetString($exeBytes)
    if ($exeStr -match [regex]::Escape($chunkHash)) {
        Write-Host "[build-all] chunk-hash OK: index-$chunkHash found in exe"
    } else {
        Write-Error "[build-all] ERROR: chunk-hash MISMATCH — index-$chunkHash NOT found in exe! Stale bundle."
        exit 1
    }
} else {
    Write-Host "[build-all] WARN: no index-*.js found in dist/assets"
}
# Copy sidecar binary (resin) next to portable exe
$sidecarSrc = Get-ChildItem -Path "src-tauri/binaries" -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match "^resin-" } | Select-Object -First 1
if($sidecarSrc) {
  Copy-Item $sidecarSrc.FullName "$GUI_STAGE/resin.exe" -Force
  Write-Host "[build-all] sidecar staged: $GUI_STAGE/resin.exe"
} else {
  Write-Host "[build-all] WARNING: sidecar binary not found - portable GUI will panic at boot"
}

# --- 5b. Stage headless backend (release/windows-backend/ self-contained) ---
$BACKEND_STAGE = "release/windows-backend"
if(Test-Path $BACKEND_STAGE) { Remove-Item $BACKEND_STAGE -Recurse -Force }
New-Item -ItemType Directory -Force -Path $BACKEND_STAGE | Out-Null

# Copy headless binary
$headlessBin = Get-ChildItem -Path "src-tauri/target","target" -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -eq "egressapikey-headless.exe" -and $_.FullName -match "release" -and $_.FullName -notmatch "[\\\\/]deps[\\\\/]" } | Select-Object -First 1
if($headlessBin) {
  Copy-Item $headlessBin.FullName "$BACKEND_STAGE/egressapikey-headless.exe" -Force
  Write-Host "[build-all] headless binary staged: $BACKEND_STAGE/egressapikey-headless.exe"
} else {
  Write-Error "[build-all] FATAL: headless binary not found (build failed?)"; exit 1
}

# Copy dist/ (frontend static assets)
if(Test-Path "dist") {
  Copy-Item "dist" "$BACKEND_STAGE/dist" -Recurse -Force
  Write-Host "[build-all] headless dist staged: $BACKEND_STAGE/dist/"
} else {
  Write-Error "[build-all] FATAL: dist/ not found (frontend build failed?)"; exit 1
}

# Copy resin sidecar binary
$sidecarBackend = Get-ChildItem -Path "src-tauri/binaries" -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match "^resin-" } | Select-Object -First 1
if($sidecarBackend) {
  Copy-Item $sidecarBackend.FullName "$BACKEND_STAGE/resin.exe" -Force
  Write-Host "[build-all] headless sidecar staged: $BACKEND_STAGE/resin.exe"
} else {
  Write-Host "[build-all] WARNING: resin sidecar binary not found - headless will fail to boot resin"
}
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

# Fetch the Resin Go sidecar binary for the current host triple and drop it
# into src-tauri/binaries/resin-<triple>{.exe} so Tauri can bundle it as a
# sidecar. We download from the upstream GitHub release tag.
# See docs/MEMORY_REUSE_DECISION.md path A.
# Read version + repo from docs/RESIN_UPSTREAM_MANIFEST.yaml (ADR-0017 T2-6)
$ErrorActionPreference="Stop"
$ProgressPreference="SilentlyContinue"
$MANIFEST="$PSScriptRoot\..\docs\RESIN_UPSTREAM_MANIFEST.yaml"
if(-not (Test-Path $MANIFEST)) { Write-Error "manifest not found: $MANIFEST"; exit 2 }
$manifestContent = Get-Content $MANIFEST -Raw
$repoMatch = [regex]::Match($manifestContent, 'repo:\s*"([^"]+)"')
$versionMatch = [regex]::Match($manifestContent, 'version:\s*"([^"]+)"')
if(-not $repoMatch.Success -or -not $versionMatch.Success) { Write-Error "cannot parse repo/version from manifest"; exit 2 }
$REPO=$repoMatch.Groups[1].Value
$REL=$versionMatch.Groups[1].Value
$BU="https://github.com/$REPO/releases/download/$REL"
$triple = & rustc -vV | Select-String "^host:" | ForEach-Object { ($_ -split "\s+")[1] }
$asset=$null; $ext=""
if($triple -match "windows-(msvc|gnu)$"){ $asset="resin-windows-amd64.zip"; $ext=".exe" }
elseif($triple -match "darwin-arm64$"){ $asset="resin-darwin-arm64.tar.gz"; $ext="" }
elseif($triple -match "darwin-(amd64|x86)"){ $asset="resin-darwin-amd64.tar.gz"; $ext="" }
elseif($triple -match "linux-(gnu|musl)-arm64$"){ $asset="resin-linux-arm64.tar.gz"; $ext="" }
elseif($triple -match "linux-(gnu|musl)$"){ $asset="resin-linux-amd64.tar.gz"; $ext="" }
else{ Write-Error "Resin sidecar unsupported triple: $triple"; exit 2 }
$destDir="src-tauri/binaries"
if(-not (Test-Path $destDir)){ New-Item -ItemType Directory -Force -Path $destDir | Out-Null }
$tag=[guid]::NewGuid().ToString("N").Substring(0,8)
$temp="$env:TEMP\resin-$tag.zip"
$expand="$env:TEMP\resin-expand-$tag"
Write-Host "downloading $BU/$asset"
Invoke-WebRequest -Uri "$BU/$asset" -OutFile $temp -UseBasicParsing
if($asset.EndsWith(".zip")){
  Expand-Archive -Path $temp -DestinationPath $expand -Force
  $srcBin=Get-ChildItem $expand -Recurse -Filter "resin*.exe" | Select-Object -First 1
  Copy-Item $srcBin.FullName "$destDir/resin-$triple$ext" -Force
}else{
  New-Item -ItemType Directory -Force -Path $expand | Out-Null
  tar -xzf $temp -C $expand
  $srcBin=Get-ChildItem $expand -Recurse -Filter "resin" | Select-Object -First 1
  Copy-Item $srcBin.FullName "$destDir/resin-$triple" -Force
}
Remove-Item $temp,$expand -Recurse -Force -ErrorAction SilentlyContinue
Write-Host "fetched resin sidecar -> $destDir/resin-$triple$ext"

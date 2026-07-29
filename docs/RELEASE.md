# Release pipeline

CI produces artifact groups landing in release/ (gitignored; GitHub Release):

| Name | Job | Output |
|---|---|---|
| windows-gui | gui matrix (windows-latest) | MSI + NSIS + portable ai-api-route.exe |
| linux-gui | gui matrix (ubuntu-latest) | deb + AppImage + portable ai-api-route |
| macos-gui | gui matrix (macos-latest) | dmg + portable ai-api-route |
| backend | backend matrix (per OS) | <os>-backend.tar.gz |

## Portable variant

Every GUI OS ships BOTH installer bundles AND the drop-and-run portable binary. The portable binary requires resin.exe alongside (build-all.sh stages both).

## iOS-class desktop

iPadOS cannot run Tauri; macOS dmg (arm64) is the Apple-silicon sibling.

## Local reproduction

```bash
bash scripts/build-all.sh
```

The release portable exe is what users get (debug exe shows ERR_CONNECTION_REFUSED without Vite dev server).

# Release pipeline

CI produces five artifact groups that land in release/ (gitignored; published to the GitHub Release):

| Name | Job | Output |
|---|---|---|
| windows-gui | gui matrix (windows-latest) | Tauri MSI + NSIS installer |
| linux-gui | gui matrix (ubuntu-latest) | Tauri deb + AppImage |
| macos-gui | gui matrix (macos-latest, universal) | Tauri dmg (and .app) |
| windows-backend | backend matrix | release/windows-backend.tar.gz (resin-core binary) |
| linux-backend | backend matrix | release/linux-backend.tar.gz |
| macos-backend | backend matrix | release/macos-backend.tar.gz |

## iOS-class desktop target

iPadOS cannot run a Tauri desktop shell. The Apple-silicon desktop sibling is the macOS universal dmg, which supports arm64. This is documented in CI and README; do not promise an iPad build.

Verify locally with bash scripts/build-all.sh before pushing.

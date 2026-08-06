# ADR-0019: build-all.ps1 + release standardization

Date: 2026-08-06
Status: ACCEPTED
Context: Only build-all.sh exists (Git Bash only). No PowerShell equivalent. Version hardcoded in tauri.conf.json. No SHA256 checksum on artifacts.

## Decision

Create scripts/build-all.ps1 as a PowerShell-native equivalent of
build-all.sh so the developer can run a full build-stage pipeline without
Git Bash. The .ps1 script:
1. Reads RESIN_UPSTREAM_MANIFEST.yaml for sidecar version (ADR-0017)
2. Runs tsc + vite build (frontend)
3. Runs cargo build --release --features custom-protocol (backend + GUI)
4. Stages portable exe + sidecar to release/windows-gui/
5. Computes SHA256 of each staged artifact -> release/sha256.txt
6. Smoke-launches the staged portable exe (title + pid alive check)

Linux/macOS GUI builds remain a CI matrix job (cross-compiling Tauri +
Go sidecar on Windows is not practical). The local build is the Windows
guaranteed artifact; CI produces the cross-platform artifacts.

backend headless tarball: cargo build --release -p resin-core produces
the library crate; a standalone headless binary is not yet wired. If
needed later, add a bin target in resin-core Cargo.toml + a stage step.

## Alternatives rejected

- (B) maintain only build-all.sh: requires Git Bash, friction for PS-only devs.
- (C) full local cross-compile: Tauri needs per-target rustc + native deps;
  Go binary needs per-OS build. Not practical on a single Windows host.

## Tradeoffs

- +80 lines new PowerShell script
- Developer can build + stage entirely in PowerShell
- Linux/macOS artifacts still need CI (manual workflow_dispatch)

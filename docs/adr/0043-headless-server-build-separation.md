# ADR-0043: Headless Server Build Separation — Source Relocation, required-features Gating, CI Job Split

> Status: ACCEPTED
> Date: 2026-08-17
> Supersedes: none (builds on ADR-0009 CLI-GUI alignment status, ADR-0017 Resin upstream manifest)
> Sources: atomcode research T17-Q1 (dual-binary architecture, 11 sources), T17-Q2 (dual-runtime frontend, 20 sources), T17-Q3 (build/packaging separation, 16 sources), T17-Q4 (documentation structure, 18 sources)

## Context

EgressAPIKEY ships a Tauri 2 desktop GUI (EgressAPIKEY.exe) and a headless HTTP server binary (egressapikey-headless.exe) from the same src-tauri crate. The headless binary is the "npm install" path for VPS deployment without the Tauri desktop shell. The current state has three problems:

1. **Tauri bundler auto-discovery hazard**: headless.rs lives in src-tauri/src/bin/ with no required-features gating. The Tauri bundler stage-2 disk scan (tauri issue #15325) can include the headless binary in the GUI installer package — it never compiled that binary, causing a "Failed to copy binary" error or shipping an unwanted binary.
2. **Release artifact nesting**: headless exe lives in release/windows-gui/npm-server/ — a non-GUI artifact nested inside the GUI channel directory. Industry practice (codeg, rssh) is to make headless an independent release artifact.
3. **CI backend job builds wrong target**: the backend CI job builds resin-core (a library crate), not the actual egressapikey-headless binary that users deploy.

## Decision

### 1. Source relocation + required-features gating (rssh pattern + tauri #15325 workaround)

Move headless.rs from src-tauri/src/bin/headless.rs to src-tauri/src/headless_main.rs. Cargo.toml [[bin]] entry points to the new path with required-features=["headless"]. Add [features] headless = [] to Cargo.toml.

This is a **defense-in-depth** combination:
- Source relocation bypasses the bundler stage-2 disk scan of src/bin/ (rssh documented fix)
- required-features prevents cargo from compiling the headless binary in GUI builds (tauri PR #14379 fix, stage-1)
- Both are needed because PR #15427 (stage-2 fix) may not be in all tauri-cli versions in the wild

### 2. Release artifact migration

Headless artifacts move from release/windows-gui/npm-server/ to release/windows-backend/. The backend directory is self-contained: egressapikey-headless.exe + dist/ (frontend static assets) + resin.exe (Go sidecar). This mirrors codeg build-server job producing codeg-server-<os>-<arch>.tar.gz with codeg-server + web/ + codeg-mcp.

### 3. CI backend job retarget

The CI backend job changes from "cargo build --release -p resin-core per-OS" to "cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless". Independent rust-cache key (server-<triple>). Artifact: <os>-backend.tar.gz.

### 4. Build script fatal enforcement

build-all.ps1 and build-all.sh change the headless build from non-fatal warning to fatal exit 1. The headless binary is an independent release artifact, not an optional GUI附属.

## Alternatives considered

- **(B) Only fix directory + gating, CI unchanged**: smaller diff but CI efficiency not optimal (headless shares GUI job webkit2gtk dependency install unnecessarily)
- **(C) Only add gating, keep nested directory**: minimal change but violates industry convention of independent release artifacts

## Consequences

- **Positive**: headless binary never pollutes GUI installer; CI backend job is lean (no GUI system deps); release/ follows codeg convention; build failure caught immediately
- **Negative**: two cargo invocations needed (GUI + headless separately) — but they share the same lib crate so incremental compilation covers most of the cost
- **Irreversible?**: No — moving the source back is trivial if a future Tauri version fully fixes the auto-discovery bug
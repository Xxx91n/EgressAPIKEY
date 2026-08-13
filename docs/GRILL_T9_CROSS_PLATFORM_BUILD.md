# GRILL T9 — Cross-Platform Build/Compile/Test Compatibility

> **Grill session**: 2026-08-14, Q1-Q11 all decided.
> **ADR**: docs/adr/0030-cross-platform-build-compat.md
> **Status**: Planning complete, ready for execution.

## Problem Statement

The project's build/compile/test pipeline has Windows-only hardcoding in build-all.sh, no test step in the local build script, no flow guard on Linux/macOS, no SHA256 checksums locally, inconsistent binary naming between CI and local, and fetch_resin.sh has cwd-dependent relative paths.

## Decisions (Q1-Q11)

| Q | Decision | Option |
|---|----------|--------|
| Q1 | Keep build-all.ps1 as-is, only fix build-all.sh | C |
| Q2 | build-all.sh gains test steps; verify-build.sh gets CI/local split | A |
| Q3 | Three-platform flow guard, Linux uses DISPLAY shield | A |
| Q4 | Local backend tar.gz staging, bin name from Cargo.toml | A |
| Q5 | verify-build.sh uses CI env var + uname dual detection | A |
| Q6 | Unify to uppercase EgressAPIKEY, sync CI matrix.bin | A |
| Q7 | fetch_resin.sh uses SCRIPT_DIR to anchor repo root | A |
| Q8 | build-all.sh generates SHA256 checksums | A |
| Q9 | Confirm actual filename then unify (confirmed: bin name = EgressAPIKEY) | A |
| Q10 | Unify headless binary name to egressapikey-headless (hyphen) | A |
| Q11 | Scope confirmed, no additions needed | — |

## Execution Plan (9 Steps)

### T9-1: fetch_resin.sh SCRIPT_DIR anchoring
**File**: scripts/fetch_resin.sh
**Change**: Add SCRIPT_DIR + REPO_ROOT at top. Replace MANIFEST path and destdir to use $REPO_ROOT.
**Verify**: Run from repo root AND from src-tauri/ subdir — both must resolve MANIFEST and write to src-tauri/binaries/.
**Test**: Bash unit: bash scripts/fetch_resin.sh from src-tauri/ cwd — verify binary lands in src-tauri/binaries/resin-*.

### T9-2: build-all.sh platform detection hardening
**File**: scripts/build-all.sh
**Change**: Anchor with SCRIPT_DIR. Add MINGW64* to case pattern for explicitness.
**Verify**: build-all.sh output on Linux/macOS/Windows-Git-Bash all show correct NAME.
**Test**: Dry-run on each OS (CI matrix or manual).

### T9-3: build-all.sh unified binary naming
**File**: scripts/build-all.sh
**Change**:
- PORT_NAME: EgressAPIKEY (non-Windows) / EgressAPIKEY.exe (Windows) — already correct, keep.
- SIDECAR_NAME: resin / resin.exe — already correct, keep.
- Remove egressapikey (lowercase) and egressapikey.exe from the find search (L66) — only search EgressAPIKEY and EgressAPIKEY.exe.
- Backend bin name: read from crates/resin-core/Cargo.toml [[bin]] name = resin-core.
**Verify**: find search matches exactly one file per OS.
**Test**: build-all.sh produces release/${NAME}-gui/EgressAPIKEY (or .exe).

### T9-4: build-all.sh test step insertion
**File**: scripts/build-all.sh
**Change**: After frontend build (L28) + before GUI build (L51), insert:
- cargo test -p resin-core --lib --quiet
- npx --no-install vitest run --reporter=dot
- node scripts/i18n-check.cjs
Each with failure exit on non-zero.
**Verify**: Running build-all.sh now fails if any test fails.
**Test**: Add a deliberate test failure → build-all.sh exits non-zero; revert → passes.

### T9-5: build-all.sh backend tar.gz staging
**File**: scripts/build-all.sh
**Change**: After resin-core build (L31), add staging:
- BACKEND_STAGE="release/${NAME}-backend"
- Find resin-core binary (from target/ or target/$TRIPLE/)
- Find egressapikey-headless binary
- Copy both to BACKEND_STAGE/
- tar -czf "$BACKEND_STAGE.tar.gz"
**Verify**: release/${NAME}-backend.tar.gz exists after build.
**Test**: Verify tar.gz contents contain resin-core + egressapikey-headless.

### T9-6: build-all.sh three-platform flow guard
**File**: scripts/build-all.sh
**Change**: Replace L96-107 with:
- windows: direct launch + pid alive check
- macos: open -W -n + pid alive check (WARN if fork makes it unreliable)
- linux: check DISPLAY env var; if empty → WARN SKIP, else launch + pid alive check
**Verify**: Windows still works; Linux without DISPLAY prints WARN not error.
**Test**: CI Linux runner (no DISPLAY by default) — verify WARN output.

### T9-7: build-all.sh SHA256 checksum generation
**File**: scripts/build-all.sh
**Change**: After staging, before flow guard, insert sha256sum for each file in GUI_STAGE/.
**Verify**: release/${NAME}-gui/*.sha256 files exist.
**Test**: Verify .sha256 content matches sha256sum -c.

### T9-8: verify-build.sh CI/local test split
**File**: scripts/verify-build.sh
**Change**: Replace with CI-aware version:
- CI=true: cargo build --workspace + cargo test --workspace (full)
- local (no CI): cargo build -p resin-core + cargo test -p resin-core --lib (skip app crate DLL issue)
- Both: pnpm build + vitest + i18n:check
**Verify**: CI (CI=true) runs full workspace; local runs resin-core only.
**Test**: CI=true bash scripts/verify-build.sh vs bash scripts/verify-build.sh on Windows.

### T9-9: CI ci.yml portable job matrix.bin fix
**File**: .github/workflows/ci.yml
**Change**: In gui-portable job matrix:
- bin: egressapikey → bin: EgressAPIKEY
- bin: egressapikey.exe → bin: EgressAPIKEY.exe
**Verify**: CI portable job cp step finds the file.
**Test**: CI run on all three platforms (manual dispatch).

## Non-Goals

- build-all.ps1 NOT touched (Q1-C decision)
- No new dependencies (Ponytail)
- No test framework changes (vitest/cargo test config stays)
- No tauri.conf.json changes

## Verification Matrix

| Check | Command | Platform |
|-------|---------|----------|
| build-all.sh runs clean | bash scripts/build-all.sh | Win/Mac/Linux |
| verify-build.sh CI mode | CI=true bash scripts/verify-build.sh | Linux (CI) |
| verify-build.sh local mode | bash scripts/verify-build.sh | Windows |
| fetch_resin.sh from subdir | cd src-tauri && bash ../scripts/fetch_resin.sh | All |
| Backend tar.gz staged | ls release/*-backend.tar.gz | All |
| SHA256 generated | ls release/*-gui/*.sha256 | All |
| Flow guard Windows | portable alive 5s | Windows |
| Flow guard Linux no DISPLAY | WARN not error | Linux headless |
| CI portable bin name | cp finds EgressAPIKEY | CI matrix |
| Binary naming consistency | EgressAPIKEY everywhere | All |

## Execution Order

T9-1 -> T9-2 -> T9-3 -> T9-4 -> T9-5 -> T9-6 -> T9-7 -> T9-8 -> T9-9

Each step is a self-contained commit with its own verification. No step depends on a later step.

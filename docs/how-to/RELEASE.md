# Release pipeline

CI (`workflow_dispatch` on `.github/workflows/ci.yml`) produces artifact groups landing in `release/` (gitignored locally) and attaches them to a single Draft GitHub Release (with `prerelease` controlled via the workflow dispatch input):

| Name | Job | Output |
|---|---|---|
| windows-gui | `gui` + `gui-portable` + `release-assets` | MSI + NSIS (`*-setup.exe`) + `windows-gui-portable.zip` (`EgressAPIKEY.exe` + `resin.exe`) |
| linux-gui | `gui` + `gui-portable` + `release-assets` | deb + AppImage + `linux-gui-portable.tar.gz` (`EgressAPIKEY` + `resin`) |
| macos-gui | `gui` + `gui-portable` + `release-assets` (+ opt-in `gui-intel`) | arm64 dmg + `macos-gui-portable-arm64.tar.gz` (`EgressAPIKEY` + `resin`); x86_64 dmg opt-in via `include_intel_mac` |
| backend | `backend` + `release-assets` (+ opt-in `backend-intel`) | `linux-backend.tar.gz`, `windows-backend.tar.gz`, `macos-arm64-backend.tar.gz` (+ opt-in `macos-x86_64-backend.tar.gz`) |
| checksums | `release-assets` | `SHA256SUMS.txt` covering all Draft Release assets (installers/bundles, portable archives, and headless backend tarballs) |

## Portable variant

Every GUI OS ships BOTH installer bundles AND the drop-and-run portable archive. The portable binary requires `resin` (`resin.exe` on Windows) alongside (`build-all.sh` and CI `gui-portable` + `release-assets` stage and verify both).

## iOS-class desktop

iPadOS cannot run Tauri; macOS dmg (arm64) is the Apple-silicon sibling.

## Local reproduction

```bash
bash scripts/build-all.sh
```

The release portable exe is what users get (debug exe shows ERR_CONNECTION_REFUSED without Vite dev server).

## Release checklist

Before promoting a build (and after any Resin sidecar version bump):

- Package versions match across `package.json`, workspace `Cargo.toml` (`Cargo.lock`), `src-tauri/tauri.conf.json`, and `scripts/npm/package.json` (`release-assets` enforces this against `vX.Y.Z` tags).
- `bash scripts/verify-build.sh` is green in CI `verify`, including its `contracts` sub-step:
  `scripts/mode-a-contract-check.cjs` boots the fetched Resin sidecar and
  re-establishes the four-scenario byte-level contract (mixed-port
  503 `NO_AVAILABLE_NODES` + SOCKS5 `05 00`; http-only refuses SOCKS5 with
  `05 ff`; socks5-only refuses HTTP with 403 `ENDPOINT_CAPABILITY_DISABLED`)
  plus the identity-attribution cases (HTTP Basic and SOCKS5
  UserPass credentials land in the request log attributed to their account
  even on the failure path). Baseline table + hand recipe:
  `tests/fixtures/t17-contract/README.md`. A contract failure after a Resin bump is a
  release blocker, not a retest-later item.
- CI `verify` job green on the release commit, plus `macos-test` (tier-2 release gate), `e2e` (Playwright), and `release-assets` asset/checksum verification green on the `workflow_dispatch` run (test evidence is the CI run, per the CI-only build policy; real-window portable verification runs via `webview-smoke.yml`).

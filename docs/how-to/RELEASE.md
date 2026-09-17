# Release pipeline

CI produces artifact groups landing in release/ (gitignored; GitHub Release):

| Name | Job | Output |
|---|---|---|
| windows-gui | gui matrix (windows-latest) | MSI + NSIS + portable EgressAPIKEY.exe |
| linux-gui | gui matrix (ubuntu-latest) | deb + AppImage + portable EgressAPIKEY |
| macos-gui | gui matrix (macos-latest) | dmg + portable EgressAPIKEY |
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

## Release checklist

Before promoting a build (and after any Resin sidecar version bump):

- `bash scripts/verify-build.sh` is green, including its `contracts` sub-step:
  `scripts/mode-a-contract-check.cjs` boots the fetched Resin sidecar and
  re-establishes the ADR-0068 D4 four-scenario byte-level contract (mixed-port
  503 `NO_AVAILABLE_NODES` + SOCKS5 `05 00`; http-only refuses SOCKS5 with
  `05 ff`; socks5-only refuses HTTP with 403 `ENDPOINT_CAPABILITY_DISABLED`)
  plus the ticket-17 identity-attribution cases (HTTP Basic and SOCKS5
  UserPass credentials land in the request log attributed to their account
  even on the failure path). Baseline table + hand recipe:
  `repro/t17-contract/README.md`. A contract failure after a Resin bump is a
  release blocker, not a retest-later item.
- CI verify job green on the release commit (the same gate runs there - test
  evidence is the CI run, per the 2026-09-04 CI-only build policy).

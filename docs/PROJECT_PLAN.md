# Project Plan - EgressAPIKEY

> Phased delivery (P0-P9). Living document.

## Completed

- P0 Repo foundation — DONE
- P1 Core proxy kernel (deprecated DTOs after Path A) — DONE
- P4 Path A G1-G5 (sidecar, ResinClient, Ghost, 5-tab, CI/CD) — DONE
- P6 CI/CD multi-platform — DONE
- P7 Documentation — DONE

## Current / Next

- P8 Polish: keyboard nav, WebDAV backup, per-view caching — IN PROGRESS
- P9 Release 0.1: tag v0.1.0 — PENDING

## Success criteria

- bash scripts/verify-build.sh exits 0
- cargo build + cargo test + pnpm build + pnpm test pass
- release/ contains Windows/Linux/macOS GUI (installer + portable) + backend tarballs

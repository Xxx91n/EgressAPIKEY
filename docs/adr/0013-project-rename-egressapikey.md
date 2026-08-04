# ADR-0013: Project rename — ai-api-route -> EgressAPIKEY

Date: 2026-08-05
Status: ACCEPTED
Decision Type: Naming / branding

## Context

The user observed: "ai-api-route sounds like API key route — but we do IP
route, with optimization strategies targeting AI." The software is an IP
egress router for AI API key pools, not an API key router.

## Decision

Rename the project to **EgressAPIKEY**.

Collision audit (2026-08-05, 5 sources):
- GitHub repositories (exact name search): 0 results
- GitHub broad search (egress+api+key in name/description): 12 results, none named EgressAPIKEY
- npm registry: 0 results
- crates.io: 0 results
- DuckDuckGo exact match: 0 results

No collision detected. The name is unique across major package registries.

## Consequences

- Repository, Cargo.toml, tauri.conf.json, package.json, README, AGENTS.md
  all need updating to the new name.
- The Tauri bundle identifier (com.ai-api-route.desktop) changes to
  (com.egressapikey.desktop) — user data directory migrates on next launch.
- DNS/domain registration is the user's responsibility (not blocking).
- The old name persists in git history; no rewrite needed.

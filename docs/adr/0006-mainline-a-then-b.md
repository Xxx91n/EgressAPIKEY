# ADR-0006: Q5 completion criterion = mainline A (function closure) then mainline B (release standardization)

Date: 2026-08-02
Status: ACCEPTED
Decision Type: Completion criterion + work sequencing

## Context

Q5 of the grill-with-docs session asked: what is the completion criterion for
this project right now? Two mainlines were proposed:

- Mainline A (function closure first): "software does not look like a toy" =
  wire the last GUI-to-real-Resin-data breakpoints; release can wait.
- Mainline B (release closure first): ship a traceable v0.1.0 = fill
  release/ with real Linux/macOS GUI + backend tarballs, then tag.

## Decision

Mainline A first, then mainline B. The two are sequential, not parallel.
A is the precondition for B: shipping a tagged release while the GUI still
has hollow data breakpoints would ship a toy with a version number, which
contradicts the user is repeated "not a toy" demand.

Q6 confirmed mainline A scope = four items:
1. platform_snapshot wired to real GET /platforms/{id}/routable-view
   (HANDOFF_PATH_A_CONTINUATION next-deep-work-1).
2. subscription import closed-loop audit-then-fix (the user test URL
   https://link123.52pokemon66.cc/... must produce visible nodes end-to-end).
3. (Item 4 in Q6 numbering) A4-3 interceptor validated against the REAL
   Resin sidecar (not just mockito) -- two distinct (api_key, endpoint) pairs
   produce two distinct X-Resin-Account headers -> two distinct leases ->
   two distinct egress IPs, proven with the live binary.
4. (Item 5 in Q6 numbering) tray context menu i18n syncs immediately when
   the GUI locale changes (no restart, no half-beat lag).

## What is NOT in mainline A

- Node-per-account editor (YAGNI: Resin v1.1.2 has no node-level account
  CRUD endpoint; user has not named this requirement).
- build-all.sh externalized sidecar fetch (hygiene, not a closure gap).
- Keyboard nav / per-view caching (user never named these; speculative).
- v0.1.0 tag (mainline B, after A closes).

## Relationship to prior ADRs

- ADR-0003 authorized the A4-3 interceptor (Q4 answer). This ADR-0006 item 3
  extends ADR-0003 is closed loop from mockito-only to live-sidecar e2e.
- ADR-0005 Q5 (subscription management depth) overlaps this ADR-0006 item 2.
  item 2 is the audit-then-fix path; ADR-0005 Q5 is the broader feature
  (enable/disable/interval/LastError). They converge: item 2 verifies the
  existing import path; ADR-0005 Q5 extends it.
- ADR-0005 is PROPOSED. This ADR-0006 depends on ADR-0005 being implemented
  eventually but does not block items 1/3/4 of mainline A.

## Consequences

- Mainline B (release standardization) does not start until all four
  mainline A items have closed-loop tests passing.
- Each mainline A item ships with: cargo test + vitest closed-loop, release
  exe rebuilt, codegraph sync, AGENTS.md section appended, commit+push.
- Glossary terms: "closed-loop test", "live-sidecar e2e", "routable-view"
  defined in docs/glossary.md (this ADR pair creates it).

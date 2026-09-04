# ADR-0066: Platform actions not integrated (reset-to-default / rebuild-routable-view / preview-filter)

- **Status**: ACCEPTED
- **Date**: 2026-09-05 (round5 T22 / 裂痕 #2 子项 — 波次 W4)
- **Supersedes**: nothing; narrows ADR-0062 D3/D4 — coverage rows R09/R13/R14
  move from 空白-pending-T22 to 故意不接 with the reasons recorded here.
- **Research basis**: `resin/internal/api/handler_platform.go:170,186,201`
  (upstream handlers, read verbatim) plus the service semantics at
  `resin/internal/service/control_plane_platform.go:546,570,700`; reports/03
  §5 Platform-Actions row; T13 `docs/architecture/RESIN_API_COVERAGE.md`
  rows R09/R13/R14.

## Context

Upstream Resin registers three platform action endpoints that are neither
resource reads nor resource writes:

- `POST /api/v1/platforms/{id}/actions/reset-to-default` (R13,
  handler_platform.go:171) — `ControlPlaneService::ResetPlatformToDefault`
  recompiles the platform from env defaults (`defaultPlatformConfig`) and
  replaces it in the node pool;
- `POST /api/v1/platforms/{id}/actions/rebuild-routable-view` (R14,
  handler_platform.go:187) — `ControlPlaneService::RebuildPlatformView`
  re-derives Resin's internal routable node view (`Pool.RebuildPlatform`);
- `POST /api/v1/platforms/preview-filter` (R09, handler_platform.go:202) —
  `ControlPlaneService::PreviewFilter` dry-runs a regex/region filter spec
  and returns the matching node list without mutating any state.

T13 left all three as 空白 pending this ticket's classification. None is a
high-frequency shell path and none has a UI ask, so the only question was
whether "unwired" is an oversight or a decision. Per ADR-0062 D2.5,
deliberateness requires a written rationale — this ADR supplies it.

## Decision

**D1 — All three actions are deliberate non-adoptions this round.** The
shell never constructs these paths: no ResinClient method, no IPC command,
no TS wrapper is added. Coverage rows R09/R13/R14 move 空白 → 故意不接.

**D2 — Rationale per action.**

- `reset-to-default` (R13): the rewrite target is the platform's compiled
  config — exactly the region_filters surface the L2 whitebox authors and
  `strategy_apply` maintains (ADR-0036 single write entry, ADR-0057
  diff-then-skip). Calling it from the shell would mutate L3 behind the
  whitebox's back and make the authoritative snapshot report drift the user
  cannot resolve from the GUI (the whitebox still holds the pre-reset
  value). The shell-side "reset" is editing the whitebox entry, never a
  hidden upstream reset.
- `rebuild-routable-view` (R14): the routable view is Resin-internal state
  that Resin itself rebuilds on node changes; the shell has no visibility
  into pool internals and no scenario where the externally observable
  surface (leases, endpoints, node list) needs a forced rebuild.
  diff-then-skip already keeps the desired→live mapping converged on every
  apply — a manual rebuild affordance would be a no-op dressed as a feature.
- `preview-filter` (R09): read-only dry-run, so it breaks nothing — but the
  capability already exists shell-side: the ADR-0054 authoritative snapshot
  merges desired filters against live nodes in memory, and the reconcile
  preview surfaces the resulting "what would change" view. Wiring the
  upstream dry-run this round would duplicate that surface. It stays the
  ONE wiring candidate of the three for a later round (launcher ticket
  T23, e.g. an "Apply Preview" affordance on filter editing).

**D3 — Deliberateness criterion (generalizes ADR-0062 D3).** An upstream
write action is deliberately not wired when invoking it from the shell
would (1) mutate L3 state that the L2 whitebox authors (authority bypass),
(2) duplicate a capability the shell already owns, or (3) expose an
operation with no GUI ask. A read-only action (dry-run) is deliberately
unwired only under (2)/(3); it remains a wiring candidate otherwise.

## Consequences

- Positive: the last three undecided rows of the coverage table are
  classified; "why isn't X wired" now has a written answer for every
  故意不接 row.
- Cost: none at runtime (zero code paths). The classification must be
  revisited if the shell ever grows a filter-preview UI (T23) or a
  whitebox-reset flow; per ADR-0062 D5 the coverage rows move in the same
  commit as any such change.
- Neutral: a future preview-filter wiring does NOT reopen this ADR — D2
  already designates it the sanctioned candidate; the wiring ticket cites
  this ADR and flips the R09 row.

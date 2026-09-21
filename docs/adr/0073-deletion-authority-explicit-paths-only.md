# ADR-0073: Deletion authority - desired-state entries leave only through explicit deletion

Status: ACCEPTED
- **Date**: 2026-09-20 (r11-wave-c grill D-002; execution ticket R11-16)
- **Extends**: [ADR-0056](0056-reconcile-platform-create-semantics.md) (apply never deletes whitebox entries), [ADR-0054](0054-reconciliation-loop-completion.md) (one-way reconcile), [ADR-0058](0058-generation-observedgeneration-echo.md) (drift phases)

## Context

The wave-B independent audit (2026-09-20) carried over an existing risk in
`syncPlatformStrategy` (`src/views/PlatformsView.tsx:120`): every
strategy-field save filters the whitebox platform list down to platforms
present in the Resin live list before `strategy_config_put` + apply - a
silent, unprompted auto-prune of the L2 desired state driven by an L3 read.
If the live list is momentarily empty (sidecar restart window), one
unrelated field edit can wipe the entire desired platform set.

Research (atomcode session 0c787d51, 2026-09-20; k8s.io declarative-config
and ArgoCD auto_sync / sync-options read directly; argo-cd#23645 as the
empty-protection-bypass incident) cross-verified the industrial stance: no
mature declarative system treats "absent from live" as a deletion signal
for desired state. Deletion is a form of prune, and prune everywhere ships
behind explicit opt-in, scope limiting, a confirmation gate, and
empty-set protection.

## Decision

1. **Deletion authority belongs to explicit deletion paths only.** A
   desired-state entry (whitebox strategy/ports rows) may be removed only
   by a hand edit to the whitebox file or, in the future, a confirmed
   cleanup action. No save/sync/apply path may drop entries based on a
   read of the live state; `strategy_config_put` accepts the caller's
   complete desired state verbatim.
2. **"Missing on live" is a drift phase, not a deletion signal.** The
   existing Drifted phase plus `reconcile_now` re-assertion is the whole
   remedy: apply rebuilds missing live platforms (ADR-0056); the desired
   state is never pruned to match a transient live read.
3. **Empty-live protection (allowEmpty semantics).** Any write-back that
   feeds on a live-state list - now or in the future - must hard-reject
   when that list is empty. An empty live read is an "engine not
   trustworthy" window, never a "user deleted everything" fact
   (argo-cd#23645 lesson: the protection must be simple and unbypassable).
4. **A future cleanup entry, if ever needed, is preview-style
   Prune=confirm**: the reconcile preview lists "desired-but-not-live -
   remove the desired entry?" and only an explicit confirmation performs
   the deletion through deletion semantics + an audit op + the backup
   ring. Not built in this round; hook left on record.

## Consequences

- R11-16 removes the `cleanedPlatforms` filter and locks two regression
  invariants: (a) a strategy-field sync never shrinks the platform entry
  set; (b) a live-list-fed write-back hard-rejects on an empty list.
- The L2 backup ring + hash-chained audit log remain the recovery path
  for pre-existing data loss; they are not a license for implicit pruning
  (prevention beats recovery).
- The industrial safety preconditions for automated deletion (explicit
  opt-in switch, scope limiting, confirmation gate, empty-set protection,
  soft-delete as the fifth) are the review template for any future
  pruning feature.

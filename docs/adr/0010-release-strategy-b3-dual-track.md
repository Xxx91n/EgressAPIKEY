# ADR-0010: Release workflow strategy — B-3 (local instantly-available + CI manual-only)

Date: 2026-08-03
Status: ACCEPTED
Decision Type: Release pipeline + CI trigger policy

## Context

Q7 of the grill-with-docs session asked how to sequence mainline B (the
release-closure phase of the project, per ADR-0006 which closed mainline A
in the prior commits). Three axes were proposed:

- B-1 (local one-shot full cross-platform release on the host) — local
  host can only build native-target binaries; Tauri bundling .deb/.dmg
  requires a native Linux/macOS runner. So a pure B-1 sail on this Windows
  host cannot produce Linux GUI or macOS dmg. It can produce Windows GUI
  and backend, plus cross-target --no-bundle portable Linux/macOS binaries
  without installer wrapping.
- B-2 (GitHub Actions release pipeline投产 + tag v0.1.0) — the existing
  .github/workflows/ci.yml already has the 5-job matrix (verify / e2e /
  backend headless / gui installer / gui-portable). It never ran to
  completion under a release tag. B-2 = make it the real release pipeline,
  attach artifacts to a v0.1.0 GitHub Release.
- B-3 (dual-track: local instantly-available every commit + CI投产)
  — the user's hard constraint AGENTS.md section 5 mandates a freshly
  staged exe after every source commit (local), and the project must
  also have a proper release pipeline (CI). The two are not
  substitutable; the user explicitly选择了 B-3.

Additional user constraint: the GitHub Actions release pipeline must
NOT run on every push. It must be manually triggered by the user from
the Actions tab. Reason: the user does not want CI to burn minutes on
every code-only push when the local release staging already covers the
"修改完用户立刻能打开" requirement; CI pari-release is reserved for
intentional release runs.

## Decision

B-3 dual-track, with CI gated to manual dispatch.

### Track 1 — Local instantly-available (this and every code commit)

`scripts/build-all.sh` on the Windows host already produces the
Windows GUI line (installer + portable + sidecar). It stays as-is.
The constraint AGENTS.md section 5 already mandates this: every source
commit must rebuild + stage `release/windows-gui/ai-api-route.exe`
with the live Vite chunk hash embedded + smoke launch verified.

What the local track produces today (and continues to produce after
this ADR):
- `release/windows-gui/ai-api-route.exe` (portable GUI, 11.47MB)
- `release/windows-gui/resin.exe` (Gold Resin sidecar binary)
- optional installer bundles (.msi / -setup.exe) when the Tauri bundler
  target layout cooperates with this host's MinGW triple
- the backend headless binary is compile-guarded here but NOT staged
  (ADR-0009: it is a stub with no network listener; staging a stub
  tarball would misrepresent the project state. The CI matrix in Track
  2 stages the backend tarball for completeness, with the stub banner
  in --help so a downstream user sees the real status.)

What Track 1 cannot do on a Windows host (deferred to Track 2):
- macOS .dmg (needs macOS runner, code signing on arm64/darwin)
- Linux .deb + .AppImage (needs native Linux runner with system deps)
- Native cross-target portable binaries -- can be built via
  `--target x86_64-unknown-linux-gnu` on this host, but without
  tauri-action bundling on a native Linux runner we cannot wrap the
  portable into a proper .deb/.AppImage. The skip is honest: the
  user-facing cross-platform release lives in Track 2.

### Track 2 — GitHub Actions release pipeline, manual dispatch only

Add to .github/workflows/ci.yml (a new release workflow file is
cleaner, but for now we gate the existing file):
```yaml
on:
  workflow_dispatch:
    inputs:
      tag:
        description: 'Git tag for the release (e.g. v0.1.0)'
        required: true
        type: string
```
Remove the old `push:` and `pull_request:` triggers entirely. The
5-job matrix (verify / e2e / backend headless / gui installer /
gui-portable) only runs when the user opens the Actions tab and
clicks "Run workflow" with a tag input.

Rationale per A7 user constraint: "GitHub Actions必须要求用户自己去
actions里面手动触发构建，要不然每次 push 都会构建". The push trigger on
`codex/*` branches was burning CI minutes on every agent commit; the
local track already covers the per-commit release-exe requirement, so
the CI matrix is freed to run only on intentional releases.

Tag handling: the workflow_dispatch `tag` input is read by the
release-upload step (softprops/action-gh-release) so the GitHub Release
is created at the tag the user typed. The user is responsible for first
`git tag v0.1.0 && git push origin v0.1.0` (their local push), then
trigger the workflow with the same tag string in the input.

## What is NOT in scope

- The ADR-0001 VPS path (headless parity refactor, deferred per ADR-0009)
  is NOT part of mainline B. It requires the dualkit/helmor dispatcher
  multi-crate refactor and its own ADR. mainline B publishes the Desktop
  path cross-platform GUI artifacts; the VPS headless refactor is a
  later grill phase.
- Removing the resin-core backend tarball from CI matrix entirely —
  keep it: the tarball is currently a stub binary but it preserves the
  pipeline for the future VPS refactor. The stub banner in --help is
  honest about its state.

## Acceptance criteria

- [ ] `ci.yml` `on:` block contains only `workflow_dispatch` (no
      push, no pull_request) — verified by `grep` show zero `push:`
      or `pull_request:` keys at the top level.
- [ ] Track 1 is unchanged; `scripts/build-all.sh` on this Windows host
      produces `release/windows-gui/ai-api-route.exe` + resin.exe with
      a smoke launch (verified commit-by-commit per AGENTS.md section 5).
- [ ] The CI matrix is invokable once on a v0.1.0 tag: open Actions tab,
      Run workflow with tag `v0.1.0`, observe all 5 jobs green; the
      GitHub Release at v0.1.0 carries 5 artifact groups.

## Consequences

- CI minutes stop draining on every `codex/*` push. The user pays for
  CI only when they want a release.
- Track 1 stays as the immediate-visibility path: code changes are testable
  by the user by opening the freshly staged exe without waiting for CI.
- A future grill phase that wants PR-blocking CI (e.g. before merging to
  main) re-introduces a pull_request trigger on main ONLY (not codex/*).
  The ADR leaves that decision to the phase that needs it.

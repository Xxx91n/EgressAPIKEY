# Pull Request

## Summary

<!-- What does this PR change, and why? Link related issues with "Fixes #<issue>". -->

## Change type

- [ ] Bug fix (behavior change limited to the fix)
- [ ] New behavior (frontend / IPC / resin-core)
- [ ] Refactor (behavior-preserving)
- [ ] Docs / community files only
- [ ] Build, CI, or scripts

## Verification

<!-- Mark only what you actually ran. CI (scripts/verify-build.sh) covers cargo build + tests, tsc + vite build, vitest, and i18n coverage. -->

- [ ] `bash scripts/verify-build.sh` passes locally, or the CI run is green
- [ ] New behavior carries tests (resin-core unit test / vitest / e2e, per repo conventions)
- [ ] User-visible strings go through i18n and exist in all 18 base locales (`pnpm i18n:check`)
- [ ] License-touching changes ran `node scripts/license-field-check.cjs`
- [ ] [AGENTS.md](AGENTS.md) updated in the same commit when structure or conventions changed
- [ ] No secrets or credentials in code, logs, or fixtures
- [ ] Shell/webview changes were rebuilt with `pnpm build` before testing (stale-bundle guard)

---
date: 2026-08-19 | branch: doc-fix | status: PLAN v2 (boxing exp merged, pending user)
explore: codegraph (86 files 1452 nodes) + grill 18+35 skill + industry 32 sources + neat-freak + writing-for-agents
ref: D:/Aworker/crx/boxing completed 2 commits (3-layer split + CI governance)
---

# Document Governance Plan: grill Workflow Blind Spot Fix

## 1. Root Cause

The grill suite (ask-matt index) main flow designed clear doc output paths:

### grill 18 skill complete workflow suite (ask-matt confirmed)

Main flow (idea -> ship):

1. /grill-with-docs: interview -> CONTEXT.md (root) + docs/adr/
2. /to-spec: spec.md -> .scratch/<slug>/ (gitignored) or issue tracker
3. /to-tickets: NN-slug.md -> .scratch/<slug>/issues/ (gitignored)
4. /implement: from ticket, fresh context

On-ramps:

- /triage: raw issues -> agent-ready issues -> main flow at /implement
- /diagnosing-bugs: hard bugs -> regression test -> /improve-codebase-architecture
- /wayfinder: huge foggy -> decision tickets -> /to-spec -> main flow

Standalone:

- /grill-me: stateless interview
- /grilling: interview primitive
- /resolving-merge-conflicts: merge/rebase hunk by hunk
- /research: background agent -> cited markdown
- /to-questionnaire: questions for someone else

Vocabulary underneath:

- /domain-modeling: domain language -> CONTEXT.md + ADR
- /codebase-design: module shape vocabulary (depth/seam/adapter)

setup:

- /setup-matt-pocock-skills: docs/agents/{issue-tracker,domain,triage-labels}.md

### doc output contract

| grill output | active location | archive location |
|---|---|---|
| CONTEXT.md | root | -- |
| ADR | docs/adr/ | -- |
| spec.md | .scratch/ | post-commit -> docs/history/specs/ |
| tickets | .scratch/ | done -> docs/history/tickets/ |
| handoff | (temp) | docs/history/handoffs/ |
| research | docs/research/ | -- |
| docs/agents/*.md | docs/agents/ | -- |

### what actually happened

Three categories of output got dumped into docs/ instead:

1. GRILL_T* execution plans (22 files ~3000 lines) - session-temp or done phases
2. HANDOFF_* handoff docs (10 files ~1200 lines) - session artifacts
3. User prompt archive (USER_PROMPT_ARCHIVE + T9_MASTER_GOAL_PROMPT) - not engineering docs

docs/ now: 9364 lines / ~95 files. 49 ADRs are compliant. 63 root-level md files (non-adr).
No .archive/, no .scratch/, no docs/agents/, no docs/history/, no CHANGELOG.md.

### Codex hard limit (from boxing project research)

- project_doc_max_bytes = 32 KiB (Codex default, configurable)
- Current AGENTS.md: 230637 bytes = 225 KiB, 7x over limit
- Over-limit: Codex silently truncates deepest file; AGENTS.md sections 10-29 likely never read by agent
- boxing project (286 lines/39218 bytes) identified this; we are 7x worse
- Industry consensus (atomcode): 60-300 line sweet spot, 150-200 start splitting
- What to keep: copy-pasteable exact commands, rules diverging from language defaults, Always/Ask first/Never boundaries
- What to split out: not-every-task needs it -> separate file + pointer

## 2. Industry Four-Layer Separation Model (32 sources)

| Layer | Framework | Manages | Location |
|---|---|---|---|
| Content organization | Diataxis | text classification | docs/ by quadrant |
| Architecture description | C4 model | diagram zoom | docs/architecture/ |
| Decision records | ADR/MADR | why archived | docs/adr/NNNN-*.md |
| Agent instructions | AGENTS.md | how agent works | root AGENTS.md <500 lines |
| Milestone timeline | Keep a Changelog | what shipped | CHANGELOG.md |

Key finding (arXiv 2026 ETH paper): agent instruction files should only contain commands and pointers, not architecture descriptions; inline overviews increase cost +20% without benefit.

### neat-freak "reduce before add" principle

- One-time plan docs (22 GRILL_T*): after merging effective content into formal docs, files are delete candidates
- Don't add new docs before evaluating if existing ones can be trimmed or deleted

### writing-for-agents information hierarchy (3 levels)

1. In-file step: what agent does every run (keep in AGENTS.md)
2. In-file reference: rules agent consults on demand (can keep in AGENTS.md)
3. Disclosed reference: only some branches need it -> separate file + pointer (phase sections)

Branching test: inline what every branch needs, pointer what only some branches reach.
Our phase sections (10-29) are only needed when reviewing project history -> should be disclosed.

### boxing project confirmation

Three-layer separation is the user's own governance decision, NOT a grill skill built-in standard.
Grill has partial answers (CONTEXT.md / ADR / docs/agents/), but doesn't cover
"Critical Lessons Learned" and "Performance Anti-Patterns" engineering convention placement.

## 3. Migration Plan (git mv, preserves history)

### Unchanged

- docs/adr/ (49 files, MADR style, compliant)
- .github/ (workflows, ISSUE_TEMPLATE)
- AGENTS.md (root, needs slimming but position unchanged)

### Moves

| Current | Target | Count | Reason |
|---|---|---|---|
| docs/GRILL_T*.md | docs/history/phases/ | 22 | done phase plans, archive |
| docs/HANDOFF_*.md | docs/history/handoffs/ | 10 | session artifacts |
| docs/USER_PROMPT_ARCHIVE.md | docs/history/prompts/ | 1 | not engineering doc |
| docs/T9_MASTER_GOAL_PROMPT.md | docs/history/prompts/ | 1 | same |
| docs/T4_*.md, T7-*.md | docs/history/phases/ | 3 | done |
| docs/ARCHITECTURE.md | docs/architecture/ | 1 | Diataxis explanation |
| docs/CONTEXT.md | root CONTEXT.md | 1 | grill design position |
| docs/glossary.md | docs/reference/ | 1 | Diataxis reference |
| docs/RESIN_ROUTING_*.md | docs/research/ | 1 | research |
| docs/KEY_ENDPOINT_*.md | docs/research/ | 2 | research |
| docs/PROTOCOL_WEIGHT_*.md | docs/research/ | 1 | research |
| docs/MEMORY_REUSE_DECISION.md | docs/architecture/ | 1 | arch decision |
| docs/REFACTOR_PLAN*.md | docs/architecture/ | 3 | arch refactor plan |
| docs/HEADLESS_*.md | docs/how-to/ | 2 | ops guide |
| docs/RELEASE.md | docs/how-to/ | 1 | release guide |
| docs/PONYTAIL_DEBT*.md | root PONYTAIL_DEBT_LEDGER.md | 2 | active debt ledger |
| docs/PROJECT_PLAN.md | -> CHANGELOG.md | 1 | milestones to changelog |
| docs/GRILL_ISSUES_BACKLOG.md | docs/history/phases/ | 1 | done backlog |

### New files

- CHANGELOG.md (Keep a Changelog 2.0 template, six types + [Unreleased])
- docs/architecture/README.md (entry point, links to ARCHITECTURE.md + MEMORY_REUSE_DECISION.md)
- docs/history/README.md ("HISTORICAL ARCHIVE - not current truth", reader x purpose)
- docs/agents/README.md (reader x purpose per writing-for-agents)
- .markdownlint-cli2.jsonc (lint config)
- .github/workflows/docs-lint.yml (markdownlint + lychee + size warning)
- .gitignore add .scratch/

## 4a. AGENTS.md Slimming (most critical fix)

Current AGENTS.md: 852 lines / 230637 bytes (7x over 32 KiB Codex limit)

### Slimming strategy (writing-for-agents progressive disclosure)

Keep (agent uses every run):

- context-mode routing block (ctx_* tool routing)
- CodeGraph conventions
- i18n conventions (full key coverage, 18 locale)
- Test conventions (cargo test, vitest, e2e)
- CI/CD conventions (build-all, release staging, chunk hash verify)
- Git hygiene (push after every commit, LF policy)
- Security redlines (IPC validation, SSRF guard, sidecar SSRF)
- Pointers to docs/ ("architecture see docs/architecture/, decisions see docs/adr/")

Migrate to docs/architecture/ (reference level, some branches need):

- Runtime state (current section 11) -> docs/architecture/runtime-state.md
- Storage locations -> docs/architecture/storage.md
- Sidecar lifecycle -> docs/architecture/sidecar.md
- Ghost safety net -> docs/architecture/ghost-safety.md

Migrate to CHANGELOG.md (process artifacts, milestones):

- IPC retarget (sections 15-20)
- Resin webhook (section 16)
- CI/CD release pipeline (section 17)
- P9-P24 phases (sections 18-29)
- R1-R2 refactor (section 23)

Expected: AGENTS.md from 230637 bytes -> ~30000 bytes (under 32 KiB)

## 5. Risks and Mitigation

### Risks

1. Other parallel fix branches may reference docs/GRILL_T\* / docs/HANDOFF_\* paths
   - Mitigation: git mv preserves history, all paths are rename not delete
   - Mitigation: check other branches for references first (grep)

2. AGENTS.md slimming may lose current truth
   - Mitigation: phase sections migrate to CHANGELOG.md, not deleted; AGENTS.md keeps pointers
   - Mitigation: AGENTS.md top block (context-mode, CodeGraph, i18n, test, CI/CD) stays, these are agent commands not milestones

3. docs/CONTEXT.md -> root CONTEXT.md may conflict with existing file
   - Mitigation: root already has CONTEXT.md with full glossary (verified)
   - Mitigation: merge if needed, don't overwrite

4. docs/history/ docs may be misread by LLM as current truth
   - Mitigation: docs/history/README.md marks "HISTORICAL ARCHIVE - not current truth"
   - Mitigation: same as ADR superseded status, kept but not trusted

### Untouched

- Source code (no .rs/.ts/.tsx changes)
- ADR content (49 ADRs unchanged, position confirmed)
- docs/adr/ structure (unchanged)
- Tests (no cargo/pnpm runs, pure doc migration)
- Build artifacts (no exe involved)

### Additional risks (from boxing project)

- AGENTS.md slimming is neat-freak "report before deciding": reported, awaiting user confirm
- Three-layer separation is user governance decision, not grill skill built-in standard
- README reader x purpose: writing-for-agents requires pointer sections mark trigger conditions
- Completed grill plan archival: neat-freak sync-matrix says "after merging effective content, file is delete candidate"
- Parallel fix branches: other branches may change files, check worktree before executing

## 6. Execution Order

Phase 1: Create dirs + .gitignore (low risk)

- mkdir docs/history/phases docs/history/handoffs docs/history/prompts
- mkdir docs/architecture docs/reference docs/research docs/how-to
- mkdir docs/agents
- .gitignore add .scratch/

Phase 2: Archive mixed products (git mv, preserve history)

- git mv docs/GRILL_T*.md docs/history/phases/
- git mv docs/HANDOFF_*.md docs/history/handoffs/
- git mv docs/USER_PROMPT_ARCHIVE.md docs/T9_MASTER_GOAL_PROMPT.md docs/history/prompts/
- git mv docs/T4_*.md docs/T7-*.md docs/history/phases/
- Create docs/history/README.md (HISTORICAL ARCHIVE marker)

Phase 3: Organize active docs (git mv)

- git mv docs/ARCHITECTURE.md docs/architecture/
- git mv docs/CONTEXT.md root (merge if existing)
- git mv docs/glossary.md docs/reference/
- git mv docs/RESIN_ROUTING_*.md docs/KEY_ENDPOINT_*.md docs/PROTOCOL_WEIGHT_*.md docs/research/
- git mv docs/MEMORY_REUSE_DECISION.md docs/REFACTOR_PLAN*.md docs/architecture/
- git mv docs/HEADLESS_*.md docs/RELEASE.md docs/how-to/
- git mv docs/PONYTAIL_DEBT*.md root
- git mv docs/PROJECT_PLAN.md docs/GRILL_ISSUES_BACKLOG.md docs/history/phases/

Phase 4: Create CHANGELOG.md

- Extract completed phases from git log
- Use Keep a Changelog 2.0 template
- Fill [Unreleased] + released versions
- Merge AGENTS.md phase sections (10-29) effective content into CHANGELOG

Phase 5: Slim AGENTS.md (highest risk, must be after Phase 4)

Per writing-for-agents branching test:

Keep (agent uses every run):

- context-mode routing block (ctx_* tool routing)
- CodeGraph conventions
- i18n conventions (full key coverage, 18 locale)
- Test conventions (cargo test, vitest, e2e)
- CI/CD conventions (build-all, release staging, chunk hash verify)
- Git hygiene (push after every commit, LF policy)
- Security redlines (IPC validation, SSRF guard, sidecar SSRF)
- Pointers to docs/ ("architecture see docs/architecture/, decisions see docs/adr/")

Migrate to docs/architecture/ (reference level):

- Runtime state (section 11) -> docs/architecture/runtime-state.md
- Storage locations -> docs/architecture/storage.md
- Sidecar lifecycle -> docs/architecture/sidecar.md
- Ghost safety net -> docs/architecture/ghost-safety.md

Migrate to CHANGELOG.md (process artifacts):

- IPC retarget (sections 15-20)
- Resin webhook (section 16)
- CI/CD release pipeline (section 17)
- P9-P24 phases (sections 18-29)
- R1-R2 refactor (section 23)

Not called (don't need):

- grill-with-docs: this task reorganizes existing, not new domain docs
- codebase-design: this task doesn't touch code module architecture

Phase 6: CI governance (low risk)

- .markdownlint-cli2.jsonc
- .github/workflows/docs-lint.yml
- Reference boxing project ai-docs-governance.yml (dead-link + layer-separation + size-warning)

## 7. Off-the-Shelf Template Wheels

| Need | Template | Repo | Usage |
|---|---|---|---|
| ADR template | adr/madr | GitHub 2.4k stars | current, keep |
| Changelog manual | Keep a Changelog 2.0 | keepachangelog.com | copy header + six types |
| Changelog auto | orhun/git-cliff | 12.1k stars | conventional commits |
| Markdown lint | DavidAnson/markdownlint-cli2-action | 188 stars | CI job |
| Link check | lycheeverse/lychee-action | 508 stars | CI + cron |
| Prose lint | vale-cli/vale | 6k stars | error-level gate |
| AGENTS.md template | Taiizor/agents-md-cookbook | GitHub | stack template + lint CI |

## 8. Sources

### Industry research (32 sources, atomcode 2 rounds)

Diataxis + C4 + ADR/MADR + AGENTS.md (agents.md official, 60k+ repos, LF AAIF)

- markdownlint/lychee/Vale (GitHub Action level)
- arXiv 2602.11988 (ETH: agent instructions should only have commands + pointers)
- Tauri/React/Rust/Vite/Airflow repo evidence
- Codex hard limit project_doc_max_bytes = 32 KiB (from boxing project research)

### grill workflow suite (18 engineering + 35 full skills)

main flow: grill-with-docs/to-spec/to-tickets/implement
on-ramps: triage/diagnosing-bugs/wayfinder
standalone: grill-me/grilling/research/handoff/to-questionnaire
vocabulary: domain-modeling/codebase-design
setup: setup-matt-pocock-skills -> docs/agents/

### neat-freak + writing-for-agents (boxing project already invoked)

neat-freak: knowledge governance closeout, "reduce before add", sync-matrix (one-time plan docs delete candidates)
writing-for-agents: information hierarchy 3 levels (step/reference/disclosed), branching test, context pointer

### boxing project reference (completed 2 commits)

boxing (D:/Aworker/crx/boxing): 286 lines/39218 bytes AGENTS.md -> 257 lines/~27000 bytes
Completed: 3-layer split (git mv 4 files + pointer + governance section) + CI governance (ai-docs-governance.yml)
Our project is 7x more severe (230637 bytes), but approach is directly referenced

### Project exploration (codegraph)

86 files 1452 nodes 4493 edges; 3-layer arch (resin-core -> src-tauri -> src);
IPC all forward to Resin sidecar; 7 Views (topology/platforms/nodes/settings/processRoute/subscriptions/diagnostics)

## 9. Decision Points (need user confirmation)

### Decision 1: AGENTS.md slimming scope

Current sections 10-29 are milestones. Per arXiv paper and neat-freak, should migrate to CHANGELOG.
But some sections contain still-effective engineering conventions (e.g. section 7.5 IPC validation, 7.6 SSRF guard).

Question: migrate sections 10-29 wholesale to CHANGELOG (pure history), or split per-section (conventions stay in AGENTS.md, milestones migrate)?

Recommendation (boxing experience): split per-section. Conventions are live Never-boundaries, milestones are process artifacts.

### Decision 2: Archive location naming

boxing uses docs/history/ (inside docs/), I initially proposed .archive/ (at repo root).

Question: use docs/history/ (boxing-validated, CI-compatible) or .archive/ (repo root)?

Recommendation: use docs/history/ like boxing, compatible with CI layer-separation check.

### Decision 3: CI governance depth

boxing landed ai-docs-governance.yml (dead-link + layer-separation + size-warning).
We could copy that or add markdownlint + lychee as independent jobs.

Recommendation: copy boxing ai-docs-governance.yml structure + add markdownlint-cli2-action.

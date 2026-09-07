# ADR-0067: License layering and third-party provenance (GPL-3.0-or-later repo-wide)

- **Status**: ACCEPTED
- **Date**: 2026-09-05 (architecture-recovery ticket 01, wave W1)
- **Research basis**: spec §8 R-A2 (atomcode-research: industrial licensing
  models for vendored third-party components combined with own copyleft code,
  plus comparable AI-gateway/proxy projects); local facts: `resin/LICENSE`
  (MIT, Resinat 2026), `resin/go.mod` (direct dep
  `github.com/sagernet/sing-box v1.12.21`), sing-box v1.12.21 upstream
  `LICENSE` (GNU GPLv3-or-later application notice, read from the module
  cache original, not from memory), root `Cargo.toml`
  (`license = "GPL-3.0-or-later"`), `README.md` / `README_CN.md` License
  sections.

## Context

The repository declares GPL-3.0-or-later in `README.md`, `README_CN.md` and
the root `Cargo.toml` `[workspace.package]` — but until this ADR there was no
root `LICENSE` file, no third-party obligation registry, and no legislated
model for how the three distributable artifacts relate license-wise. The repo
is preparing to go public; at that moment every one of these gaps becomes a
trust loss for an evaluator trying to answer "who owns what and what are the
obligations".

The load-bearing fact chain (all verbatim from upstream originals):

1. Resin v1.2.0 (vendored at `resin/`, shipped as the compiled sidecar
   binary) declares **MIT** (`resin/LICENSE`, "Copyright (c) 2026 Resinat and
   contributors").
2. Resin's `go.mod` at v1.2.0 pins `github.com/sagernet/sing-box v1.12.21`.
3. sing-box v1.12.21's `LICENSE` is a GNU GPLv3-or-later application notice
   ("Copyright (C) 2022 by nekohasekai … version 3 … or (at your option) any
   later version"), plus a name-association clause.
4. Therefore the compiled `resin` sidecar binary **conveys
   GPL-3.0-or-later obligations** when distributed, regardless of Resin's own
   MIT declaration. A permissive upstream declaration does not erase copyleft
   obligations inherited through the dependency tree.

## Decision

### D1 — Repo-wide GPL-3.0-or-later, verbatim, with a layering declaration

The root `LICENSE` file is the official GPL-3.0-or-later plain text from
<https://www.gnu.org/licenses/gpl-3.0.txt> (byte-identical, zero rewording).
The whole repository declares GPL-3.0-or-later. Directory-level layering:

| Layer | Content | Effective license |
| --- | --- | --- |
| Own shell source (`src/`, `src-tauri/`, `crates/resin-core/`, `scripts/`, docs) | first-party work | GPL-3.0-or-later (root `LICENSE`) |
| Vendored Resin source (`resin/`) | third-party upstream source, vendored | MIT (Resinat's own declaration, `resin/LICENSE`) |
| Compiled sidecar binary (`resin` in release artifacts) | compiled output of the vendored tree | GPL-3.0-or-later obligations conveyed on distribution (dependency-tree truth) |

MIT is one-way compatible into GPL-3.0-or-later, so vendoring MIT Resin
source inside a GPL repo is legal as-is. Alternatives rejected: switching the
repo to MIT to "match Resin's declaration" (would discard the copyleft
obligations the sidecar binary actually carries — self-inconsistent);
AGPL-3.0 (network-use clause has no bearing on a local desktop shell +
loopback sidecar; adds friction for no gained precision); dual-license
"MIT OR GPL-3.0-or-later" (the sidecar's GPL obligations are not ours to
relicense away — the dual offer would be untrue for the distributed binary).

### D2 — Published-artifact obligation table (THIRD_PARTY.md)

`THIRD_PARTY.md` is the obligation registry for every externally distributed
artifact: shell source, sidecar binary, release bundle. Each entry records
{component, version, distribution form, declared license, dependency-tree
truth, upstream link}. Resin v1.2.0 carries the **two-layer value** —
declared MIT + dependency-tree GPL-3.0-or-later via sing-box (go.mod citation
+ upstream LICENSE quotation included) — and every artifact's redistribution
obligations are listed under it. On any `RESIN_UPSTREAM_MANIFEST.yaml`
version bump this registry is re-verified against upstream originals at the
new tag in the same commit (enforced by the ADR-0017 amendment below).

External readers (evaluators, redistributors, auditors) should start from
[UPSTREAM.md](../architecture/UPSTREAM.md) — the single-page router over the
coverage ledger, the version manifest, this registry, and the layering
decision — rather than hunting the four documents individually.

### D3 — Mere-aggregation boundary (invariant)

The Tauri shell (`src-tauri/` + `crates/resin-core/` + `src/`) and the Resin
sidecar are **independent works distributed side by side**. They interact
exclusively over the loopback REST seam (`ResinClient`,
`crates/resin-core/src/resin_client.rs`; shell → `http://127.0.0.1` only).
This is mere aggregation under GPL-3.0 §2: the shell's GPL terms do not
"infect" Resin's own source (which stays MIT), and Resin's MIT terms do not
weaken the shell's GPL. The invariant is: **no in-process linking, no
embedding of Resin source into the shell (or shell source into Resin), no
shared memory/struct surface beyond the REST seam.** If any future change
introduces in-process coupling (e.g. FFI, embedding the Go tree, a shared
build), this ADR's aggregation analysis must be revisited before the change
lands.

### D4 — License-field discipline (machine-checked)

`README.md`, root `Cargo.toml` `[workspace.package].license`, and
`package.json` `license` must all equal the repo-wide value
(GPL-3.0-or-later); member crates inherit via `license.workspace = true` and
never hardcode; root `LICENSE` must be the official GPL-3.0 plain text (LF,
no BOM). Enforced by `scripts/license-field-check.cjs` (style follows the
ipc-manifest guard: zero deps, FAIL lines + non-zero exit on any drift).
**This ticket ships the script unmounted** — wiring it into
`scripts/verify-build.sh` / `package.json` scripts belongs to the CI-gate
ticket (03) so multiple windows never edit verify-build concurrently
(spec D-06). Self-test evidence (break → red, restore → green) is in the
ticket report.

### D5 — Contributor licensing: decision point recorded, not decided

Whether to require a CLA/DCO before accepting public contributions is **a
decision point that must be resolved before the repo goes public**, but this
ticket deliberately does not land one. Recorded for the pre-public round:
options are (a) DCO sign-off only (lightest, standard for small copyleft
projects), (b) CLA (title transfer / broad license grant — only if a
foundation or company stewardship emerges), (c) inbound=outbound
(inbound contributions license under GPL-3.0-or-later by default, no extra
instrument). Default lean: inbound=outbound. The chosen option lands in
`CONTRIBUTING.md` as part of the community-files ticket set.

## Consequences

- Root `LICENSE` (official text) + `THIRD_PARTY.md` (two-layer registry)
  exist; evaluators can trace "who owns what" without reading go.mod trees
  themselves.
- `docs/RESIN_UPSTREAM_MANIFEST.yaml` now records the two-layer value
  instead of a bare `license: "MIT"`.
- `CONTEXT.md` gains the license-layering vocabulary (License Layering /
  Mere Aggregation / Third-Party Provenance); `AGENTS.md` carries the
  operating clause.
- The field-consistency guard exists but is not yet mounted (ticket 03).
- Every future upstream Resin bump re-runs the dependency-tree license scan
  (ADR-0017 amendment).
- No CLA lands in this ticket (D5 stays a recorded decision point).

# Third-Party Components and License Obligations

This file is the registry of third-party components that ship inside
EgressAPIKEY's distributable artifacts, and the license obligations each one
carries. It exists so that any evaluator or redistributor can verify in one
place **who owns what and what the obligations are** (spec: ADR-0067 D2).

Two layers are recorded per component and they are not the same thing:

- **Declared license** — what the component's own `LICENSE` file says.
- **Dependency-tree truth** — what the component's dependency graph actually
  conveys at compile/bundle time. A permissive declaration does not erase
  copyleft obligations inherited through dependencies.

The repository-wide declaration is [`LICENSE`](LICENSE)
(GPL-3.0-or-later, verbatim official text from
<https://www.gnu.org/licenses/gpl-3.0.txt>). The layering decision and the
mere-aggregation boundary are legislated in
[ADR-0067](docs/adr/0067-license-layering-provenance.md); the version-pinning
discipline that forces a re-scan of this file on every upstream bump is
[ADR-0017](docs/adr/0017-upstream-resin-version-manifest.md) + its 2026-09-05
dependency-tree-scan amendment.

## Registry

| Component | Version | Distribution form | Declared license | Dependency-tree truth | Upstream |
| --- | --- | --- | --- | --- | --- |
| Resin sidecar | v1.2.0 | vendored at `resin/`; compiled `resin` binary shipped in `release/<os>-gui/` and `release/<os>-backend/` | MIT | GPL-3.0-or-later (via sing-box, see below) | <https://github.com/Resinat/Resin> |

## Resin sidecar (vendored, v1.2.0)

### Layer 1 — declared license: MIT

`resin/LICENSE`, verbatim header:

```text
MIT License

Copyright (c) 2026 Resinat and contributors
```

Obligations at this layer: keep the copyright + permission notice with any
redistribution of the source or the binary. That alone would be sufficient —
if the dependency tree stayed permissive. It does not.

### Layer 2 — dependency-tree truth: GPL-3.0-or-later via sing-box

Evidence chain, all citations from upstream originals at the pinned version:

1. `resin/go.mod` (upstream: <https://github.com/Resinat/Resin/blob/v1.2.0/go.mod>)
   declares the direct dependency, verbatim:

   ```text
   github.com/sagernet/sing-box v1.12.21
   ```

2. sing-box v1.12.21's own `LICENSE`
   (<https://github.com/SagerNet/sing-box/blob/v1.12.21/LICENSE>) is a
   GNU GPLv3 application notice, verbatim opening:

   ```text
   Copyright (C) 2022 by nekohasekai <contact-sagernet@sekai.icu>

   This program is free software: you can redistribute it and/or modify
   it under the terms of the GNU General Public License as published by
   the Free Software Foundation, either version 3 of the License, or
   (at your option) any later version.
   ```

   → SPDX value **GPL-3.0-or-later**. (The same file closes with a
   name-association clause: derivative works may not use the sing-box name or
   imply association without prior consent — independent of the GPL terms.)

3. Consequence: the compiled `resin` sidecar binary incorporates GPL-3.0-or-later
   code, so **distributing that binary conveys GPL-3.0-or-later obligations**
   (corresponding source availability included) regardless of Resin's MIT
   declaration. The MIT layer still governs Resin's own source files.

### Why the repository declares GPL-3.0-or-later

With the sidecar binary carrying GPL obligations, a repo-wide
GPL-3.0-or-later declaration is the only self-consistent value: MIT is
one-way compatible into GPL-3.0, and the shell (`src-tauri/`, `crates/resin-core/`,
`src/`) is an independent work that shares no code with the sidecar — the two
interact exclusively over a loopback REST seam (`ResinClient`, ADR-0067 D3),
which is mere aggregation, not a derivative work. Details and rejected
alternatives: ADR-0067 D1.

### Redistribution obligations per artifact

- **Shell source** (`src/`, `src-tauri/`, `crates/`): GPL-3.0-or-later per the
  root [`LICENSE`](LICENSE).
- **Resin sidecar binary**: GPL-3.0-or-later obligations for the combined
  binary (layer 2 above); Resin's own source remains MIT (layer 1).
- **Release bundle overall** (`release/`): an aggregate of independent works
  distributed side by side — each part keeps its own terms; nothing is merged
  or in-process linked (mere-aggregation invariant, ADR-0067 D3).

## Maintenance rules

- Any `docs/RESIN_UPSTREAM_MANIFEST.yaml` version bump MUST re-verify this
  entry against upstream originals at the new tag (go.mod + LICENSE at the
  exact tag URL) and update this file in the same commit (ADR-0017 amendment,
  2026-09-05).
- Evidence means quoted upstream originals with links — never from memory.
- New vendored or bundled third-party components get their own registry row
  (and a section if their layering is non-trivial) in the same commit that
  introduces them.
- Field consistency between `README.md`, root `Cargo.toml` and `package.json`
  is machine-checked by `scripts/license-field-check.cjs` (mounting owned by
  the CI-gate ticket; ADR-0067 D4).

# Domain Docs

How the engineering skills should consume this repo's domain documentation when
exploring the codebase.

## Layout: single-context

- `CONTEXT.md` at the repo root — the domain glossary (one entry per term, each
  with an _Avoid_ list of synonyms to not drift to)
- `docs/adr/` — architectural decision records (`00NN-slug.md`, numbered from
  `0001`)

There is no `CONTEXT-MAP.md` and no per-package `docs/adr/`: this repo is
single-context, and both files above are the only domain-doc sources.

## Before exploring, read these

- `CONTEXT.md` — always, before writing any prose that names domain concepts
- `docs/adr/` — the entries touching the area you're about to work in

If one of these is missing, proceed silently; `/domain-modeling` (reached via
`/grill-with-docs` and `/improve-codebase-architecture`) recreates terms and
decisions lazily as they resolve.

## Use the glossary's vocabulary

When your output names a domain concept — an issue title, a refactor proposal, a
hypothesis, a test name — use the term exactly as defined in `CONTEXT.md`. Don't
drift to the synonyms the glossary explicitly lists under _Avoid_.

If the concept you need isn't in the glossary yet, that's a signal — either you're
inventing language the project doesn't use (reconsider), or there's a real gap
(note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than
silently overriding:

> _Contradicts ADR-0036 (策略白盒单一写入口) — but worth reopening because…_

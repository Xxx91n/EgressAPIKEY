# Domain Docs

How the engineering skills should consume this repo's domain documentation when
exploring the codebase.

## Layout: single-context

- `GLOSSARY.md` at the repo root — the domain glossary (one line per term, each
  with an _Avoid_ list of synonyms to not drift to)

There is no `CONTEXT-MAP.md`: this repo is single-context and the glossary is
the only domain-doc source. Architectural decisions live as self-contained
comments at the code they govern (the ADR directory was retired 2026-10; the
records survive in git history).

## Before exploring, read these

- `GLOSSARY.md` — always, before writing any prose that names domain concepts

## Use the glossary's vocabulary

When your output names a domain concept — an issue title, a refactor proposal, a
hypothesis, a test name — use the term exactly as defined in `GLOSSARY.md`.
Don't drift to the synonyms the glossary explicitly lists under _Avoid_.

If the concept you need isn't in the glossary yet, that's a signal — either
you're inventing language the project doesn't use (reconsider), or there's a
real gap (add the term to `GLOSSARY.md` in the same commit).

## Flag decision conflicts

If your output contradicts a decision inlined in a source comment, surface it
explicitly rather than silently overriding.

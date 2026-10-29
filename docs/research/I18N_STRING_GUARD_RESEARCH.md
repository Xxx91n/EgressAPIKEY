# I18N String Guard Research — can a diff guard tell "correct French typography" from "silent damage"?

> Sources: Unicode CLDR `common/main/{fr,fr_CA,en,de}.xml` (unicode-org/cldr@main,
> fetched 2026-10-28), UCD `PropList.txt` / `LineBreak.txt` / `UnicodeData.txt`,
> `git-scm.com/docs/git-diff`, `git/git` `xdiff/xutils.c` + `xdiff/xmacros.h`
> (master), GNU gettext manual (Fuzzy Entries, msgmerge, msgfmt), FormatJS CLI
> docs, Lingui CLI/conf/catalog-format docs, i18next-parser README, Weblate docs,
> Jest CLI docs, Playwright snapshot docs, Renovate config docs, GitHub
> CODEOWNERS docs, actions/checkout README, Prettier rationale + options,
> RFC 8259, ECMA-262 §12.2, ESLint `no-irregular-whitespace` docs and rule
> source, EditorConfig spec, MDN `String.normalize`, Python `str.isspace`,
> Microsoft French (France) and French (Canada) Localization Style Guides
> (`aka.ms/french-france-styleguide` p. 33, `aka.ms/french-canada-styleguide`
> p. 33), OQLF *Vitrine linguistique* spacing table, fr.wikipedia *Espace
> insécable* (citing *Lexique des règles typographiques en usage à l'Imprimerie
> nationale*, p. 61 / p. 148-149).
>
> Claims marked **[measured]** were executed in this working tree or against this
> repository's git history during this research and are reproducible from the
> quoted commands. Unmarked claims are read from the cited primary source.
> Anything I could only establish from a search snippet is labelled
> **[search-summary, unverified]**.
>
> This document deliberately contains **no literal invisible characters** — every
> U+00A0 / U+202F is written as a code-point name, so the file itself can never be
> damaged by the same class of edit it is about.

---

## TL;DR

1. **The space before a French colon is mandated, not optional.** Every authority
   checked agrees on the *presence* of the space; only *which character* carries it
   is contested (U+00A0 word space vs U+202F narrow no-break space), and fr-FR and
   fr-CA differ.
2. **Option 2 does not survive.** A "whitespace-only inside a string" guard flags
   the legitimate French fix and the damage **identically** — measured at 32/32 in
   both directions on the real incident. Its "near-zero false positives" premise is
   wrong; it is 100 % false-positive on the legitimate repair.
3. **Option 1 survives**, and is the only shape that can be *correct*, because it
   does not attempt to judge. Two hard constraints: it must compare **raw** code
   points (no NFKC, no `trim`), and it must not use `git diff -w` (which hides
   exactly this class of damage).
4. **gettext `fuzzy` does not solve this**, and its mechanism is not transferable to
   a flat JSON catalog: fuzzy keys off *source* (`msgid`) drift, not *target* drift.
   The transferable idea is the **approval gate**, not the flag.
5. **Invisible characters break every normalization step you are tempted to add.** U+00A0/U+202F
   are `White_Space=yes` and NFKC-equivalent to U+0020, so `trim()`, `\s`, and NFKC all
   silently collapse the correct French string into the damaged one (measured). And
   `git diff -w` cannot even see U+00A0/U+202F as whitespace, so it hides the
   plain-space variant while showing the NNBSP variant — the worst possible split.

## Verified baseline: what actually happened in this repo

**[measured]** Reproduced from `git show ef48f95b:src/locales/fr/common.json` versus
`git show e327def8:src/locales/fr/common.json` (the damage commit `e327def8` and its
parent `ef48f95b`), and from the rollback `e8a2731f`.

| Fact | Value |
| --- | --- |
| Locale files touched by `e327def8` | 17 (`ar de en es fr hi id it ko nl pl pt ru th tr vi zh`) |
| French strings restored by `e8a2731f` | 32 |
| … of which differ **only** by U+0020 count | 32 / 32 |
| … that are colon repairs (`" :"` restored) | 27 |
| … that are semicolon repairs (`" ;"` restored) | 5 |

The 32 French repairs split cleanly into 27 colons and 5 semicolons — i.e. exactly
the set of French *signes doubles* (`: ; ! ?`), which is the rule the Microsoft
fr-FR guide states verbatim (see Q1). This is strong internal evidence that the
rollback was a **correctness** repair, not a stylistic preference.

**[measured]** Critically, the rollback also restored a space before the colon in
**en** (`"Bound :"`) and **de** (`"Gebunden :"`):

```
e327def8: src/locales/en/common.json  "portBound": "Bound:{{port}} → {{platform}}"
e8a2731f: src/locales/en/common.json  "portBound": "Bound :{{port}} → {{platform}}"
```

That is wrong in both languages — see Q1. Two caveats keep this honest:

- The space was **pre-existing** in en/de (present at `5f235d6f`), so `e8a2731f`
  *restored* it rather than inventing it; the mechanical cleanup had accidentally
  made en/de correct. This is the mirror-image asymmetry: the damage commit was
  wrong for fr and right for en/de.
- Either way the incident contains **both** a legitimate French repair **and** an
  illegitimate English/German repair, in the same commit, as the same one-character
  delta. Any guard must route the first to "approve" and the second to "reject", and
  no purely mechanical rule can do both. That asymmetry is the whole argument for
  Option 1.

**[measured]** The existing guard is blind to all of it: `scripts/i18n-check.cjs`
compares only the *key set* of each locale against `en`. The key sets of
`fr_before` and `fr_damaged` are identical (543 keys), so `pnpm i18n:check` returns
`i18n coverage OK` on the damaged tree. The line-ending guard in `ci.yml` is
`git diff --check`, which only inspects `core.whitespace` classes (trailing
whitespace at EOL / before tab / at EOF) and never looks inside string literals.

---

## Q1 — Is the space before a French colon actually mandated?

**Verdict: yes. Mandated in fr-FR and fr-CA. The character differs; the presence
does not.** This is what kills Option 2.

### 1.1 The rule in fr-FR (France)

**Microsoft French (France) Localization Style Guide**, p. 33, section
"Symbols & nonbreaking spaces", subsection "Nonbreaking space":

> "Remember that in French, a nonbreaking space is required before the following
> punctuation: `; ! : ?`"

([aka.ms/french-france-styleguide](https://aka.ms/french-france-styleguide), PDF
p. 33; **[measured]** extracted with `pdftotext -enc UTF-8`.)

The same guide gives worked examples in its "Colon" section (p. 29):

> "(+) Vous avez ouvert deux fichiers : le fichier source et le fichier cible.
> (+) Microsoft déclare : « Le succès est au rendez-vous ! »."

Note the space before `:` in both. Note also that the en dash / em dash guidance on
the same pages is explicitly *replaceable* ("it's recommended that you replace the
em dash with a period, comma, or parentheses"), whereas the nonbreaking-space
sentence is stated as a requirement ("is required"). The distinction is textual and
deliberate in the source.

**[measured]** A detail worth recording: in that PDF the NBSP is *typed as U+00A0*
in the `&nbsp;:` HTML fragment example (`d'identification suivantes&nbsp;:</FONT>`),
while the prose examples carry plain U+0020 (the PDF's own extraction shows no
U+00A0 anywhere in the body text — `s.count('\u00a0') == 0`, `s.count('\u202f') == 0`).
So even the style guide that mandates the rule does not itself round-trip the
character in its examples. That is Q5's problem in miniature.

**OQLF (Office québécois de la langue française), *Vitrine linguistique*,
"Espacement avant et après les signes de ponctuation et les symboles"** — the
"Deux-points" row reads:

> before: "Une espace insécable" · after: "Une espace"
> example: "Les étapes sont les suivantes : planification, …"

([vitrinelinguistique.oqlf.gouv.qc.ca/22039/…](https://vitrinelinguistique.oqlf.gouv.qc.ca/22039/la-typographie/espacement/espacement-avant-et-apres-les-signes-de-ponctuation-et-les-symboles))
The same table has a separate row "Deux-points (dans les heures numériques)" =
"Pas d'espace", i.e. the rule is *contextual*: `13:52:45` takes no space, a
prose colon does. **This matters for a guard**: a rule of the form "no space before
colon in locale X" is wrong even for the locale that "has no space rule", because
time literals are an exception inside the very same file.

**Imprimerie nationale, *Lexique des règles typographiques en usage à l'Imprimerie
nationale*** (2002, 6th ed., ISBN 978-2-7433-0482-9). I could not fetch the printed
work, so this is **[search-summary, unverified]** at the page level, but two
independent secondary sources quote it consistently:
- fr.wikipedia *Espace insécable*, citing *Lexique* p. 149: the French code
  typographique recommends before the colon "non une espace fine insécable mais une
  *espace insécable* (espace-mot insécable)".
- The same article cites *Lexique* p. 61 for the general no-break rule.
- A widely mirrored summary states "Les deux-points sont précédés d'une espace
  insécable et suivi d'une espace normale" — **[search-summary, unverified]**.

Treat the *presence* of the space as verified (Microsoft + OQLF + CLDR all agree);
treat the *"word space, not fine space"* refinement as very likely but
secondary-sourced.

### 1.2 French France vs French Canada

The two regions **disagree on the character, not on the presence**.

- fr.wikipedia *Espace insécable*, section "Au Canada": "Au Canada, le deux-points
  est séparé du mot précédent par une espace insécable … tandis que le
  point-virgule, le point d'interrogation et le point d'exclamation s'emploient sans
  espace sauf si on dispose de l'espace fine insécable". (**[search-summary]**
  against the printed OQLF *Banque de dépannage linguistique*, but the OQLF table in
  §1.1 independently confirms the colon gets "une espace insécable" while the other
  double signs are handled differently.)
- **Microsoft fr-CA guide**, p. 33, gives the rule *with the character annotated*:
  `(+) Chapitre 1 : Installation (nbsp before "1" and before ":")` — **[measured]**
  from `aka.ms/french-canada-styleguide`.

**Unicode CLDR is the decisive machine-readable witness, and it encodes the
difference.** In `unicode-org/cldr@main`:

| File | Line | Pattern | Space before `:` |
| --- | --- | --- | --- |
| `common/main/fr.xml` | 19 | `<localeKeyTypePattern>{0}<U+202F>: {1}</localeKeyTypePattern>` | **U+202F** |
| `common/main/fr.xml` | 1652 | `<codePattern type="language">langue<U+202F>: {0}</codePattern>` | **U+202F** |
| `common/main/fr_CA.xml` | 18 | `<localeKeyTypePattern>{0}<U+00A0>: {1}</localeKeyTypePattern>` | **U+00A0** |
| `common/main/fr_CA.xml` | 1317 | `<codePattern type="language">langue : {0}</codePattern>` | **U+0020** |
| `common/main/en.xml` | 1800 | `<codePattern type="language">Language: {0}</codePattern>` | **none** |
| `common/main/de.xml` | 1551 | `<codePattern type="language">Sprache: {0}</codePattern>` | **none** |

**[measured]** Raw bytes for `fr.xml:19` confirm U+202F, not U+00A0:
`...3e7b307de280af3a20...` — `e2 80 af` is U+202F NARROW NO-BREAK SPACE.

Three conclusions follow, and all three are fatal to a mechanical rule:

1. **fr-FR wants U+202F (narrow no-break) in CLDR's own data**, while Microsoft
   fr-FR says "nonbreaking space" and the *Lexique* reportedly says word-space. So
   the "correct" character is genuinely ambiguous *between authoritative sources for
   the same locale*.
2. **en and de take no space at all** (`Language:`, `Sprache:`). The `e8a2731f`
   rollback that added `"Bound :"` and `"Gebunden :"` was therefore **wrong** —
   it over-applied a French rule to two locales that do not have it.
3. **fr_CA's own file is internally inconsistent** (U+00A0 in
   `localeKeyTypePattern`, U+0020 in `codePattern`). A guard that demands one exact
   character per locale would fail on upstream CLDR itself.

### 1.3 Why this settles Q1 for guard design

The fact pattern is: *the same one-character delta* is correct in fr-FR, also correct
(but different character) in fr-CA, and incorrect in en/de. Since the guard sees
only `"... :"` → `"...:"` or `"...:"` → `"... :"`, **locale-aware character tables
do not disambiguate either** — because the correct character is itself disputed
(U+00A0 vs U+202F) and because the *same* locale file legitimately contains both
the spaced and unspaced forms (digital time).

> **Q1 answer: yes, mandated; character is U+202F in CLDR fr, U+00A0 in Microsoft
> fr-FR/fr-CA prose and in CLDR fr_CA's key pattern; en/de mandate no space. Because
> the "correct" character is disputed and context-dependent, no in-string whitespace
> rule can be both sound and complete. Option 2 is therefore not viable.**

## Q2 — How do real projects guard i18n strings against silent edits?

I looked for a project that ships a "translation changed without a key change" gate.
**None does, as a *target*-side check.** The ecosystem guards the *source* side and
gates *release*, not *edit*.

### 2.1 gettext `fuzzy` — real mechanism, wrong axis

GNU gettext manual, **§9.7.6 Fuzzy Entries**:

> "Fuzzy entries, even if they account for translated entries for most other
> purposes, usually call for revision by the translator. Those may be produced by
> applying the program `msgmerge` to update an older translated PO files according
> to a new PO template file, when this tool hypothesises that some new `msgid` has
> been modified only slightly out of an older one, and chooses to pair what it thinks
> to be the old translation for the new modified entry."
>
> "The slight alteration in the original string (the `msgid` string) should often be
> reflected in the translated string, and this requires the intervention of the
> translator. For this reason, `msgmerge` might mark some entries as being fuzzy."

([gnu.org/software/gettext/manual/html_node/Fuzzy-Entries.html](https://www.gnu.org/software/gettext/manual/html_node/Fuzzy-Entries.html))

And `msgmerge` docs (§7.1): "Where an exact match cannot be found, fuzzy matching is
used to produce better results"; `-N` / `--no-fuzzy-matching` disables it.

**Does fuzzy solve our problem? No — and the reason is precise:** fuzzy is triggered
by **`msgid` drift** (the *source* string changed), not by `msgstr` drift (the
*translation* was edited). Our incident is the opposite: `msgid` was untouched in
every locale, and only `msgstr` changed. gettext would have noticed **nothing**.
In a PO workflow the translator's edit is simply accepted — it is by definition an
authorised act, because translators are the only actors who write `msgstr`.

Second blocker: `fuzzy` is a **per-entry attribute stored in the file itself**
("Each PO file entry may have a set of *attributes* … using a special system comment").
A flat `{"key": "value"}` JSON catalog has nowhere to carry it without changing the
format — which would break `i18next`'s loader and `scripts/i18n-check.cjs`.

Third: even the flag is advisory. `msgfmt --use-fuzzy` includes fuzzy entries in the
compiled output, and the manual calls that "usually wrong, because fuzzy messages are
exactly those which have not been validated by a human translator". So the *default*
is to drop fuzzy strings from the binary, and the gate is at **compile**, not at
**commit**.

Takeaway: fuzzy is an **approval-required marker**, not a **content-diff detector**.
That is the shape worth borrowing (Q4), not the mechanism.

### 2.2 Content-hash message IDs — FormatJS

FormatJS CLI, `--id-interpolation-pattern`:

> "If certain message descriptors don't have id, this `pattern` will be used to
> automatically generate IDs for them. Default to `[sha512:contenthash:base64:6]`."

([formatjs.github.io/docs/tooling/cli/](https://formatjs.github.io/docs/tooling/cli/))

Its extracted-catalog example shows the consequence directly:

```
{
  "hak27d": { "defaultMessage": "Control Panel",        "description": "…" },
  "haqsd": { "defaultMessage": "Delete user {name}",    "description": "…" },
  "explicit-id": { "defaultMessage": "Confirm Password","description": "…" }
}
```

**This is the closest thing in the ecosystem to a per-message checksum** — and it is
exactly the mechanism that would have caught our incident: changing
`"Gebunden :{{port}}"` changes the hash, so the message gets a *new ID*, the old
translation is orphaned, and every locale shows a missing translation.

But note what it costs, and why we cannot simply adopt it:

- It inverts the identity: **the message text *is* the key**. A typography fix is then
  indistinguishable from deleting one message and adding another — which is *correct*
  behaviour but forces retranslation of all 18 locales for a one-character change.
- It only works for the **source** locale (`defaultMessage` lives in code). Translations
  are keyed by that ID; editing a *translation* still changes nothing but the value.
  So FormatJS would catch "en string edited" and would **not** catch "fr translation
  edited" — again the wrong axis for our incident, where 17 *translation* files were
  the casualty.
- It requires the whole extraction pipeline (`react-intl` descriptors) that this repo
  does not use (it uses `i18next` + `i18next-parser`).

### 2.3 Lingui — hashing exists, but not as a guard

Lingui's `lingui extract` merges and **preserves** existing translations by design
("the extract command merges messages extracted from source files with existing
message catalogs, ensuring that translated messages are preserved and not
accidentally lost"). Its validation surface is `lingui check missing` / `compile
--strict`, which fail on **missing** translations:

> "`lingui check missing` … Checks whether locale catalogs have missing translations."

Nothing in the CLI compares *old value* to *new value*. Lingui also explicitly
supports a metadata-bearing JSON form ("`style: 'lingui'` includes full Lingui-specific
metadata") which can carry `obsolete` / `fuzzy` flags — i.e. Lingui re-implements the
gettext idea rather than solving the diff problem. Its `minimal` style warns: "all
metadata about the message is lost. This includes … any message flags (obsolete,
fuzzy, etc.)."

### 2.4 i18next / i18next-parser — preserves, never diffs

The README documents exactly the behaviours that make silent edits invisible:

- "Preserve existing translations; only append missing keys as empty strings." — the
  default merge is source→catalog, one-way, and never inspects the value it keeps.
- `keepRemoved: false` — "Keep keys from the catalog that are no longer in code".
- `resetDefaultValueLocale: null` — "The locale to compare with default values to
  determine whether a default value has been changed. If this is set and a default
  value differs from a translation in the specified locale, all entries for that key
  across locales are reset to the default value, and existing translations are moved
  to the `_old` file."

That last option is the **only** value-comparison knob in the toolchain — and its
action is *destructive* (reset all locales), not *flag*. It also only fires for the
one locale designated as the source of truth. This repo's config does not set it.

### 2.5 Weblate — the approval model, on the source side

Weblate docs, "Editing source strings and keys":

> "Changing source text marks existing translations as needing editing. Changing only
> the key, context, or explanation preserves their states."

([docs.weblate.org/en/latest/user/translating.html](https://docs.weblate.org/en/latest/user/translating.html))

This is the gettext rule restated for a hosted TMS: **source edit ⇒ mark target
stale**. Same axis limitation. Its transferable part is that "needing editing" is a
*state a human must clear*, not an automatic revert — again the approval shape.

### 2.6 Pseudo-localization and checksum manifests

I found **no** mainstream project that ships a per-message hash manifest that fails CI
when a translation value changes without a key change. Pseudo-localization is a
*coverage/layout* technique (it replaces source text to expose hard-coded strings and
truncation), not a tamper detector, and it does not run against real translations.
The nearest published pattern is FormatJS's contenthash IDs (§2.2), which is a
*derivation* trick rather than an explicit manifest. **[search-summary, unverified]**
for "no such tool exists" — I searched and did not find one; absence of evidence is
not proof.

### 2.7 What this means

Every mechanism in the ecosystem answers "did the **source** change so that
translations are now stale?" Our threat is "did someone rewrite **translations**
in place?" Nothing in gettext, FormatJS, Lingui, i18next or Weblate detects that,
because in all of those systems the only actor who writes translations is a
translator, and their writes are trusted by construction. Our incident is a new
actor class (**a bulk text tool**) writing to the trusted side. That is why the
fix has to be a gate at the boundary where that actor operates — the commit —
not a feature borrowed from a TMS.

> **Q2 answer: gettext `fuzzy` does not solve it (it keys off `msgid` drift, is stored
> per-entry in the file format, and is only enforced at compile). No project was found
> that hashes per-message values and fails on a translation-only change; FormatJS
> contenthash IDs are the closest and catch only source-side edits. The reusable idea
> is "mark for human approval", not "detect and revert".**

## Q3 — Whitespace-only diff detection: does any tool separate "inside a string" from "formatting"?

**Short answer: no. Git cannot, by documented design. Prettier will not touch strings.
The one linter that could protect message content explicitly opts out of strings by
default.** And the specific behaviour that matters — does `-w` ignore whitespace
*inside* string literals — is **confirmed by both documentation and source**.

### 3.1 `git diff -w` / `-b`: documented semantics

`git diff` options, verbatim ([git-scm.com/docs/git-diff](https://git-scm.com/docs/git-diff)):

> `-b` / `--ignore-space-change`
> "Ignore changes in amount of whitespace. This ignores whitespace at line end, and
> considers all other sequences of one or more whitespace characters to be equivalent."
>
> `-w` / `--ignore-all-space`
> "Ignore whitespace when comparing lines. This ignores differences even if one line
> has whitespace where the other line has none."

Both are defined at the **line** level, with no notion of a string literal. Git has
no lexer for JSON/JS in `xdiff`; it compares bytes of whole lines.

### 3.2 `git diff -w` confirmed to ignore whitespace inside string literals — from source

`xdiff/xutils.c`, `xdl_recmatch()`, lines 173–203
([raw.githubusercontent.com/git/git/master/xdiff/xutils.c](https://raw.githubusercontent.com/git/git/master/xdiff/xutils.c)):

```c
173: int xdl_recmatch(const char *l1, long s1, const char *l2, long s2, long flags)
...
193:     if (flags & XDF_IGNORE_WHITESPACE) {
194:         goto skip_ws;
195:         while (i1 < s1 && i2 < s2) {
196:             if (l1[i1++] != l2[i2++])
197:                 return 0;
198:         skip_ws:
199:             while (i1 < s1 && XDL_ISSPACE(l1[i1]))
200:                 i1++;
201:             while (i2 < s2 && XDL_ISSPACE(l2[i2]))
202:                 i2++;
203:         }
```

The skip loop runs over the **entire raw line buffer** (`l1`/`l2`, offsets `i1`/`i2`
from 0). There is no state machine, no quote tracking, no escape handling. Every
space on the line is discarded before comparison — whether it is indentation, a space
between tokens, or a space inside `"…"`.

`xdiff/xmacros.h:33`:

```c
#define XDL_ISSPACE(c) (isspace((unsigned char)(c)))
```

`isspace()` in the **C locale** matches exactly six bytes: `' '`, `\t`, `\n`, `\v`,
`\f`, `\r` (C89 §7.4.1.3 / POSIX `isspace`). And `(unsigned char)` truncates each
byte — so multi-byte UTF-8 sequences (U+00A0 = `C2 A0`, U+202F = `E2 80 AF`) can
never match, and are compared byte-wise as ordinary characters. **This is the single
most consequential finding in this document** — see §3.4.

### 3.3 Measured behaviour (git 2.56.0.windows.3)

**[measured]** Constructed pairs of one-line JSON files and ran `git diff --no-index`.

```
$ git diff --no-index -w s1.json s2.json   # "Trier : x"  ->  "Trier: x"
exit=0        # identical under -w  => ASCII space before colon is HIDDEN

$ git diff --no-index -w n1.json s2.json   # "Trier<U+00A0>: x" -> "Trier: x"
exit=1        # still different     => NBSP removal is VISIBLE

$ git diff --no-index -w t1.json s2.json   # "Trier<U+202F>: x" -> "Trier: x"
exit=1        # still different     => NNBSP removal is VISIBLE
```

Two independent confirmations of the same conclusion:

- **`-w` hides the damage when the "correct" form was a plain space.** The exact
  `e8a2731f` French repair — `"Trier : par défaut"` → `"Trier: par défaut"` — is
  **invisible to `git diff -w`**. A CI gate built on `git diff -w` would have passed
  the damage commit silently. This is measured, not theorised.
- **`-w` shows it when the correct form is U+00A0/U+202F.** So the same flag is blind
  to the fr-FR-with-plain-space form and catches the CLDR-fr form. A guard whose
  sensitivity depends on which of two *both-authoritative* characters the translator
  used is not a guard.

Also **[measured]**: `-b` behaves like `-w` for this case (both collapse runs of
whitespace), and neither flag changes what `git diff --check` reports, because
`core.whitespace` classes are `blank-at-eol`, `space-before-tab`, `indent-with-non-tab`,
`tab-in-indent`, `blank-at-eof`, `cr-at-eol` — all *line-shape* concepts, none of
which look inside a string. So the repo's existing `git diff --check` line-ending
guard is structurally incapable of seeing this class of change
([git-config docs, `core.whitespace`](https://git-scm.com/docs/git-config#Documentation/git-config.txt-corewhitespace)).

### 3.4 `-w` is not just unsuitable — it is *anti*-correlated with correctness here

This is the sharpest result of the whole investigation:

| Intended change | Correct in fr? | `git diff -w` sees it? |
| --- | --- | --- |
| `"Trier: x"` → `"Trier : x"` (add plain space) | yes (MS fr-FR wording) | **no** — hidden |
| `"Trier: x"` → `"Trier<U+202F>: x"` (add NNBSP) | yes (CLDR fr) | yes |
| `"Trier : x"` → `"Trier: x"` (remove plain space = the damage) | no | **no** — hidden |
| `"Trier<U+202F>: x"` → `"Trier: x"` (remove NNBSP) | no | yes |

So `-w` hides the *legitimate plain-space repair* and hides the *damaging
plain-space removal* — and it cannot tell them apart, because by its own definition
they are the same edit. Using `-w` to reduce noise on a locale diff review is
therefore **actively dangerous**: it removes exactly the signal the reviewer needs.

### 3.5 Prettier: safe for strings, but that safety is the limitation

Prettier parses JSON and re-prints it. It **does not rewrite string contents**:

- Rationale: "Prettier only *prints* code. It does not transform it." and
  "Prettier maintains the way your string is escaped. For example, `"🙂"` won't be
  formatted into `"\uD83D\uDE42"` and vice versa."
  ([prettier.io/docs/rationale](https://prettier.io/docs/rationale))
- **[measured]** `npx prettier@3.4.2 --parser json` on a file containing
  `"Trier<U+00A0>: par"`, `"x <U+00A0> y"`, `"Lia<U+202F>: v"` re-emits the same
  bytes: output `od -c` shows `T r i e r 302 240 :` and `L i a 342 200 257 :`.
  No normalization, no `\u` escaping, no space collapse.

Consequence **in both directions**:
- **Good:** Prettier will not *cause* this incident. A `prettier --write` over
  `src/locales/**` is safe, and adding it is a cheap defense-in-depth (it also
  guarantees the diff of a formatting run is empty, so a "formatting" commit cannot
  smuggle string edits).
- **Bad:** Prettier also will not *detect* it. There is no Prettier mode that reports
  "this string's content changed". And in this repo Prettier is **not installed at
  all** — **[measured]** no `prettier` in `package.json` dependencies, no
  `.prettierrc`, no `.editorconfig`. So Prettier is not currently part of the
  pipeline and cannot be relied on as a guard.

### 3.6 ESLint `no-irregular-whitespace`: the linter that *could* protect strings — and doesn't, by default

This is the one linter in the JS ecosystem whose whole job is invisible whitespace, and
its default is to **allow it inside strings**:

> "`"skipStrings": true` (default) allows any whitespace characters in string literals"
> — [eslint.org/docs/latest/rules/no-irregular-whitespace](https://eslint.org/docs/latest/rules/no-irregular-whitespace)

The rule source (`lib/rules/no-irregular-whitespace.js:20-22`, **[measured]** fetched from
`eslint/eslint@main`) enumerates precisely the characters in question:

```js
const IRREGULAR_WHITESPACE =
    /[\f\v\u0085\ufeff\u00a0\u1680\u180e\u2000…\u200b\u202f\u205f\u3000]+/gu;
```

U+00A0 **and** U+202F are both listed. So the rule *knows* about them; it simply
declines to look inside strings because that is where they are legitimate.

Furthermore, **the rule does not apply to JSON at all**: ESLint lints JS/TS, and this
repo's `eslint.config.cjs` **[measured]** does not enable it and does not cover
`src/locales/**`. So the ecosystem's dedicated tool is (a) off by default for
strings and (b) not wired to JSON catalogs.

**What you *can* borrow from it: the inverse setting.** `{ "skipStrings": false }`
would make ESLint flag every U+00A0/U+202F in a `.ts` string. That is the wrong
policy for locale data (those characters are the payload) but the *right* policy for
**code**: a guard can legitimately assert "no invisible space in any `.ts`/`.tsx`
string in `src/`", which is a cheap, precise, zero-false-positive rule. It is not the
guard we need, but it is a free adjacent win.

### 3.7 The only linter that protects message *content* checks structure, not vectors

`eslint-plugin-i18n-json` is the dedicated JSON-i18n linter. Its four rules are
`valid-json`, `valid-message-syntax`, `identical-keys`, `sorted-keys` and
`identical-placeholders`. Not one compares *values* to their previous state —
`identical-keys` compares the key set (which this repo already does in
`scripts/i18n-check.cjs`) and `identical-placeholders` compares placeholder sets
against a reference file. So the dedicated tool still cannot see our incident.

> **Q3 answer: no established tool distinguishes "whitespace inside a string literal"
> from "whitespace in formatting". Git's `-w`/`-b` are documented at line granularity
> and their implementation (`xdl_recmatch`, `xdiff/xutils.c:193-203`) skips whitespace
> over the whole raw line with no quote awareness — CONFIRMED by source, so `-w` DOES
> ignore whitespace inside string literals. Worse, it hides the legitimate French
> plain-space repair and the damaging plain-space removal equally. Prettier preserves
> string bytes (measured) so it cannot cause the bug but also cannot catch it; it is
> not installed here. ESLint's dedicated rule defaults to `skipStrings: true` and does
> not cover JSON.**

## Q4 — Is there a canonical shape for "this change requires explicit approval"?

**Yes, and it is consistent across four independent tools: the guarded artifact is
written in a *pending* state, CI fails on the pending state, and a human moves it to
*approved* by an action that is itself reviewed. The guard never decides correctness —
it only converts a silent change into a blocking, attributable one.** That is exactly
the shape the team's lean toward Option 1-red describes.

### 4.1 Jest snapshots — the reference implementation

`--ci` ([jestjs.io/docs/cli](https://jestjs.io/docs/cli)):

> "When this option is provided, Jest will assume it is running in a CI environment.
> This changes the behavior when a new snapshot is encountered. Instead of the regular
> behavior of storing a new snapshot automatically, it will fail the test and require
> Jest to be run with `--updateSnapshot`."

`--updateSnapshot` / `-u`: "Use this flag to re-record every snapshot that fails during
this test run."

The three properties worth copying:
1. **Local run auto-accepts; CI run refuses.** The guard's strictness is a function of
   *where* it runs, not of *what* changed. This is why it has near-zero friction and
   near-zero false negatives.
2. **The approval is a separate, visible, reviewable artifact change.** Running `-u`
   rewrites `__snapshots__/*.snap`, so the approval shows up in the diff as a *file*
   the reviewer can read — not as a flag hidden in a command line or a comment.
3. **The failure message names the remedy.** "run with `--updateSnapshot`" — an
   operator (or agent) that hits the gate is told exactly what to do, which is what
   stops an agent from "fixing" it by hand-editing the golden file.

### 4.2 Playwright — same shape, plus explicit "review any changes"

`toHaveScreenshot()` writes a reference on first run, then compares. Docs:

> "Sometimes you need to update the reference screenshot, for example when the page has
> changed. Do this with the `--update-snapshots` flag."
>
> "You should commit this directory to your version control (e.g. `git`), and **review
> any changes to it**."
> — [playwright.dev/docs/test-snapshots](https://playwright.dev/docs/test-snapshots)

The explicit "review any changes to it" instruction is the part teams forget: golden
files are only a guard **if the diff of the golden file is in the reviewer's path**.
A binary `.png` under `-*-snapshots/` gets review by convention. A JSON catalog is
reviewed by nobody unless a human is forced to look.

### 4.3 Renovate — approval as a first-class, per-scope gate

`dependencyDashboardApproval` ([docs.renovatebot.com/configuration-options](https://docs.renovatebot.com/configuration-options/#dependencydashboardapproval)):

> "Controls if updates need manual approval from the Dependency Dashboard issue before
> PRs are created." … "you can tell Renovate to wait for your approval from the
> Dependency Dashboard before creating a branch/PR. You can approve a pending PR by
> selecting the checkbox in the Dependency Dashboard issue."
>
> scopes: `repo, packageRule, manager, updateType, vulnerabilityAlert, group`

`prCreation: "approval"` — "Renovate creates branches for updates immediately, but
creates the PR *after* getting Dependency Dashboard approval".

Two transferable ideas:
- **Approval is scoped and configurable** (all updates / major only / one package).
  For us: "any locale value change needs approval" is the right default, and the scope
  knob would let a future rule relax it per-locale.
- **Approval is an out-of-band act on a state machine**, not an in-diff annotation.
  This is why it survives rebases and squash-merges: nobody has to preserve a marker
  in the commit text.

### 4.4 Changesets — approval is the *artifact*, not a flag

Changesets "lets contributors declare how their changes should be released" via a
checked-in changeset file that is reviewed with the code. The mechanism is the same
family as 4.1/4.2: **the intent statement is a committed file**, not a boolean.

### 4.5 GitHub CODEOWNERS — the "no guard, review only" baseline (Option 3)

CODEOWNERS is real and works: "Code owners are automatically requested for review when
someone opens a pull request that modifies code that they own", and branch protection
can "Require review from Code Owners"
([docs.github.com …/about-code-owners](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners)).

But three facts make it insufficient **as the sole defense** here:

1. **It only fires on a PR.** This repo's default delivery is direct commits to a
   branch (`ci.yml` triggers on `push: branches: ['**']`; there is no PR-only gate).
   **[measured]** `find . -iname CODEOWNERS` → nothing exists in this repo today.
2. **It relies on the reviewer reading a whitespace diff.** `"Trier : x"` vs
   `"Trier: x"` renders in a GitHub diff as two nearly identical lines; humans
   demonstrably miss this (it is precisely what an AI agent was asked to do and got
   wrong). Review without a *highlight* is not review.
3. **`-w`-style tooling in the review path would hide it entirely** (§3.4).

So Option 3 is a *necessary complement* (a human must own `src/locales/**`) but is not
a guard: it has no artifact, no failure mode, and no signal to the reviewer beyond
"this file changed".

### 4.6 The converged shape

Across 4.1–4.5 the same five-part pattern appears:

| Part | Jest `--ci` | Playwright | Renovate | What we should copy |
| --- | --- | --- | --- | --- |
| Guarded artifact is separate from the code | `.snap` files | `*-snapshots/` | branch/PR | manifest file, not the catalog |
| CI fails on *unapproved* drift | new snapshot fails | missing/mismatched ref fails | waits for checkbox | guard fails on unapproved value change |
| Approval is an explicit act | `-u` | `--update-snapshots` | dashboard checkbox | explicit, attributable action |
| Approval is visible in review | `.snap` diff | `.png` diff | PR appears | the approval must show in the diff |
| Failure names the remedy | "run with `--updateSnapshot`" | "run `--update-snapshots`" | dashboard link | error must print the exact remedy |

> **Q4 answer: yes — the canonical shape is "fail on drift, require an explicit,
> attributable, reviewable approval act". Jest `--ci` + `-u` is the cleanest existing
> implementation: local runs auto-accept, CI refuses, and the approval materialises as a
> reviewable file change. Renovate adds the useful refinement that approval is an
> out-of-band state transition rather than an in-diff annotation, which survives
> rebase/squash. Borrow the shape; do not invent a new one.**

## Q5 — Does storing U+00A0 / U+202F in JSON locale files cause problems?

**Yes, in four distinct ways, and each one silently destroys the "correct" French
string. This is the strongest argument that the guard must store and compare raw
code points and must never normalize.**

### 5.1 What these characters *are*, per Unicode

| Char | Name | Category | `White_Space` | Line_Break | NFC | NFKC |
| --- | --- | --- | --- | --- | --- | --- |
| U+00A0 | NO-BREAK SPACE | `Zs` | **yes** | `GL` | unchanged | **U+0020** |
| U+202F | NARROW NO-BREAK SPACE | `Zs` | **yes** | `GL` | unchanged | **U+0020** |
| U+2009 | THIN SPACE | `Zs` | yes (not NBSP) | `BA` | unchanged | **U+0020** |
| U+0020 | SPACE | `Zs` | yes | `SP` | — | — |

Sources: `UnicodeData.txt` lines for `00A0`, `202F`, `2009`;
`PropList.txt:15` (`00A0 ; White_Space`) and `:20` (`202F ; White_Space`);
`LineBreak.txt:97` (`00A0 ; GL`) and `:922` (`202F ; GL`).
UAX #14 defines `GL` as "Non-breaking ("Glue") … Prohibit line breaks before and
after" ([unicode.org/reports/tr14](https://www.unicode.org/reports/tr14/)).

Two facts matter immediately:
- **Both are `White_Space=yes`.** So anything that says "strip/normalize/collapse
  whitespace" will eat them. This is the whole failure mode.
- **Both are NFKC-compatibility-equivalent to U+0020, but *not* NFC-equivalent.**
  NFC preserves them; NFKC destroys them.

### 5.2 NFKC silently converts the correct French string into the damaged one

**[measured]**, Node:

```js
const good = "Trier\u202f: par defaut";   // CLDR-correct fr
const bad  = "Trier : par defaut";        // plain space
good.normalize("NFKC") === bad.normalize("NFKC")   // => true   (!!)
good.normalize("NFKC") === "Trier : par defaut"    // => true
good.normalize("NFC")  === good                    // => true   (NFC is safe)
```

and Python, same result:

```
0xa0   NFC=['0xa0']   NFKC=['0x20']
0x202f NFC=['0x202f'] NFKC=['0x20']
```

So **any guard that normalizes with NFKC before comparing will report the correct
fr-FR string and the plain-space string as identical**, i.e. it will pass the bug.
This is a *false negative* introduced by the guard itself — worse than no guard,
because it creates the belief that the check exists.

Conversely, NFC is safe (preserves both), and `JSON.parse`/`JSON.stringify` are
byte-faithful for these characters (**[measured]**: round-trip preserves the exact
bytes; `JSON.stringify` emits raw UTF-8, never `\u00a0`).

### 5.3 JS `\s` / `trim` / `replace(/\s+/g)` all eat them

**[measured]**:

```js
/\s/.test("\u00a0")  // true
/\s/.test("\u202f")  // true
" \u00a0\u202fx ".trim()            // "x"      <- leading/trailing NBSP gone
"a \u00a0 \u202f b".replace(/\s+/g," ")  // "a b"  <- NNBSP collapsed
```

This is specified, not incidental. ECMA-262 §12.2 (White Space):

> "The ECMAScript white space code points are listed in Table 31 … any code point in
> general category **'Space_Separator'** … Note 1: `U+0020` (SPACE) and `U+00A0`
> (NO-BREAK SPACE) code points are part of `<USP>`."

Since U+202F is `Zs` (= `Space_Separator`), `\s` matches it too. Python is the same by
contract: "A character is *whitespace* if … its general category is `Zs` ('Separator,
space'), or its bidirectional class is one of `WS`, `B`, or `S`"
([docs.python.org/3/library/stdtypes.html#str.isspace](https://docs.python.org/3/library/stdtypes.html#str.isspace)).

**Design consequence:** a guard that does `value.trim()` or
`JSON.stringify(value).split(/\s+/)` before comparing is *guaranteed* to erase the
very distinction it is meant to police. The guard must compare with `===` on the raw
string, augmenting only by *counting* code points (see §7.3).

### 5.4 Diffs: git shows them as raw bytes; review UIs may render them as nothing

- Git emits the raw UTF-8 bytes. **[measured]** `git diff | cat -A` on
  `"Trier : par"` → `"Trier: par"` shows
  `-{"a":"Trier : par"}$` / `+{"a":"Trier: par"}$` — a human-readable one-character
  difference. But for the NBSP case, `git diff | cat -A` on
  `"Trier<U+202F>: x"` produced
  `-{"a":"TrierM-bM-^@M-/: x"}$` — the character appears as **nothing at all** in a
  normal terminal or GitHub diff view. A reviewer sees two identical lines.
- `git diff -w` removes the ASCII-space case entirely (§3.3/3.4) and *cannot* remove
  the NBSP case, because `XDL_ISSPACE` is ASCII-`isspace` (`xdiff/xmacros.h:33`).

So the *same* logical edit is either invisible-by-rendering (NBSP removed) or
invisible-by-flag (space removed). There is no diff view in which a reviewer reliably
sees it.

### 5.5 Editors/formatters: no evidence of forced normalization, but no protection either

- **Prettier: preserves.** Measured in §3.5 — bytes unchanged, no `\u` escaping, no
  collapsing. The docs state the general principle ("Prettier maintains the way your
  string is escaped").
- **ESLint `no-irregular-whitespace`: would report, but `skipStrings: true` by
  default** and it does not run on JSON here (§3.6). Its char class includes both
  U+00A0 and U+202F, so it is *capable* of flagging them in `.ts` files.
- **`trim_trailing_whitespace`** in EditorConfig is specified as "remove all whitespace
  characters preceding newline characters in the file" — it is defined at end-of-line,
  so it does not touch mid-string characters
  ([spec.editorconfig.org](https://spec.editorconfig.org/)). **This repo has no
  `.editorconfig`** **[measured]**, so this is moot today, but it is a reason *not* to
  add one with a broad `trim_trailing_whitespace = true` on `src/locales/**`.
- **JSON serializers differ between languages.** **[measured]**:
  - JS `JSON.stringify` → raw UTF-8 (`"x : y"` with the real NBSP).
  - Python `json.dumps(ensure_ascii=True)` (the **default**) → `"x\u00a0: y"`.
  - Python `json.dumps(ensure_ascii=False)` → raw UTF-8.
  So a tooling round-trip through Python with defaults will *rewrite the file* with
  escape sequences, producing a whole-file diff of 543 lines for a zero-semantic
  change — and, if anyone later "cleans up" the escapes by hand, an opportunity to
  drop one. RFC 8259 permits both forms (`\uXXXX` is a valid `char` escape), so both
  are correct JSON; they are just different bytes.

I found **no** primary evidence of an editor or formatter silently rewriting U+00A0 →
U+0020 on save. **[search-summary, unverified]** — I looked and did not find it. The
real risk is not a rogue formatter but **our own** `trim`/`normalize`/regex steps
(§5.2, §5.3), which are documented to destroy these characters.

### 5.6 The repo's current state

**[measured]** Scanning every file under `src/locales/**` for
`U+00A0 U+202F U+2009 U+2007 U+200B U+FEFF`:

```
files containing invisible spaces: NONE
```

and in `fr/common.json`, all 27 space-before-colon occurrences are **plain U+0020**
(e.g. `backup.passphraseHint` = `"Facultatif : chiffre l'archive"`, U+0020 before `:`).

So today the repo is *self-consistent with plain spaces* and contains zero invisible
characters. Two implications:
- The immediate risk is only the plain-space form — which is exactly the form
  `git diff -w` hides. The most dangerous configuration and the current configuration
  coincide.
- There is no existing NBSP to break, so adopting "plain U+0020 before French double
  punctuation" as the house convention is *available* to us and would make the
  correct string human-visible in every diff. See §7.4.

> **Q5 answer: yes, real problems. U+00A0/U+202F are `White_Space=yes` and
> NFKC-equivalent to U+0020 (but NFC-stable), so `trim()`, `\s`, and any NFKC
> normalization silently convert the correct French string into the damaged one —
> measured. JSON round-trips them fine in JS (and escapes them under Python's default
> `ensure_ascii=True`), and Prettier preserves them, but git renders them as invisible
> in normal diff viewers while `git diff -w` erases the plain-space variant. No
> evidence found of editors force-normalizing on save; the danger is our own
> normalization code. The repo currently has zero invisible characters and uses plain
> spaces throughout — which is the case most exposed to `-w`.**

## 对本仓的适用性

### 结论：只有方案 1 存活。方案 2 必须否决，方案 3 只能作为补充而非防线。

**方案 2（只标记"字符串内仅空白差异"的 hunk）被法语排版事实直接杀死，且是可实测的杀死，不是设计口味问题。**

在真实事故数据上跑两个方向（**[measured]**，`ef48f95b` → `e327def8` 及其逆）：

```
opt2 在「损坏」方向:  flagged(字符串内仅空白差异) = 32  missed(真实文字改动) = 0
opt2 在「修复」方向:  flagged(字符串内仅空白差异) = 32  missed(真实文字改动) = 0
```

两个方向输出**逐条相同、数量相同、内容对偶**。方案 2 声称的"接近零误报"恰恰反了：它对合法修复是 **100% 误报**，而合法修复正是这个 guard 存在的原因。它不是"能区分"，它是"必然把两者归为同一类"，然后无论选择放行还是拦截，都必错一半。

根源不是实现不够聪明，而是**信息论层面的不可能**：

1. 法语冒号前的空格是**强制**的（Microsoft fr-FR 风格指南 p.33："a nonbreaking space is required before the following punctuation: `; ! : ?`"；OQLF 表 "Deux-points" 行："Une espace insécable" 在前）。所以 `Trier: x` → `Trier : x` 是**修 bug**。
2. 英语/德语冒号前**不得**有空格（CLDR `en.xml:1800` `Language: {0}`、`de.xml:1551` `Sprache: {0}`）。所以同一串改动在 en/de 是**引入 bug**。注意方向细节：`Bound :` 的空格是**原本就有**的（见 `5f235d6f`），损坏提交 `e327def8` 反而把它改对了；是回滚 `e8a2731f` 把 en/de 又改错了。**损坏与回滚各错一半**，这恰好证明"机械修改"和"机械回滚"同样不可信。
3. 因此 `"... :"` ↔ `"...:"` 这一对 diff 在**同一个 commit 里**既可能是对的也可能是错的，判据只在"这是哪个 locale、哪条规则、哪个标点"。方案 2 的定义里没有 locale 维度，也没有标点类维度 —— 加进去就变成方案 1 的雏形，那就不是方案 2 了。
4. 而且连"正确的字符"都不唯一：CLDR `fr.xml:19` 用 **U+202F**，`fr_CA.xml:18` 用 **U+00A0**，`fr_CA.xml:1317` 的 `codePattern` 又是普通空格，Microsoft fr-FR 正文写 "nonbreaking space" 而 *Lexique* 据二手资料说用词间不换行空格。**上游 CLDR 自己都不自洽**，任何"按 locale 查表判对错"的规则都会在 CLDR 上误报。

**方案 3（只靠 CODEOWNERS 评审 `src/locales/**`）不足以作为防线，理由是本仓的实测事实而非原则性反对：**

- **[measured]** 本仓**没有** CODEOWNERS 文件（`find . -iname CODEOWNERS` 无结果），所以它今天保护力为零；加上也只是"请求评审"。
- `ci.yml` 的触发是 `push: branches: ['**']`，不是 PR-only；直推路径上 CODEOWNERS 不触发。
- 最致命：评审要看的是 `"Trier : x"` vs `"Trier: x"` 这种一行内差一个空格的 diff。§5.4 实测，如果正确形式是 U+202F，`git diff | cat -A` 会把它渲染成 `M-bM-^@M-/` 之类的转义串，普通终端/GitHub 视图里**看不见**；而若正确形式是普通空格，`git diff -w` 会**直接隐藏整行**。也就是说，人眼评审恰恰在这种 diff 上最不可靠 —— 这次事故就是"AI 被要求做机械清理、声称只动注释、实际动了字符串"的结果，说明"看一眼"不是可靠控制。
- CODEOWNERS 应保留（谁拥有 locale 仍要有主），但它**不产生失败信号**，无法阻止下一次。它是**补充**，不是方案。

**方案 1 存活，但必须带三条硬约束，否则会以更隐蔽的方式失败：**

方案 1 的正确性来源不是"能判断对错"，而是**放弃了判断**：把"值变了"这一可判定事实转成"需要人批准"这一流程状态。这正是 Q4 找到的既有范式（Jest `--ci` + `-u`、Playwright `--update-snapshots`、Renovate `dashboardApproval`）：guard 只负责把静默变更变成**阻塞的、可归因的**变更，正确性判断留给批准动作。团队倾向的"红 = 需要显式批准（commit-message 标记）"是对的，且不是发明，是借用。（标记的载体落地时改为提交态清单，见文末「落地记录」。）

三条约束，每条都有本仓实测依据：

1. **绝不能归一化后再比较。** NFKC 会把 U+00A0/U+202F 压成 U+0020（实测：`"Trier\u202f: x".normalize("NFKC") === "Trier : x".normalize("NFKC")` → `true`）。用 NFKC 的 guard 会把"正确法语"与"被压平版本"判为相同，**放过 bug**。同理禁用 `trim()` 与 `\s`（ECMA-262 §12.2：U+00A0 属 `<USP>`，U+202F 属 `Zs` 因而也被 `\s` 匹配）。只允许 `===` 比较原始字符串。NFC 是安全的（实测保留两者）。
2. **绝不能用 `git diff -w` 作为输入。** 源码实证 `xdiff/xutils.c:193-203` 在整个**原始行缓冲**上跳过空白，无引号状态机；`xdiff/xmacros.h:33` 的 `XDL_ISSPACE` 是 C-locale `isspace`，只认 6 个 ASCII 字节。实测：`"Trier : x"`→`"Trier: x"` 在 `-w` 下 **exit=0（看不见）**。用 `-w` 的 guard 会静默放过本次事故的主形态（本仓 27 处法语冒号全是普通 U+0020，正是被 `-w` 隐藏的形态）。guard 必须自己做 **JSON.parse 双侧 + 叶子值 `===` 比较**，不经过任何基于 `-w` 的 diff。
3. **必须在 CI 侧强制、本地侧放行。** Jest 的 `--ci` 语义。CI ready 的细节：本仓 `ci.yml` 用 `actions/checkout@v4` **未设 `fetch-depth`**，默认为 1（"Only a single commit is fetched by default"），因此**任何靠 `HEAD^` 做基线 diff 的设计都会静默失效**——`fetch-depth: 2`（或 0）是那种设计的前提条件。（实际落地的清单形态不读 git 历史，所以不需要它，见文末「落地记录」。）

**可落地的形态（与既有机制最贴近、改动最小）：**

- 复用 `scripts/` 的 guard 惯例（`i18n-check.cjs` 同级新增一个脚本，由 `scripts/verify-build.sh` 调用 —— 该脚本已经在跑 `i18n-check.cjs`，CI 的 verify job 已经在跑 `verify-build.sh`，所以**接线成本为零**，无需新建 workflow）。
- 基线取 `git show <base>:src/locales/<lc>/common.json`，`JSON.parse` 双侧，展平后比叶子值；任何值变更 → 非零退出并**打印每条变更的 `key`、locale、以及前后值的码点转写**（把 U+00A0 显示成 `[U+00A0]`，因为原始字符在终端里不可见 —— §5.4 实测）。**（基线来源未采用：`git show` 依赖被上面第 3 条否定的历史可用性，落地改为提交态清单，见文末「落地记录」。）**
- 批准机制按 Q4 借形：commit message 的 trailer（`git interpret-trailers` 原生支持 "a key-value pair with a colon as a separator"，例如 `I18N-Approved: <reason>`，且本仓历史已经在用 `Co-Authored-By:` trailer，格式不陌生）。guard 检测到该 trailer 则放行并把变更摘要打进日志。**这套"红→标记→放行"的语义与 Renovate 的 dashboard approval 同构**：批准是带外的状态转移，不在 diff 里，因此能扛住 rebase/squash。**（未采用：落地时改成提交态哈希清单，理由见文末「落地记录」——版本控制写入走 GitButler，amend / squash 会改写提交消息。）**
- 附带一个免费收益：把 `eslint-plugin` 层面或仓库自检里加一条 **`src/**/*.ts` 字符串内禁止 U+00A0/U+202F**（ESLint `no-irregular-whitespace` 的字符表已含这两者，见 `lib/rules/no-irregular-whitespace.js:20-21`；只需 `skipStrings: false` 且限定 `src/`）。这条对代码是零误报，且与 locale 数据互不干扰 —— 因为 locale 里的这些字符是**载荷**，代码里的这些字符**几乎必然是事故**。

**另外两点应当写进决策记录：**

- **本次事故的破坏面比 issue 记录更大，而且回滚本身也不干净。** issue 说"17 个语种 portBound 各 1 条"；实测 `e8a2731f` 的 fr 是 **32 条**（27 冒号 + 5 分号，即法语"双标点"全集）。并且 en/de 的 `Bound :` / `Gebunden :` 空格**原本就存在**（`5f235d6f` 即如此），损坏提交 `e327def8` 反而在 en/de 上是对的，回滚 `e8a2731f` 又把它们改错了。所以"回滚 = 修复"这个前提不成立。这对选 guard 有直接后果：**机械修改和机械回滚同样会错**，guard 的目标因此不该是"自动回滚"，而只能是"强制人看"。
- **`pnpm i18n:check` 对本次事故完全无效**（实测：key set 相同，543 keys，返回 OK）。所以"本仓已有 i18n guard"不能作为不新增 guard 的理由；现有 guard 守的是**覆盖率**，不守**内容**。

**一句话：方案 1 存活，因为它是唯一一个承认"guard 不能判断正确性"的方案；方案 2 用实测数据证明它对合法修复 100% 误报；方案 3 在本仓连文件都不存在，且其依赖的人眼评审恰好对这类 diff 最不可靠。落地时必须禁止 NFKC/trim、禁止 `-w`，并在 CI 侧以提交态清单强制（只读清单、不依赖 git 历史——见文末「落地记录」）。**

## 落地记录（2026-10-29，issue #26）

上文写于决策之前。实际落地的形态有两处不同，记在这里以免把建议当成现状：

1. **批准机制是提交态哈希清单，不是 commit-message trailer。** 本文"可落地的形态"推荐用 trailer；实际采用 `docs/I18N_CONTENT_MANIFEST.json`——每 locale 每 key 一条 `{d, t}`，`d` 是原始值的 sha256 前 12 位并作为比较对象，`t` 是批准时的码点转写（不换行空格写成 `[U+00A0]`），`node scripts/i18n-content-check.cjs --update` 是唯一的批准动作。理由：版本控制写入走 GitButler，amend / squash 会改写提交消息；清单是普通文件，其 diff 在 PR 里直接可见，正好是 Q4 借形时看重的"批准是提交态产物"。
2. **不需要 `fetch-depth: 2`。** 守卫只读提交态清单、不读 git 历史，浅克隆（默认 1 层）即可工作，也因此不依赖 `actions/checkout` 的任何改动。

另有一处上文未涉及、落地时补上的判断：守卫除值之外也报告**清单与目录树的漂移**（未记录的 key、被删的 key、新增或消失的 locale 目录），因为清单必须描述它所指纹化的目录树，否则一个往 18 个 locale 同时塞同一条坏文案的工具能同时躲过内容守卫与覆盖率守卫。跨 locale 的 key 集合一致性仍归 `scripts/i18n-check.cjs`。

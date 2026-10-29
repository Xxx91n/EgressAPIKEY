// i18n-content-check.cjs - user-visible copy CONTENT guard.
// scripts/i18n-check.cjs compares only key SETS, so incident e327def8 rewrote the
// VALUES of 48 strings (32 fr + one in each of 16 other locales) with every guard
// still green. This closes the value side and nothing else: a committed manifest
// (docs/I18N_CONTENT_MANIFEST.json) holds, per locale and key, the digest of the
// raw value plus its code-point transcription; check mode re-flattens the
// catalogs, compares digests with strict equality, and exits non-zero naming
// locale, key and both transcriptions - invisible characters as [U+00A0] escapes,
// because a terminal renders them as nothing. Approval is an explicit --update, so
// the manifest diff is the record of who accepted what; check mode never writes.
// Two hard constraints, both measured in docs/research/I18N_STRING_GUARD_RESEARCH.md:
//   - no NFKC, no trim, no regex whitespace collapsing: NFKC folds U+00A0 and
//     U+202F into U+0020, so a normalizing guard passes the damage it exists to catch;
//   - no `git diff -w` as input: it reports "Trier : x" -> "Trier: x" as identical.
// The guard judges nothing about typography - the same one-space delta is a fix in
// fr and a defect in en/de. Key coverage stays with i18n-check.cjs; a key with no
// recorded fingerprint is drift here only because the manifest then no longer
// describes the catalog.
// Zero deps, style aligned with scripts/i18n-check.cjs; --self-test follows the
// assertEq skeleton of scripts/ipc-manifest-check.cjs.
//
// Usage:
//   node scripts/i18n-content-check.cjs            # check (build gate, read-only)
//   node scripts/i18n-content-check.cjs --update   # rewrite the manifest (approval)
//   node scripts/i18n-content-check.cjs --self-test

const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const crypto = require("node:crypto");

const ROOT = path.join(__dirname, "..");
const LOCALES_DIR = path.join(ROOT, "src", "locales");
const MANIFEST_PATH = path.join(ROOT, "docs", "I18N_CONTENT_MANIFEST.json");
const NAMESPACE = "common";
const SCHEMA = 1;
const DIGEST_HEX = 12;
// A mass rewrite is the expected failure shape, so the log stays bounded and
// names the rest rather than burying the signal in 9774 entries.
const MAX_REPORTED = 40;

const REMEDY =
  "If these changes are intended, approve them by regenerating the manifest and\n" +
  "committing that diff (the diff is the approval record):\n" +
  "  node scripts/i18n-content-check.cjs --update";

function digest(value) {
  return crypto.createHash("sha256").update(value, "utf8").digest("hex").slice(0, DIGEST_HEX);
}

function escapeCodePoint(cp) {
  return "[U+" + cp.toString(16).toUpperCase().padStart(4, "0") + "]";
}

// Code points a terminal cannot show: C0/C1 controls, every Unicode White_Space
// except ordinary U+0020 (escaping spaces too would make every line unreadable),
// and the zero-width / bidi / format marks. Everything else stays literal so the
// manifest keeps the readability and rough size of the catalog it fingerprints.
// A mark missing from this table is still caught: the gate is the digest, this is
// only the rendering.
const HIDDEN_RANGES = [
  [0x0000, 0x001f], [0x007f, 0x009f], [0x00a0, 0x00a0], [0x00ad, 0x00ad],
  [0x0600, 0x0605], [0x061c, 0x061c], [0x06dd, 0x06dd], [0x070f, 0x070f],
  [0x08e2, 0x08e2], [0x1680, 0x1680], [0x180e, 0x180e], [0x2000, 0x200f],
  [0x2028, 0x202f], [0x205f, 0x2064], [0x2066, 0x206f], [0x3000, 0x3000],
  [0xfeff, 0xfeff], [0xfffc, 0xfffc],
];

function isHidden(cp) {
  for (const [lo, hi] of HIDDEN_RANGES) if (cp >= lo && cp <= hi) return true;
  return false;
}

function transcribe(value) {
  let out = "";
  for (const ch of value) {
    const cp = ch.codePointAt(0);
    out += isHidden(cp) ? escapeCodePoint(cp) : ch;
  }
  return out;
}

// The before value only exists here as rendered text, so the pointer walks
// transcriptions: an [U+XXXX] escape counts as one token, otherwise a pointer
// into "a[U+00A0]: b" would stop at the "[" and name the wrong character.
function tokens(text) {
  const chars = Array.from(text);
  const out = [];
  for (let i = 0; i < chars.length; i++) {
    if (chars[i] === "[") {
      const rest = chars.slice(i).join("");
      const m = /^\[U\+([0-9A-F]{4,6})\]/.exec(rest);
      if (m) {
        out.push(parseInt(m[1], 16));
        i += m[0].length - 1;
        continue;
      }
    }
    out.push(chars[i].codePointAt(0));
  }
  return out;
}

function diffPoint(before, after) {
  const a = tokens(before);
  const b = tokens(after);
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) {
      return "first difference at token " + (i + 1) + ": " + escapeCodePoint(a[i]) + " -> " + escapeCodePoint(b[i]);
    }
  }
  return "identical up to token " + n + "; length " + a.length + " -> " + b.length;
}

function listLocales(localesDir) {
  return fs
    .readdirSync(localesDir, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
}

function flatten(obj, prefix, out) {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? prefix + "." + k : k;
    if (v && typeof v === "object" && !Array.isArray(v)) flatten(v, key, out);
    else out.push({ key: key, value: typeof v === "string" ? v : JSON.stringify(v) });
  }
  return out;
}

function collectCatalog(localesDir) {
  const locales = [];
  const errors = [];
  for (const locale of listLocales(localesDir)) {
    const file = path.join(localesDir, locale, NAMESPACE + ".json");
    if (!fs.existsSync(file)) {
      errors.push("locale directory " + locale + " has no " + NAMESPACE + ".json");
      continue;
    }
    let parsed;
    try {
      parsed = JSON.parse(fs.readFileSync(file, "utf8"));
    } catch (e) {
      errors.push("cannot parse " + path.relative(ROOT, file).split(path.sep).join("/") + ": " + e.message);
      continue;
    }
    locales.push({ locale: locale, entries: flatten(parsed, "", []) });
  }
  return { locales: locales, errors: errors };
}

function buildManifest(catalog) {
  const locales = {};
  for (const lc of catalog.locales) {
    const map = {};
    for (const e of lc.entries) map[e.key] = { d: digest(e.value), t: transcribe(e.value) };
    locales[lc.locale] = map;
  }
  return { schema: SCHEMA, locales: locales };
}

// One line per key: a manifest diff has to name the drifted key, and
// JSON.stringify's two-space indent spends two lines doing it. Key order follows
// the catalog file, so a reordered catalog reorders the manifest instead of
// churning every entry.
function renderManifest(manifest) {
  const lines = ["{", '  "schema": ' + manifest.schema + ",", '  "locales": {'];
  const localeEntries = Object.entries(manifest.locales);
  localeEntries.forEach(([locale, map], i) => {
    lines.push("    " + JSON.stringify(locale) + ": {");
    const keys = Object.keys(map);
    keys.forEach((key, j) => {
      const e = map[key];
      lines.push("      " + JSON.stringify(key) + ": " + JSON.stringify({ d: e.d, t: e.t }) + (j + 1 < keys.length ? "," : ""));
    });
    lines.push("    }" + (i + 1 < localeEntries.length ? "," : ""));
  });
  lines.push("  }", "}");
  return lines.join("\n") + "\n";
}

function compare(catalog, manifest) {
  const problems = [];
  const recorded = manifest.locales || {};
  for (const lc of catalog.locales) {
    const map = recorded[lc.locale];
    if (!map) {
      problems.push({ locale: lc.locale, key: "*", kind: "locale-unrecorded", count: lc.entries.length });
      continue;
    }
    for (const e of lc.entries) {
      const now = digest(e.value);
      const old = map[e.key];
      if (!old) {
        problems.push({ locale: lc.locale, key: e.key, kind: "added", after: { d: now, t: transcribe(e.value) } });
      } else if (old.d !== now) {
        problems.push({
          locale: lc.locale,
          key: e.key,
          kind: "changed",
          before: { d: old.d, t: old.t },
          after: { d: now, t: transcribe(e.value) },
        });
      }
    }
    const live = new Set(lc.entries.map((e) => e.key));
    for (const key of Object.keys(map)) {
      if (!live.has(key)) problems.push({ locale: lc.locale, key: key, kind: "removed", before: map[key] });
    }
  }
  for (const locale of Object.keys(recorded)) {
    if (!catalog.locales.some((l) => l.locale === locale)) {
      problems.push({ locale: locale, key: "*", kind: "locale-removed", count: Object.keys(recorded[locale]).length });
    }
  }
  return problems;
}

function formatProblem(p) {
  if (p.kind === "changed") {
    return [
      "  " + p.locale + "  " + p.key,
      "    before  " + JSON.stringify(p.before.t) + "  [" + p.before.d + "]",
      "    after   " + JSON.stringify(p.after.t) + "  [" + p.after.d + "]",
      "    " + diffPoint(p.before.t, p.after.t),
    ];
  }
  if (p.kind === "added") {
    return ["  " + p.locale + "  " + p.key + "  (no recorded fingerprint)", "    value   " + JSON.stringify(p.after.t) + "  [" + p.after.d + "]"];
  }
  if (p.kind === "removed") {
    return ["  " + p.locale + "  " + p.key + "  (key deleted, fingerprint still recorded)", "    was     " + JSON.stringify(p.before.t) + "  [" + p.before.d + "]"];
  }
  if (p.kind === "locale-unrecorded") {
    return ["  " + p.locale + "  *  (locale directory has no entry in the manifest; " + p.count + " keys unguarded)"];
  }
  return ["  " + p.locale + "  *  (locale directory is gone; " + p.count + " fingerprints still recorded)"];
}

function printProblems(problems, header, print) {
  print(header);
  for (const p of problems.slice(0, MAX_REPORTED)) {
    for (const line of formatProblem(p)) print(line);
  }
  if (problems.length > MAX_REPORTED) {
    print("  ... and " + (problems.length - MAX_REPORTED) + " more not listed");
  }
}

function loadManifest(manifestPath) {
  if (!fs.existsSync(manifestPath)) {
    return { errors: ["missing manifest " + path.relative(ROOT, manifestPath).split(path.sep).join("/") + " - create it with: node scripts/i18n-content-check.cjs --update"] };
  }
  let parsed;
  try {
    parsed = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  } catch (e) {
    return { errors: ["manifest " + path.basename(manifestPath) + " is not valid JSON: " + e.message] };
  }
  if (parsed.schema !== SCHEMA) {
    return { errors: ["manifest schema is " + parsed.schema + ", this guard reads " + SCHEMA + " - regenerate with --update"] };
  }
  return { manifest: parsed };
}

// Read-only by construction: the only writeFileSync call in this file lives in
// runUpdate, and the self-test asserts the manifest bytes are untouched here.
function runCheck(localesDir, manifestPath) {
  const catalog = collectCatalog(localesDir);
  const loaded = loadManifest(manifestPath);
  if (catalog.errors.length || loaded.errors) {
    return { errors: catalog.errors.concat(loaded.errors || []), problems: [], fingerprints: 0, localeCount: catalog.locales.length };
  }
  const problems = compare(catalog, loaded.manifest);
  let fingerprints = 0;
  for (const lc of catalog.locales) fingerprints += lc.entries.length;
  return { errors: [], problems: problems, fingerprints: fingerprints, localeCount: catalog.locales.length };
}

function runUpdate(localesDir, manifestPath) {
  const catalog = collectCatalog(localesDir);
  if (catalog.errors.length) return { errors: catalog.errors, drifted: 0, fingerprints: 0 };
  const loaded = loadManifest(manifestPath);
  const previous = loaded.manifest || { schema: SCHEMA, locales: {} };
  const manifest = buildManifest(catalog);
  const text = renderManifest(manifest);
  fs.writeFileSync(manifestPath, text, "utf8");
  const drifted = compare(catalog, previous);
  let fingerprints = 0;
  for (const lc of catalog.locales) fingerprints += lc.entries.length;
  return { errors: [], drifted: drifted.length, problems: drifted, fingerprints: fingerprints, localeCount: catalog.locales.length };
}

function assertEq(actual, expected, label) {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a !== b) {
    console.error("self-test FAIL: " + label + " expected " + b + " got " + a);
    process.exit(1);
  }
  console.log("self-test ok: " + label);
}

// Fixtures are the real incident strings, read from git history and written here as
// escapes so this file carries no invisible character either. The three states:
//   ef48f95b = catalog before the cleanup, e327def8 = the damage, e8a2731f = rollback.
//   fr  backup.passphraseHint  "Facultatif : chiffre l'archive" -> "Facultatif: ..."
//   fr  nodes.refreshNoChange  "... n\u0153uds ; r\u00e9essayer"    -> "... n\u0153uds; ..."
//   en  platform.portBound     "Bound :{{port}} ..."            <-> "Bound:{{port}} ..."
//   de  platform.portBound     "Gebunden :{{port}} ..."         <-> "Gebunden:{{port}} ..."
// The 32 fr repairs split 27 colons + 5 semicolons, i.e. the French signes doubles.
// fr and en/de are OPPOSITE here and the names stay neutral on purpose: the spaced
// form is mandated in fr (Microsoft fr-FR p.33, OQLF, CLDR fr) and a defect in en/de
// (CLDR "Language:", "Sprache:"). e327def8 was wrong for fr and right for en/de;
// e8a2731f fixed fr and re-broke en/de. So the en/de pair is the reverse fixture -
// whichever direction you call the repair, this guard treats both the same way.
// The no-normalization trio below is not from git history: the U+00A0 / U+202F forms
// are the CLDR fr.xml / fr_CA.xml shapes recorded in
// docs/research/I18N_STRING_GUARD_RESEARCH.md section 1.2.
const FR_COLON_SPACED = "Facultatif : chiffre l'archive";
const FR_COLON_UNSPACED = "Facultatif: chiffre l'archive";
const FR_SEMI_SPACED = "Aucun changement du nombre de n\u0153uds ; r\u00e9essayer";
const FR_SEMI_UNSPACED = "Aucun changement du nombre de n\u0153uds; r\u00e9essayer";
const EN_BOUND_SPACED = "Bound :{{port}} \u2192 {{platform}}";
const EN_BOUND_UNSPACED = "Bound:{{port}} \u2192 {{platform}}";
const DE_BOUND_SPACED = "Gebunden :{{port}} \u2192 {{platform}}";
const DE_BOUND_UNSPACED = "Gebunden:{{port}} \u2192 {{platform}}";

function catalogOf(locales) {
  return { locales: locales, errors: [] };
}

function entry(locale, key, value) {
  return {
    locale: locale,
    entries: [{ key: key, value: value }],
  };
}

function manifestOf(locale, key, value) {
  return { schema: SCHEMA, locales: { [locale]: { [key]: { d: digest(value), t: transcribe(value) } } } };
}

function kinds(problems) {
  return problems.map((p) => p.kind + ":" + p.locale + ":" + p.key);
}

function selfTest() {
  // 1. Strict equality on raw strings: the invisible forms must not collapse.
  //    NFKC would fold all three to U+0020 and pass the damage (research 5.2).
  const plain = "Trier : par d\u00e9faut";
  const nbsp = "Trier\u00a0: par d\u00e9faut";
  const nnbsp = "Trier\u202f: par d\u00e9faut";
  assertEq(
    [digest(plain) === digest(nbsp), digest(plain) === digest(nnbsp), digest(nbsp) === digest(nnbsp)],
    [false, false, false],
    "no NFKC: U+0020 / U+00A0 / U+202F variants hash differently"
  );
  assertEq(transcribe(nbsp), "Trier[U+00A0]: par d\u00e9faut", "invisible space is rendered as a code-point escape");
  assertEq(transcribe(plain), plain, "ordinary space stays literal so lines remain readable");

  // 2. Damage direction, both French double punctuation classes.
  for (const [key, ok, damaged] of [
    ["backup.passphraseHint", FR_COLON_SPACED, FR_COLON_UNSPACED],
    ["nodes.refreshNoChange", FR_SEMI_SPACED, FR_SEMI_UNSPACED],
  ]) {
    const problems = compare(catalogOf([entry("fr", key, damaged)]), manifestOf("fr", key, ok));
    assertEq(kinds(problems), ["changed:fr:" + key], "fr damage is caught: " + key);
  }

  // 3. Negative assertion: a legitimate typographic repair is not an error once
  //    approved. The guard owns no whitespace rule, so the repaired fr string
  //    passes exactly as the damaged one did.
  assertEq(
    compare(catalogOf([entry("fr", "backup.passphraseHint", FR_COLON_SPACED)]), manifestOf("fr", "backup.passphraseHint", FR_COLON_SPACED)).length,
    0,
    "legitimate fr repair (space restored) is not judged an error"
  );

  // 4. The en/de reverse fixture: the same one-space delta, opposite verdict in
  //    CLDR terms, identical behaviour here. A self-test that only asserted
  //    "erasing the space errors" would contradict the correct en/de fix.
  assertEq(
    kinds(compare(catalogOf([entry("en", "platform.portBound", EN_BOUND_SPACED)]), manifestOf("en", "platform.portBound", EN_BOUND_UNSPACED))),
    ["changed:en:platform.portBound"],
    "en space added = drift, not a verdict"
  );
  assertEq(
    kinds(compare(catalogOf([entry("de", "platform.portBound", DE_BOUND_UNSPACED)]), manifestOf("de", "platform.portBound", DE_BOUND_SPACED))),
    ["changed:de:platform.portBound"],
    "de space removed = drift, same shape as en"
  );
  assertEq(
    compare(catalogOf([entry("en", "platform.portBound", EN_BOUND_UNSPACED)]), manifestOf("en", "platform.portBound", EN_BOUND_UNSPACED)).length,
    0,
    "CLDR-correct en (no space) passes too - the guard never picks a side"
  );

  // 5. The report names locale, key and both transcriptions, and points at the
  //    deleted space in code points (a single space is otherwise invisible).
  const [problem] = compare(catalogOf([entry("fr", "backup.passphraseHint", FR_COLON_UNSPACED)]), manifestOf("fr", "backup.passphraseHint", FR_COLON_SPACED));
  assertEq(
    formatProblem(problem),
    [
      "  fr  backup.passphraseHint",
      "    before  \"Facultatif : chiffre l'archive\"  [" + digest(FR_COLON_SPACED) + "]",
      "    after   \"Facultatif: chiffre l'archive\"  [" + digest(FR_COLON_UNSPACED) + "]",
      "    first difference at token 11: [U+0020] -> [U+003A]",
    ],
    "report prints locale, key, before/after transcription and the difference"
  );
  const nbspProblem = compare(catalogOf([entry("fr", "k", plain)]), manifestOf("fr", "k", nbsp))[0];
  assertEq(
    diffPoint(nbspProblem.before.t, nbspProblem.after.t),
    "first difference at token 6: [U+00A0] -> [U+0020]",
    "escape-aware pointer names U+00A0, not the [ of its own rendering"
  );

  // 6. Key set changes are drift against the manifest; cross-locale coverage
  //    stays scripts/i18n-check.cjs's job.
  assertEq(
    kinds(compare(catalogOf([entry("fr", "a", "x")]), { schema: SCHEMA, locales: { fr: { b: { d: digest("x"), t: "x" } } } })),
    ["added:fr:a", "removed:fr:b"],
    "added and removed keys are both manifest drift"
  );
  assertEq(
    kinds(compare(catalogOf([entry("fr", "a", "x"), entry("ps", "a", "x")]), manifestOf("fr", "a", "x"))),
    ["locale-unrecorded:ps:*"],
    "a new locale directory is unrecorded"
  );
  assertEq(
    kinds(compare(catalogOf([]), manifestOf("fr", "a", "x"))),
    ["locale-removed:fr:*"],
    "a deleted locale directory is reported"
  );

  // 7. Rendering is deterministic and round-trips, so an unchanged catalog
  //    produces a byte-identical manifest and --update cannot smuggle churn.
  const m = buildManifest(catalogOf([entry("fr", "k", FR_SEMI_SPACED), entry("en", "k2", "quote \" and \\ back")]));
  const rendered = renderManifest(m);
  assertEq(renderManifest(JSON.parse(rendered)), rendered, "manifest render is deterministic and round-trips");
  assertEq(rendered.includes("\u00a0"), false, "manifest text carries no raw invisible character");

  // 8. Filesystem half: --update is the only writer, and check mode is read-only
  //    even when it fails.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "i18n-content-selftest-"));
  try {
    const localesDir = path.join(dir, "locales");
    const frDir = path.join(localesDir, "fr");
    fs.mkdirSync(frDir, { recursive: true });
    const catalogFile = path.join(frDir, NAMESPACE + ".json");
    const manifestPath = path.join(dir, "manifest.json");
    fs.writeFileSync(catalogFile, JSON.stringify({ backup: { passphraseHint: FR_COLON_UNSPACED } }), "utf8");

    assertEq(runCheck(localesDir, manifestPath).errors.length, 1, "missing manifest is an error naming --update");
    const updated = runUpdate(localesDir, manifestPath);
    assertEq([updated.errors.length, updated.drifted], [0, 1], "--update reports the drift it approves");
    const written = fs.readFileSync(manifestPath, "utf8");

    fs.writeFileSync(catalogFile, JSON.stringify({ backup: { passphraseHint: FR_COLON_SPACED } }), "utf8");
    const failed = runCheck(localesDir, manifestPath);
    assertEq(kinds(failed.problems), ["changed:fr:backup.passphraseHint"], "check catches the repaired value");
    assertEq(fs.readFileSync(manifestPath, "utf8"), written, "check mode never rewrites the manifest");

    runUpdate(localesDir, manifestPath);
    const afterApproval = fs.readFileSync(manifestPath, "utf8");
    assertEq(kinds(runCheck(localesDir, manifestPath).problems), [], "approved value checks clean");
    runUpdate(localesDir, manifestPath);
    assertEq(fs.readFileSync(manifestPath, "utf8"), afterApproval, "--update is idempotent");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
  console.log("self-test: all assertions passed");
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--self-test")) {
    selfTest();
    return;
  }
  if (argv.includes("--update")) {
    const res = runUpdate(LOCALES_DIR, MANIFEST_PATH);
    if (res.errors.length) {
      console.error("i18n content manifest NOT written:");
      for (const e of res.errors) console.error("  - " + e);
      process.exit(1);
    }
    if (res.problems.length) {
      printProblems(res.problems, "Approving " + res.drifted + " content change(s):", console.log);
    }
    console.log(
      "i18n content manifest written: " + res.fingerprints + " fingerprints across " + res.localeCount + " locales (" + res.drifted + " drifted)"
    );
    return;
  }
  const res = runCheck(LOCALES_DIR, MANIFEST_PATH);
  if (res.errors.length) {
    console.error("i18n content guard cannot run:");
    for (const e of res.errors) console.error("  - " + e);
    process.exit(1);
  }
  if (res.problems.length) {
    printProblems(res.problems, "i18n content drift: " + res.problems.length + " value(s) differ from " + path.basename(MANIFEST_PATH) + ":", console.error);
    console.error(REMEDY);
    process.exit(1);
  }
  console.log("i18n content OK: " + res.fingerprints + " fingerprints across " + res.localeCount + " locales match the committed manifest");
}

main();

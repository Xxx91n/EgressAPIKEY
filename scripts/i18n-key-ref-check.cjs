// i18n-key-ref-check.cjs - i18n key REFERENCE-EXISTENCE guard.
// scripts/i18n-check.cjs compares key sets locale against locale, and
// scripts/i18n-content-check.cjs fingerprints the values of keys that exist.
// Neither one sees a key that source code USES but no catalog CONTAINS: 55fd259c
// shipped topConverge.tooltip.header that way (caught by eye), and
// DiagnosticsView later used diagnostics.portHealth.platform / .policy under a
// string leaf, so users read the raw key. This guard closes exactly that relation
// and nothing else: every whole-literal key used in src/ must resolve to a string
// leaf of src/locales/en/common.json (en is the canonical catalog; i18n-check.cjs
// keeps the other 17 locales equal to it).
//
// The direction is reference existence (used => present), not extraction
// (present <= what a lexer could find). It never writes a catalog: the i18n key
// codegen / `pnpm i18n:scan` rewrite was rejected (#39) because a lexer cannot see
// keys built at runtime and deletes them.
//
// What counts as a whole-literal key (checked):
//   t("a.b") / t('a.b') / t(`a.b`) whose literal is the whole first argument
//   (followed by `,` or `)`), including i18n.t("a.b"); and i18nKey="a.b" (Trans).
// What is ignored on purpose (a static scan cannot know the key; not reported):
//   - template literals with interpolation:  t(`nav.${item.key}`)
//   - concatenation:                         t("strategy." + aClass)
//   - identifier / expression arguments:     t(labelKey), t(i18nKey),
//                                            t(COMPACT_REV_KEY[phase]), t(a ? "x" : "y")
// Comments are blanked before scanning, so prose that quotes a call is not a use.
//
// Rust-emitted keys: the error.* family reaches the webview as data
// (IpcError.i18n_key, built in crates/resin-core/src/ipc_error.rs) and is only
// rendered through translateError(t(i18nKey)), so the TS scan above structurally
// cannot see it - and, being ignored as an identifier argument, can never report
// it either. A second pass covers it instead of an allowlist: every "error.<key>"
// string literal in ipc_error.rs must resolve in en as well.
//
// Plurals: t("x", { count }) resolves "x" or any CLDR-suffixed "x_one"/"x_other"/...
// sibling, so either form counts as present.
// Zero deps, style aligned with scripts/i18n-check.cjs; check mode is read-only
// and reports through the exit code; --self-test follows the assertEq skeleton of
// scripts/ipc-manifest-check.cjs.
//
// Usage:
//   node scripts/i18n-key-ref-check.cjs              # check (build gate, read-only)
//   node scripts/i18n-key-ref-check.cjs --self-test  # scanner self-test

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.join(__dirname, "..");
const SRC_DIR = path.join(ROOT, "src");
const EN_CATALOG = path.join(SRC_DIR, "locales", "en", "common.json");
const RUST_EMITTER = path.join(ROOT, "crates", "resin-core", "src", "ipc_error.rs");
const PLURAL_SUFFIXES = ["zero", "one", "two", "few", "many", "other"];

function relPosix(file) {
  return path.relative(ROOT, file).split(path.sep).join("/");
}

function listSourceFiles(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (e.name === "locales") continue;
      out.push(...listSourceFiles(p));
    } else if (/\.(ts|tsx|js|jsx)$/.test(e.name) && !e.name.endsWith(".d.ts")) {
      out.push(p);
    }
  }
  return out;
}

// Replace every comment with spaces (newlines kept, so offsets and line numbers
// survive) while leaving string, template and regex literals intact. Inside a
// template literal, ${...} is code again (tracked by brace depth). A `/` starts a
// regex literal when the previous significant character cannot end an operand.
// Known limit: bare JSX text is not parsed as such, so an apostrophe in it
// (don't) opens a pseudo-string to the end of that line; a comment after it on
// the same line is then scanned as code - a loud false red, never a silent miss.
function blankComments(src) {
  const out = src.split("");
  const n = src.length;
  const stack = []; // "tpl" = inside template text; a number = brace depth of an open ${
  let i = 0;
  let prev = "";
  const blank = (from, to) => {
    for (let k = from; k < to; k++) if (out[k] !== "\n" && out[k] !== "\r") out[k] = " ";
  };
  const inTemplateText = () => stack.length > 0 && stack[stack.length - 1] === "tpl";
  while (i < n) {
    const c = src[i];
    if (inTemplateText()) {
      if (c === "\\") { i += 2; continue; }
      if (c === "`") { stack.pop(); prev = "`"; i++; continue; }
      if (c === "$" && src[i + 1] === "{") { stack.push(0); prev = "{"; i += 2; continue; }
      i++;
      continue;
    }
    // `//` right after `:` is a URL in bare JSX text (see https://x.io), not a comment.
    if (c === "/" && src[i + 1] === "/" && src[i - 1] !== ":") {
      let j = i;
      while (j < n && src[j] !== "\n") j++;
      blank(i, j);
      i = j;
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      const end = src.indexOf("*/", i + 2);
      const j = end === -1 ? n : end + 2;
      blank(i, j);
      i = j;
      continue;
    }
    if (c === '"' || c === "'") {
      let j = i + 1;
      while (j < n && src[j] !== c && src[j] !== "\n") j += src[j] === "\\" ? 2 : 1;
      i = j + 1;
      prev = c;
      continue;
    }
    if (c === "`") { stack.push("tpl"); i++; continue; }
    if (c === "/" && (prev === "" || "(,=:[!&|?{};+-*%<>~^".includes(prev))) {
      let j = i + 1;
      let inClass = false;
      while (j < n && src[j] !== "\n") {
        if (src[j] === "\\") { j += 2; continue; }
        if (src[j] === "[") inClass = true;
        else if (src[j] === "]") inClass = false;
        else if (src[j] === "/" && !inClass) break;
        j++;
      }
      i = j + 1;
      prev = "/";
      continue;
    }
    if (stack.length > 0 && typeof stack[stack.length - 1] === "number") {
      if (c === "{") stack[stack.length - 1]++;
      else if (c === "}") {
        if (stack[stack.length - 1] === 0) { stack.pop(); i++; continue; }
        stack[stack.length - 1]--;
      }
    }
    if (!/\s/.test(c)) prev = c;
    i++;
  }
  return out.join("");
}

// Whole-literal key references in one source text. Returns [{ key, line }].
// t( must not be the tail of a longer identifier (format(, set(), but may follow a
// dot (i18n.t(). The literal must be the entire first argument: the next
// significant character is `,` or `)`; anything else (`+`, `?`, `as`, `[`) is an
// expression and is ignored.
function extractKeyRefs(src) {
  const code = blankComments(src);
  const refs = [];
  const lineAt = (idx) => code.slice(0, idx).split("\n").length;
  const call = /(?<![A-Za-z0-9_$])t\(\s*(["'`])((?:\\.|(?!\1)[^\\\n])*)\1\s*([^\s])/g;
  let m;
  while ((m = call.exec(code)) !== null) {
    const [, quote, body, next] = m;
    if (next !== "," && next !== ")") continue;
    if (quote === "`" && body.includes("${")) continue;
    refs.push({ key: body, line: lineAt(m.index) });
  }
  // i18nKey="a.b" or i18nKey={"a.b"}; in the braced form the literal must close
  // the expression (i18nKey={"strategy." + x} is concatenation and is ignored).
  const attr = /(?<![A-Za-z0-9_$])i18nKey\s*=\s*(\{\s*)?(["'])([^"'\n]*)\2\s*([^\s])?/g;
  while ((m = attr.exec(code)) !== null) {
    if (m[1] && m[4] !== "}") continue;
    refs.push({ key: m[3], line: lineAt(m.index) });
  }
  return refs;
}

// Rust emitter pass: every "error.<ident>" string literal, comment lines excluded.
function extractRustErrorKeys(src) {
  const keys = new Set();
  for (const raw of src.split(/\r?\n/)) {
    const line = raw.replace(/\/\/.*$/, "");
    const re = /"(error\.[A-Za-z0-9_.]+)"/g;
    let m;
    while ((m = re.exec(line)) !== null) keys.add(m[1]);
  }
  return [...keys];
}

// True when key names a string leaf of the catalog (or a plural sibling of one).
// A key that lands on an object (a namespace) or walks THROUGH a string leaf
// (diagnostics.portHealth.platform under "Port Health Check") is unresolved.
function resolves(catalog, key) {
  const leaf = (k) => {
    let node = catalog;
    for (const seg of k.split(".")) {
      if (node === null || typeof node !== "object" || !Object.prototype.hasOwnProperty.call(node, seg)) return false;
      node = node[seg];
    }
    return typeof node === "string";
  };
  if (leaf(key)) return true;
  return PLURAL_SUFFIXES.some((s) => leaf(key + "_" + s));
}

function check() {
  const catalog = JSON.parse(fs.readFileSync(EN_CATALOG, "utf8"));
  const problems = [];
  let checked = 0;
  for (const f of listSourceFiles(SRC_DIR)) {
    const rel = relPosix(f);
    for (const ref of extractKeyRefs(fs.readFileSync(f, "utf8"))) {
      checked++;
      if (!resolves(catalog, ref.key)) problems.push(rel + ":" + ref.line + "  " + ref.key);
    }
  }
  const rustRel = relPosix(RUST_EMITTER);
  const rustKeys = extractRustErrorKeys(fs.readFileSync(RUST_EMITTER, "utf8"));
  for (const key of rustKeys) {
    if (!resolves(catalog, key)) problems.push(rustRel + "  " + key + "  (Rust-emitted i18n_key)");
  }
  return { problems: problems, checked: checked, rustKeys: rustKeys.length };
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

function keysOf(src) {
  return extractKeyRefs(src).map((r) => r.key);
}

// Synthetic fixtures only. The legitimate cases must PASS (resolve, or be
// ignored) - otherwise a guard that reports everything would also "catch" the
// defect and the self-test would prove nothing.
function selfTest() {
  const catalog = {
    nav: { topology: "Topology" },
    strategy: { random: "Random" },
    strategyVerify: { platform: "Platform", policy: "Policy" },
    diagnostics: { portHealth: "Port Health Check" },
    subscription: { imported_one: "Imported {{count}} node", imported_other: "Imported {{count}} nodes" },
    error: { notFound: "Not found" },
  };

  // Checked: whole literals in every quote style, member calls, Trans i18nKey.
  assertEq(
    keysOf([
      't("strategyVerify.platform")',
      "t('strategy.random', { x: 1 })",
      "t(`nav.topology`)",
      'i18n.t("error.notFound")',
      '<Trans i18nKey="strategyVerify.policy" />',
      "<Trans i18nKey={'nav.topology'} />",
    ].join("\n")),
    ["strategyVerify.platform", "strategy.random", "nav.topology", "error.notFound", "strategyVerify.policy", "nav.topology"],
    "whole-literal t() in three quote styles, member call, Trans i18nKey"
  );

  // Ignored: template literal with interpolation (src/App.tsx nav.${item.key}).
  assertEq(keysOf("aria-label={t(`nav.${item.key}`)}\ntitle={t(`theme.${th}`)}"), [], "template literal with ${} is ignored");
  // Ignored: concatenation (src/views/PlatformsView.tsx "strategy." + aClass).
  assertEq(keysOf('const a = t("strategy." + aClass);\n{t("nodes." + toast.key, toast.opts)}'), [], "concatenation is ignored");
  // Ignored: identifier / expression arguments (labelKey, i18nKey, lookup table, ternary).
  assertEq(
    keysOf([
      "t(labelKey, { error: e })",
      "t(i18nKey, { defaultValue: i18nKey })",
      "t(COMPACT_REV_KEY[convergePhase], { rev })",
      't(cond ? "a.b" : "c.d")',
      "t(disabledReasonKey(reason))",
    ].join("\n")),
    [],
    "identifier, lookup-table, ternary and call arguments are ignored"
  );
  // Not a t() call: longer identifiers ending in t, and comments quoting a call.
  assertEq(
    keysOf([
      'format("x.y"); set("x.y"); assert("x.y");',
      '// aria-label = t("subscription.delete", name)',
      '/* t("gone.key") */ const s = "// not a comment"; t("nav.topology")',
      'const re = /["`]/g; t("strategy.random")',
      'const tpl = `a ${ { k: 1 }.k } b`; t("error.notFound")',
    ].join("\n")),
    ["nav.topology", "strategy.random", "error.notFound"],
    "other identifiers, comments, strings with //, regex and nested template braces"
  );
  assertEq(
    keysOf('<Trans i18nKey={"strategy." + aClass} />\n<Trans i18nKey={labelKey} />'),
    [],
    "Trans i18nKey concatenation and identifier are ignored"
  );
  assertEq(
    keysOf('<a>see https://x.io {t("nav.topology")}</a>\n<p>a // b</p>'),
    ["nav.topology"],
    "URL // in JSX text is not a comment"
  );
  assertEq(extractKeyRefs('/* a\n b */\nt("x.y")').map((r) => r.line), [3], "line numbers survive comment blanking");

  // Resolution: legitimate keys pass, plural siblings pass.
  assertEq(
    ["strategyVerify.platform", "strategyVerify.policy", "strategy.random", "nav.topology", "subscription.imported", "error.notFound"].map(
      (k) => resolves(catalog, k)
    ),
    [true, true, true, true, true, true],
    "legitimate keys and plural siblings resolve (no false red)"
  );
  // Resolution: the real defect shape, an absent key, and a namespace are unresolved.
  assertEq(
    ["diagnostics.portHealth.platform", "diagnostics.portHealth.policy", "topConverge.tooltip.header", "strategyVerify", "nav.missing"].map(
      (k) => resolves(catalog, k)
    ),
    [false, false, false, false, false],
    "key under a string leaf, absent key and namespace object are unresolved"
  );

  // Rust pass: error.* literals collected, comment-only mentions skipped.
  assertEq(
    extractRustErrorKeys(
      [
        '    /// existing "error.docOnly" locale key',
        '            i18n_key: "error.bindConflict".into(),',
        '        return IpcError::internal_keyed("error.notFound", raw); // "error.trailing"',
      ].join("\r\n")
    ),
    ["error.bindConflict", "error.notFound"],
    "Rust emitter: error.* literals collected, comments skipped (CRLF)"
  );
  console.log("self-test: all assertions passed");
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--self-test")) {
    selfTest();
    return;
  }
  const res = check();
  if (res.problems.length) {
    console.error(
      "i18n key references with no entry in src/locales/en/common.json (" + res.problems.length + "):"
    );
    for (const p of res.problems) console.error("  - " + p);
    console.error("Add the key to all 18 locales (then approve with node scripts/i18n-content-check.cjs --update),");
    console.error("or point the call at an existing key.");
    process.exit(1);
  }
  console.log(
    "i18n key references OK: " + res.checked + " whole-literal keys in src/ and " + res.rustKeys +
      " Rust-emitted error.* keys resolve in en"
  );
}

main();

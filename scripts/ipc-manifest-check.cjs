// ipc-manifest-check.cjs - IPC command registry guard (product invariants only).
// Keeps the registered IPC surface equal to the code truth:
// 1. extracts every #[tauri::command] fn name under src-tauri/src (CRLF/LF safe),
// 2. reconciles it against the generate_handler! registry in
// src-tauri/src/main.rs - a definition that is never registered, or a
// registry entry with no definition, is a runtime bug (the command
// silently does not exist / the router panics), so drift exits non-zero.
// The former third leg (diffing a generated docs/agents/ipc-manifest.md fence
// and an AGENTS.md pointer block) is gone: that was a documentation gate, the
// manifest file and its --write mode were retired with the governance layer.
// Duplicate command names are still a hard error (two commands cannot share
// one invoke name).
// Zero deps, style aligned with scripts/i18n-check.cjs.
//
// Usage:
// node scripts/ipc-manifest-check.cjs # check (build gate)
// node scripts/ipc-manifest-check.cjs --self-test # parser self-test

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.join(__dirname, "..");
const SRC_DIR = path.join(ROOT, "src-tauri", "src");
const MAIN_RS = path.join(SRC_DIR, "main.rs");

function listRsFiles(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...listRsFiles(p));
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
}

// Extract command names (honours rename = "..."; handles CRLF, doc comments,
// extra attributes between the command attribute and fn, and fn declared on
// the same line as the attribute). A non-fn item after the attribute resets
// the pending state, so an attribute can never bleed onto a later function.
function extractCommandNames(src) {
  const names = [];
  let pending = false;
  let rename = null;
  for (const raw of src.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.startsWith("#[tauri::command")) {
      const rm = line.match(/rename\s*=\s*"([^"]+)"/);
      rename = rm ? rm[1] : null;
      const rest = line.slice(line.indexOf("]") + 1);
      const fm = rest.match(/(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)/);
      if (fm) {
        names.push(rename || fm[1]);
        pending = false;
        rename = null;
      } else {
        pending = true;
      }
      continue;
    }
    if (!pending) continue;
    if (line === "" || line.startsWith("//")) continue;
    if (line.startsWith("#[")) {
      const rm = line.match(/rename\s*=\s*"([^"]+)"/);
      if (rm) rename = rm[1];
      continue;
    }
    const fm = line.match(/^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)/);
    if (fm) names.push(rename || fm[1]);
    pending = false;
    rename = null;
  }
  return names;
}

// Registry names from the generate_handler![...] block: line comments are
// stripped first so prose inside registry comments cannot leak in, then each
// path segment (commands::foo) contributes only its final ident.
function parseRegistry(mainSrc) {
  const m = mainSrc.match(/generate_handler!\s*\[([\s\S]*?)\]/);
  if (!m) return null;
  const body = m[1]
    .split(/\r?\n/)
    .map((l) => l.replace(/\/\/.*$/, ""))
    .join("\n");
  const re = /(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)/g;
  const names = [];
  let mm;
  while ((mm = re.exec(body)) !== null) names.push(mm[1]);
  return names;
}

function collectDefined() {
  const defined = [];
  const dups = [];
  const seen = new Set();
  for (const f of listRsFiles(SRC_DIR)) {
    const file = path.relative(ROOT, f).split(path.sep).join("/");
    for (const name of extractCommandNames(fs.readFileSync(f, "utf8"))) {
      if (seen.has(name)) dups.push(name + " (also at " + file + ")");
      seen.add(name);
      defined.push({ name: name, file: file });
    }
  }
  return { defined: defined, dups: dups };
}

function diff(a, b) {
  return a.filter((x) => !b.includes(x));
}

function check() {
  const codeErrors = [];
  const { defined, dups } = collectDefined();
  for (const d of dups) codeErrors.push("duplicate command name: " + d);
  const regNames = parseRegistry(fs.readFileSync(MAIN_RS, "utf8"));
  if (regNames === null) {
    codeErrors.push("generate_handler![...] not found in src-tauri/src/main.rs");
  } else {
    const definedNames = defined.map((c) => c.name);
    for (const n of diff(regNames, definedNames)) {
      codeErrors.push("in generate_handler! registry but no #[tauri::command] definition: " + n);
    }
    for (const n of diff(definedNames, regNames)) {
      codeErrors.push("#[tauri::command] defined but NOT registered in generate_handler!: " + n);
    }
  }
  return { codeErrors: codeErrors, defined: defined };
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

// Minimal self-test on synthetic fixtures: CRLF vs LF, rename handling,
// same-line attribute fn, non-command reset, comment-stripped registry parse.
function selfTest() {
  const crlfSrc = [
    "#[tauri::command]",
    "pub async fn alpha(state: tauri::State<'_, crate::SharedRegistry>) -> u32 { 1 }",
    "",
    '#[tauri::command(rename = "bravo_char")]',
    "fn bravo() {}",
    "",
    "/// doc comment",
    "#[derive(Clone)]",
    "struct NotACommand;",
    "fn plain_helper() {}",
  ].join("\r\n");
  assertEq(
    extractCommandNames(crlfSrc),
    ["alpha", "bravo_char"],
    "CRLF + rename + same-line attr fn + non-command reset"
  );
  assertEq(extractCommandNames(crlfSrc.replace(/\r\n/g, "\n")), ["alpha", "bravo_char"], "LF variant equals CRLF variant");
  const wrapped = [
    "#[tauri::command]",
    "// comment between attr and fn",
    "#[allow(clippy::too_many_arguments)]",
    "pub async fn charlie(",
    "  a: u32,",
    ") {}",
  ].join("\n");
  assertEq(extractCommandNames(wrapped), ["charlie"], "extra attributes + comments + wrapped signature");
  const reg = [
    "generate_handler![",
    "  commands::alpha, // keep alpha forever",
    "  commands::bravo_char,",
    "  plain::delta,",
    "]",
  ].join("\n");
  assertEq(parseRegistry(reg), ["alpha", "bravo_char", "delta"], "registry: comments stripped, path suffix captured");
  assertEq(parseRegistry("no registry here"), null, "missing generate_handler! -> null");
  assertEq(diff(["alpha", "bravo_char", "delta"], ["alpha", "bravo_char"]), ["delta"], "registry drift detected");
  console.log("self-test: all assertions passed");
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--self-test")) {
    selfTest();
    return;
  }
  const res = check();
  if (res.codeErrors.length) {
    console.error("IPC command registry drift detected (" + res.codeErrors.length + " problem(s)):");
    for (const e of res.codeErrors) console.error("  - " + e);
    process.exit(1);
  }
  console.log("IPC command registry OK: " + res.defined.length + " commands; definitions = generate_handler!");
}

main();
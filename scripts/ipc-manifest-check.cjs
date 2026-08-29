// ipc-manifest-check.cjs - IPC manifest guard (architecture-recovery ticket 03).
// Keeps the AGENTS.md command manifest equal to code truth:
//   1. extracts every #[tauri::command] fn name under src-tauri/src (CRLF/LF safe),
//   2. reconciles them against the generate_handler! registry in src-tauri/src/main.rs,
//   3. diffs both against the ipc-manifest fenced block in AGENTS.md section 7.6
//      (command name + definition-file attribution).
// Any drift exits non-zero. Style aligned with scripts/i18n-check.cjs (no deps).
//
// Usage:
//   node scripts/ipc-manifest-check.cjs            # check (build gate)
//   node scripts/ipc-manifest-check.cjs --write    # regenerate the AGENTS.md block
//   node scripts/ipc-manifest-check.cjs --self-test

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.join(__dirname, "..");
const SRC_DIR = path.join(ROOT, "src-tauri", "src");
const MAIN_RS = path.join(SRC_DIR, "main.rs");
const AGENTS_MD = path.join(ROOT, "AGENTS.md");
const FENCE_LANG = "ipc-manifest";
const FENCE = "\x60\x60\x60";

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
  const body = m[1].split(/\r?\n/).map((l) => l.replace(/\/\/.*$/, "")).join("\n");
  const re = /(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)/g;
  const names = [];
  let mm;
  while ((mm = re.exec(body)) !== null) names.push(mm[1]);
  return names;
}

function manifestRe() {
  return new RegExp(FENCE + FENCE_LANG + "\\r?\\n([\\s\\S]*?)" + FENCE);
}

function parseManifest(agentsSrc) {
  const m = agentsSrc.match(manifestRe());
  if (!m) return null;
  const entries = [];
  for (const raw of m[1].split(/\r?\n/)) {
    const t = raw.trim();
    if (t === "" || t.startsWith("#")) continue;
    const em = t.match(/^([a-z_][a-z0-9_]*)\s*=\s*(\S+)$/);
    if (!em) return null; // malformed entry counts as drift, never ignored
    entries.push({ name: em[1], file: em[2] });
  }
  return entries;
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
  const manifestErrors = [];
  const { defined, dups } = collectDefined();
  const regNames = parseRegistry(fs.readFileSync(MAIN_RS, "utf8"));
  if (regNames === null) codeErrors.push("generate_handler![...] not found in src-tauri/src/main.rs");
  const manifest = parseManifest(fs.readFileSync(AGENTS_MD, "utf8"));
  if (manifest === null) manifestErrors.push("ipc-manifest fence missing or malformed in AGENTS.md section 7.6");
  for (const d of dups) codeErrors.push("duplicate command name: " + d);
  const definedNames = defined.map((c) => c.name);
  if (regNames !== null) {
    for (const n of diff(regNames, definedNames)) codeErrors.push("in generate_handler! registry but no #[tauri::command] definition: " + n);
    for (const n of diff(definedNames, regNames)) codeErrors.push("#[tauri::command] defined but NOT registered in generate_handler!: " + n);
  }
  if (regNames !== null && manifest !== null) {
    const manifestNames = manifest.map((e) => e.name);
    for (const n of diff(definedNames, manifestNames)) manifestErrors.push("missing from AGENTS.md manifest: " + n);
    for (const n of diff(manifestNames, definedNames)) manifestErrors.push("phantom in AGENTS.md manifest (no such command in src-tauri): " + n);
    const fileOf = {};
    for (const c of defined) fileOf[c.name] = c.file;
    for (const e of manifest) {
      if (fileOf[e.name] && fileOf[e.name] !== e.file) {
        manifestErrors.push("wrong file attribution for " + e.name + ": manifest says " + e.file + ", code says " + fileOf[e.name]);
      }
    }
  }
  return { codeErrors: codeErrors, manifestErrors: manifestErrors, defined: defined };
}

function buildBlock(defined) {
  const d = new Date();
  const today = d.getFullYear() + "-" + String(d.getMonth() + 1).padStart(2, "0") + "-" + String(d.getDate()).padStart(2, "0");
  const lines = [
    "# Tauri IPC command manifest - REGENERATED by scripts/ipc-manifest-check.cjs --write.",
    "# Machine-checked on every build (pnpm ipc:check / scripts/verify-build.sh / CI):",
    "# entries must equal the #[tauri::command] set under src-tauri/src AND the",
    "# generate_handler! registry in src-tauri/src/main.rs. Format: <command> = <file>.",
    "# Do not hand-edit entries. Regenerated: " + today + " (" + defined.length + " commands)",
  ];
  for (const c of defined) lines.push(c.name + " = " + c.file);
  return lines.join("\n");
}

function writeManifest(defined) {
  let agentsSrc = fs.readFileSync(AGENTS_MD, "utf8");
  const re = new RegExp("(" + FENCE + FENCE_LANG + "\\r?\\n)([\\s\\S]*?)(?=" + FENCE + ")");
  if (!re.test(agentsSrc)) {
    console.error("--write failed: no ipc-manifest fence in AGENTS.md");
    process.exit(1);
  }
  agentsSrc = agentsSrc.replace(re, (whole, open) => open + buildBlock(defined) + "\n");
  fs.writeFileSync(AGENTS_MD, agentsSrc);
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
// comment-stripped registry parsing, fence parsing in both line endings,
// and drift detection on a manifest containing a phantom entry.
function selfTest() {
  const fence = String.fromCharCode(96, 96, 96);
  const crlfSrc = [
    "#[tauri::command]",
    "pub async fn alpha(state: tauri::State<'_, crate::SharedRegistry>) -> u32 { 1 }",
    "",
    "#[tauri::command(rename = \"bravo_char\")]",
    "fn bravo() {}",
    "",
    "/// doc comment",
    "#[derive(Clone)]",
    "struct NotACommand;",
    "fn plain_helper() {}",
  ].join("\r\n");
  assertEq(extractCommandNames(crlfSrc), ["alpha", "bravo_char"], "CRLF + rename + same-line attr fn + non-command reset");
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
  const agents = [
    "intro",
    "",
    fence + FENCE_LANG,
    "alpha = commands/mod.rs",
    "# comment line is ignored",
    "bravo_char = commands/mod.rs",
    "ghost_cmd = commands/mod.rs",
    fence,
    "",
    "outro",
  ].join("\n");
  const entries = parseManifest(agents);
  assertEq(entries.map((e) => e.name), ["alpha", "bravo_char", "ghost_cmd"], "manifest parse skips comments");
  assertEq(parseManifest(agents.replace(/\n/g, "\r\n")), entries, "manifest parse CRLF == LF");
  assertEq(parseManifest("no fence here"), null, "missing fence -> null");
  assertEq(parseManifest(fence + FENCE_LANG + "\nbad line!!\n" + fence), null, "malformed entry -> null");
  const definedNames = ["alpha", "bravo_char"];
  assertEq(diff(entries.map((e) => e.name), definedNames), ["ghost_cmd"], "phantom manifest entry detected");
  assertEq(diff(["alpha", "bravo_char", "delta"], definedNames), ["delta"], "defined/registry drift detected");
  console.log("self-test: all assertions passed");
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--self-test")) {
    selfTest();
    return;
  }
  const res = check();
  if (argv.includes("--write")) {
    // --write fixes MANIFEST drift from code truth; it only refuses when the
    // code side itself is inconsistent (definitions vs registry), because
    // regenerating from inconsistent code truth would freeze the bug in docs.
    if (res.codeErrors.length) {
      console.error("--write refused: fix code/registry drift first");
      for (const e of res.codeErrors) console.error("  - " + e);
      process.exit(1);
    }
    writeManifest(res.defined);
    console.log("AGENTS.md ipc-manifest block regenerated (" + res.defined.length + " commands)");
    return;
  }
  const errors = res.codeErrors.concat(res.manifestErrors);
  if (errors.length) {
    console.error("IPC manifest drift detected (" + errors.length + " problem(s)):");
    for (const e of errors) console.error("  - " + e);
    console.error("Fix the drift, or regenerate the manifest: node scripts/ipc-manifest-check.cjs --write");
    process.exit(1);
  }
  console.log("IPC manifest OK: " + res.defined.length + " commands; defined = registered = AGENTS.md manifest");
}

main();

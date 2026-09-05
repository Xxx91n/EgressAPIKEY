// license-field-check.cjs - license field consistency guard
// (architecture-recovery ticket 01, ADR-0067 D4).
//
// Keeps the three declared license fields equal to the repo-wide value:
//   1. package.json "license"
//   2. root Cargo.toml [workspace.package] license (member crates must use
//      `license.workspace = true`, never a hardcoded string)
//   3. README.md "## License" section body
// plus root LICENSE presence/shape (official GPL-3.0 plain text, LF, no BOM).
//
// This ticket ships the script WITHOUT mounting it: wiring into
// scripts/verify-build.sh / package.json scripts belongs to the CI-gate
// ticket (03) so multiple windows never edit verify-build concurrently.
//
// Usage:
//   node scripts/license-field-check.cjs            # check (exit 0 = green)
//
// Self-test recipe (used at ticket 01 close-out, evidence in the ticket
// report): break one field -> expect exit 1 + FAIL lines; restore -> exit 0.

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.join(__dirname, "..");
const EXPECTED = "GPL-3.0-or-later";

const checks = [];
function record(label, ok, detail) {
  checks.push({ label, ok, detail });
}

// 1. package.json
const pkgPath = path.join(ROOT, "package.json");
try {
  const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
  record(
    "package.json license",
    pkg.license === EXPECTED,
    `got ${JSON.stringify(pkg.license ?? undefined)}, want ${JSON.stringify(EXPECTED)}`,
  );
} catch (e) {
  record("package.json license", false, `unreadable: ${e.message}`);
}

// 2. Root Cargo.toml [workspace.package] license + member hardcode scan.
const cargoPath = path.join(ROOT, "Cargo.toml");
try {
  const src = fs.readFileSync(cargoPath, "utf8");
  const ws = src.split(/\r?\n/);
  const wsStart = ws.findIndex((l) => l.trim() === "[workspace.package]");
  const wsEnd =
    wsStart === -1
      ? -1
      : ws.findIndex((l, i) => i > wsStart && /^\s*\[/.test(l));
  const section = ws.slice(wsStart + 1, wsEnd === -1 ? ws.length : wsEnd);
  const m = section
    .map((l) => l.match(/^license\s*=\s*"([^"]*)"/))
    .find(Boolean);
  record(
    "Cargo.toml [workspace.package] license",
    !!m && m[1] === EXPECTED,
    m ? `got ${JSON.stringify(m[1])}, want ${JSON.stringify(EXPECTED)}` : "field missing",
  );
} catch (e) {
  record("Cargo.toml [workspace.package] license", false, `unreadable: ${e.message}`);
}

// Member crates must inherit via license.workspace, not hardcode.
const memberTomls = [
  path.join(ROOT, "crates", "resin-core", "Cargo.toml"),
  path.join(ROOT, "src-tauri", "Cargo.toml"),
];
for (const p of memberTomls) {
  const rel = path.relative(ROOT, p).replace(/\\/g, "/");
  try {
    const lines = fs.readFileSync(p, "utf8").split(/\r?\n/);
    const hard = lines.find((l) => /^license\s*=\s*"/.test(l));
    record(
      `${rel} inherits workspace license`,
      !hard,
      hard ? `hardcoded ${hard.trim()}; use license.workspace = true` : "ok",
    );
  } catch (e) {
    record(`${rel} inherits workspace license`, false, `unreadable: ${e.message}`);
  }
}

// 3. README.md "## License" section.
const readmePath = path.join(ROOT, "README.md");
try {
  const lines = fs.readFileSync(readmePath, "utf8").split(/\r?\n/);
  const h = lines.findIndex((l) => l.trim() === "## License");
  if (h === -1) {
    record('README.md "## License" section', false, "heading not found");
  } else {
    let end = lines.length;
    for (let i = h + 1; i < lines.length; i++) {
      if (/^## /.test(lines[i])) {
        end = i;
        break;
      }
    }
    const body = lines
      .slice(h + 1, end)
      .join("\n")
      .replace(/\r/g, "")
      .trim();
    record(
      'README.md "## License" section',
      body === EXPECTED,
      `got ${JSON.stringify(body)}, want ${JSON.stringify(EXPECTED)}`,
    );
  }
} catch (e) {
  record('README.md "## License" section', false, `unreadable: ${e.message}`);
}

// 4. Root LICENSE: official GPL-3.0 plain text shape, LF only, no BOM.
const licPath = path.join(ROOT, "LICENSE");
try {
  const buf = fs.readFileSync(licPath);
  const hasBom = buf[0] === 0xef && buf[1] === 0xbb && buf[2] === 0xbf;
  const text = buf.toString("utf8");
  const firstLine = text.split(/\r?\n/, 1)[0];
  const hasCr = /\r/.test(text);
  const shapeOk =
    firstLine.includes("GNU GENERAL PUBLIC LICENSE") &&
    /Version 3, 29 June 2007/.test(text) &&
    text.length > 30000;
  record(
    "LICENSE official GPL-3.0 text",
    shapeOk && !hasBom && !hasCr,
    [
      shapeOk ? null : `unexpected shape (first line ${JSON.stringify(firstLine)})`,
      hasBom ? "UTF-8 BOM present" : null,
      hasCr ? "CRLF bytes present (repo policy is LF)" : null,
    ]
      .filter(Boolean)
      .join("; ") || "ok",
  );
} catch (e) {
  record("LICENSE official GPL-3.0 text", false, `unreadable: ${e.message}`);
}

// Report.
const failed = checks.filter((c) => !c.ok);
for (const c of checks) {
  console.log(`${c.ok ? "OK  " : "FAIL"}  ${c.label}${c.ok ? "" : ` — ${c.detail}`}`);
}
if (failed.length > 0) {
  console.error(`license-field-check: FAILED (${failed.length} error(s))`);
  process.exit(1);
}
console.log(`license-field-check: OK (${checks.length} checks, expected ${JSON.stringify(EXPECTED)})`);

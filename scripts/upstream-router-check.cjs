#!/usr/bin/env node
// upstream-router-check.cjs - upstream-router link-integrity gate
// Spec D-C3.9.
//
// Locks the upstream-documentation router to reality:
//
// 1. UPSTREAM.md (docs/architecture/) is the single-page router over the
//    upstream four-pack. Its links must resolve to real repo files, the
//    four-pack anchors (RESIN_API_COVERAGE.md / RESIN_UPSTREAM_MANIFEST.yaml
//    / THIRD_PARTY.md / ADR-0067) must all be present, and the page must
//    stay a ROUTER - no fenced code blocks, no vendoring of other docs'
//    content beyond quoted file paths and decision ids.
// 2. RELEASE_NOTES.md sections must align with CHANGELOG.md versions: every
//    "## <version> (<date>)" here needs a matching "## [<version>] - <date>"
//    in the CHANGELOG with the same version AND date, and every CHANGELOG
//    version heading needs a release section here.
//
// Style follows scripts/license-field-check.cjs / readme-lang-check.cjs:
// zero deps, "OK/FAIL" lines, non-zero exit on any failure.
//
// Mounted by scripts/verify-build.sh - any failure here fails
// the whole gate.
//
// Usage:
//   node scripts/upstream-router-check.cjs   # exit 0 = green
//
// Self-test recipe:
//   break a link in docs/architecture/UPSTREAM.md (e.g. rename THIRD_PARTY.md
//   to THIRD_PART.md in the markdown link target) -> expect exit 1;
//   restore -> exit 0. Same for removing a RELEASE_NOTES.md section that the
//   CHANGELOG still has.

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.join(__dirname, "..");
const UPSTREAM = path.join(ROOT, "docs", "architecture", "UPSTREAM.md");
const RELEASE_NOTES = path.join(ROOT, "docs", "RELEASE_NOTES.md");
const CHANGELOG = path.join(ROOT, "CHANGELOG.md");

const FOUR_PACK = [
  "RESIN_API_COVERAGE.md",
  "RESIN_UPSTREAM_MANIFEST.yaml",
  "THIRD_PARTY.md",
  "0067-license-layering-provenance.md",
];

const checks = [];
function record(label, ok, detail) {
  checks.push({ label, ok, detail: ok ? "" : detail });
}

function read(p) {
  return fs.readFileSync(p, "utf8");
}

// Markdown link targets of the form ](<target>) - includes relative links
// only (skips http(s), mailto and bare anchors), strips optional anchors.
function relativeTargets(text) {
  const targets = [];
  const re = /\]\(([^)\s]+)\)/g;
  let m;
  while ((m = re.exec(text)) !== null) {
    const raw = m[1];
    if (/^(https?:|mailto:|#)/i.test(raw)) continue;
    targets.push(raw.split("#")[0]);
  }
  return targets;
}

function resolveFrom(docPath, target) {
  return path.normalize(path.join(path.dirname(docPath), decodeURIComponent(target)));
}

// 1. UPSTREAM.md link integrity: every relative markdown link must resolve
//    to an existing repo file from the doc's own directory.
let upstreamText = null;
try {
  upstreamText = read(UPSTREAM);
} catch (e) {
  console.error("upstream-router-check: cannot read " + UPSTREAM + " - " + e.message);
  process.exit(1);
}
const upTargets = relativeTargets(upstreamText);
record("UPSTREAM.md has relative links", upTargets.length >= 6,
  "only " + upTargets.length + " relative link(s) found");
const broken = [];
const seen = new Set();
for (const t of upTargets) {
  if (seen.has(t)) continue;
  seen.add(t);
  const abs = resolveFrom(UPSTREAM, t);
  if (!fs.existsSync(abs)) broken.push(t + " -> " + path.relative(ROOT, abs));
}
record("UPSTREAM.md relative links resolve (" + seen.size + " unique)", broken.length === 0,
  broken.join("; "));

// 2. The four-pack anchors are all routed from UPSTREAM.md.
const missingPack = FOUR_PACK.filter((f) => !upstreamText.includes(f));
record("UPSTREAM.md routes the upstream four-pack", missingPack.length === 0,
  "missing: " + JSON.stringify(missingPack));

// 3. Router discipline: a single-page router carries no code blocks and no
//    pasted endpoint tables (those live in the four-pack, not here).
record("UPSTREAM.md stays a router (no fenced code blocks)", !/\n\u0060\u0060\u0060/.test(upstreamText),
  "found a fenced code block - route to the owning doc instead");
record("UPSTREAM.md does not duplicate the coverage table", !/\|\s*R0?1\s*\|/.test(upstreamText),
  "looks like a pasted API-coverage row - link RESIN_API_COVERAGE.md instead");

// 4. RELEASE_NOTES.md <-> CHANGELOG.md version/date alignment.
let notesText, changelogText;
try {
  notesText = read(RELEASE_NOTES);
  changelogText = read(CHANGELOG);
} catch (e) {
  console.error("upstream-router-check: cannot read RELEASE_NOTES/CHANGELOG - " + e.message);
  process.exit(1);
}
// "## 0.1.0 (2026-09-06)" - version + ISO date, Unreleased allowed as a
// CHANGELOG-only section.
const notesRe = /^## (?!Unreleased\b)([^ (]+) \((\d{4}-\d{2}-\d{2})\)\s*$/gm;
const notesReleases = [];
let nm;
while ((nm = notesRe.exec(notesText)) !== null) notesReleases.push({ version: nm[1], date: nm[2] });
// "## [0.1.0] - 2026-09-06" - changelog side; [Unreleased] is exempt.
const clRe = /^## \[([^\]]+)\] - (\d{4}-\d{2}-\d{2})\s*$/gm;
const clReleases = [];
let cm;
while ((cm = clRe.exec(changelogText)) !== null) clReleases.push({ version: cm[1], date: cm[2] });

record("RELEASE_NOTES.md has at least one release section", notesReleases.length >= 1,
  "no '## <version> (<YYYY-MM-DD>)' headings found");
for (const r of notesReleases) {
  const match = clReleases.find((c) => c.version === r.version);
  record("RELEASE_NOTES " + r.version + " exists in CHANGELOG", !!match,
    "no '## [" + r.version + "]' heading in CHANGELOG.md");
  if (match) {
    record("RELEASE_NOTES " + r.version + " date matches CHANGELOG", match.date === r.date,
      "notes " + r.date + " vs changelog " + match.date);
  }
}
const unrouted = clReleases.filter((c) => !notesReleases.some((r) => r.version === c.version));
record("every CHANGELOG release has a RELEASE_NOTES section", unrouted.length === 0,
  "unrouted versions: " + JSON.stringify(unrouted.map((c) => c.version)));

// Report (style matches the sibling gates).
let failed = 0;
for (const c of checks) {
  if (c.ok) {
    console.log("OK    " + c.label);
  } else {
    failed++;
    console.log("FAIL  " + c.label + " - " + c.detail);
  }
}
if (failed > 0) {
  console.error("upstream-router-check: FAILED (" + failed + " error(s))");
  process.exit(1);
}
console.log("upstream-router-check: OK (" + checks.length + " checks - UPSTREAM.md link integrity + four-pack routing + router discipline + RELEASE_NOTES<->CHANGELOG alignment)");

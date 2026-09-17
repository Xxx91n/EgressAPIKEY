#!/usr/bin/env node
// readme-lang-check.cjs - bilingual README alignment guard
// (architecture-recovery ticket 03, spec D-06).
//
// Locks README_CN.md to the EN-final structure of README.md (the ticket 02
// product): section headings must correspond 1:1 through the explicit
// mapping table below (same order, no extras on either side), the License
// anchor body must equal the repo-wide license value in BOTH files, the
// language cross-links must exist, and the spec D-05 drift tokens (mihomo /
// axum / the unpublished npm global install) must stay absent from both
// homepages.
//
// "Section title sets match" is enforced as a 1:1 mapping (EN canonical
// heading <-> CN translated heading) because the CN page is a translated
// mirror, not a byte copy. The mapping table is the lock: adding, removing,
// renaming or reordering a section on one side without the other fails here.
//
// Mounted by scripts/verify-build.sh (ticket 03) alongside ticket 01's
// license-field-check.cjs - any failure here fails the whole gate.
//
// Usage:
//   node scripts/readme-lang-check.cjs     # exit 0 = green
//
// Self-test recipe (evidence in reports/03-readme-cn-sync-report.md):
// delete one section from README_CN.md -> expect exit 1; restore -> exit 0.

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.join(__dirname, "..");
const FILES = {
  en: path.join(ROOT, "README.md"),
  cn: path.join(ROOT, "README_CN.md"),
};
const LICENSE_VALUE = "GPL-3.0-or-later";

// EN heading <-> CN heading, in mirror order. This table IS the lock.
const SECTION_MAP = [
  ["What & why", "是什么与为什么"],
  ["Architecture", "架构"],
  ["Download", "下载"],
  ["Features", "特性"],
  ["Screenshots", "截图"],
  ["Quick start (development)", "快速开始（开发）"],
  ["Documentation", "文档"],
  ["Compliance notice", "合规声明"],
  ["Contributing", "参与贡献"],
  ["Third-party notices", "第三方声明"],
  ["License", "许可"],
];

// Drift tokens that must never reappear on either homepage (spec D-05,
// ADR-0050: axum + mihomo deleted; @egressapikey/server is not on npm).
const FORBIDDEN = [
  [/mihomo/i, "mihomo (ADR-0050 deleted the sidecar; CN cleanup ticket 03)"],
  [/axum/i, "axum (ADR-0050 removed the dependency)"],
  [/npm install -g @egressapikey\/server/, "npm global install of @egressapikey/server (not published; use the source-run wording)"],
];

const checks = [];
function record(label, ok, detail) {
  checks.push({ label, ok, detail: ok ? "" : detail });
}

function load(file) {
  const buf = fs.readFileSync(file);
  const text = buf.toString("utf8");
  return {
    text,
    lines: text.split(/\r?\n/),
    hasBom: buf[0] === 0xef && buf[1] === 0xbb && buf[2] === 0xbf,
    hasCr: /\r/.test(text),
  };
}

// H1 = first ATX level-1 line; sections = level-2 heading texts.
function h1Of(lines) {
  const h = lines.find((l) => /^#\s/.test(l));
  return h ? h.replace(/^#\s+/, "").trim() : null;
}
function sectionsOf(lines) {
  return lines.filter((l) => /^## /.test(l)).map((l) => l.replace(/^## /, "").trim());
}

// Body of a "## <heading>" section: lines after the heading until the next
// "## " heading or EOF, joined and trimmed.
function sectionBody(lines, heading) {
  const start = lines.findIndex(
    (l) => /^## /.test(l) && l.replace(/^## /, "").trim() === heading
  );
  if (start === -1) return null;
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    if (/^## /.test(lines[i])) {
      end = i;
      break;
    }
  }
  return lines.slice(start + 1, end).join("\n").trim();
}

let en, cn;
try {
  en = load(FILES.en);
  cn = load(FILES.cn);
} catch (e) {
  console.error("readme-lang-check: unreadable README pair - " + e.message);
  process.exit(1);
}

// 1. File shape: UTF-8 no BOM, LF only (repo file-integrity policy).
function shapeDetail(f) {
  return [f.hasBom ? "UTF-8 BOM present" : null, f.hasCr ? "CRLF bytes present" : null]
    .filter(Boolean)
    .join("; ");
}
record("README.md UTF-8 no BOM, LF only", !en.hasBom && !en.hasCr, shapeDetail(en));
record("README_CN.md UTF-8 no BOM, LF only", !cn.hasBom && !cn.hasCr, shapeDetail(cn));

// 2. H1 title identical on both pages.
const enH1 = h1Of(en.lines);
const cnH1 = h1Of(cn.lines);
record("H1 title identical", enH1 !== null && enH1 === cnH1,
  "EN " + JSON.stringify(enH1) + " vs CN " + JSON.stringify(cnH1));

// 3. Section sets correspond 1:1 to the mapping, same order, no extras.
const enSections = sectionsOf(en.lines);
const cnSections = sectionsOf(cn.lines);
const wantEn = SECTION_MAP.map((p) => p[0]);
const wantCn = SECTION_MAP.map((p) => p[1]);
record("README.md sections match the mapping (order + no extras)",
  JSON.stringify(enSections) === JSON.stringify(wantEn),
  diffDetail("EN", enSections, wantEn));
record("README_CN.md sections match the mapping (order + no extras)",
  JSON.stringify(cnSections) === JSON.stringify(wantCn),
  diffDetail("CN", cnSections, wantCn));

// 4. License anchor: both License section bodies equal the repo license value.
const enLic = sectionBody(en.lines, "License");
const cnLic = sectionBody(cn.lines, "许可");
record("README.md License anchor body", enLic === LICENSE_VALUE,
  "got " + JSON.stringify(enLic) + ", want " + JSON.stringify(LICENSE_VALUE));
record("README_CN.md License anchor body", cnLic === LICENSE_VALUE,
  "got " + JSON.stringify(cnLic) + ", want " + JSON.stringify(LICENSE_VALUE));

// 5. Language cross-links exist in both directions.
record("README.md links to the CN mirror", /\]\(README_CN\.md\)/.test(en.text),
  "expected a README_CN.md markdown link");
record("README_CN.md links to the EN page", /\]\(README\.md\)/.test(cn.text),
  "expected a README.md markdown link");

// 6. D-05 drift tokens stay banned from both homepages.
for (const pair of [["README.md", en.text], ["README_CN.md", cn.text]]) {
  const hits = FORBIDDEN.filter((f) => f[0].test(pair[1])).map((f) => f[1]);
  record(pair[0] + " free of D-05 drift tokens", hits.length === 0, hits.join("; "));
}

function diffDetail(side, got, want) {
  const missing = want.filter((h) => !got.includes(h));
  const extra = got.filter((h) => !want.includes(h));
  const orderNote =
    missing.length === 0 && extra.length === 0 && JSON.stringify(got) !== JSON.stringify(want)
      ? " (same set, different order)"
      : "";
  return side + " headings got " + JSON.stringify(got) + "; expected " + JSON.stringify(want)
    + (missing.length ? "; missing: " + JSON.stringify(missing) : "")
    + (extra.length ? "; unexpected: " + JSON.stringify(extra) : "")
    + orderNote;
}


// 7 (ticket 10). Sync-comment format on both homepages: first line must be
// "<!-- synced-with: <counterpart> @ <40-hex sha> -->" (spec D-C3.2).
const SYNC_RE = /^<!-- synced-with: (README\.md|README_CN\.md) @ ([0-9a-f]{40}) -->$/;
for (const pair of [["README.md", en, "README_CN.md"], ["README_CN.md", cn, "README.md"]]) {
  const first = pair[1].lines[0] || "";
  const m = first.match(SYNC_RE);
  record(
    pair[0] + " commit-hash sync comment (format + counterpart)",
    m !== null && m[1] === pair[2],
    "first line " + JSON.stringify(first) + " does not match '<!-- synced-with: " + pair[2] + " @ <sha> -->'"
  );
}

// 9 (ticket 14, spec D-C3.9). Homepage Documentation-table coverage for the
// upstream-router pair: both homepages must link docs/architecture/UPSTREAM.md
// and docs/RELEASE_NOTES.md (the ticket 14 products) so the router page and
// the per-release notes stay reachable from the front doors.
const REQUIRED_DOC_LINKS = [
  "docs/architecture/UPSTREAM.md",
  "docs/RELEASE_NOTES.md",
];
for (const pair of [["README.md", en], ["README_CN.md", cn]]) {
  const missing = REQUIRED_DOC_LINKS.filter((l) => !pair[1].text.includes(l));
  record(
    pair[0] + " links the upstream-router pair (UPSTREAM + RELEASE_NOTES)",
    missing.length === 0,
    missing.length ? "missing: " + JSON.stringify(missing) : ""
  );
}

// 8 (ticket 10). Mirror coverage: assets/readme hero + architecture + 3 screenshot
// slots must be referenced in BOTH homepages (same relative paths, D-C3.1/D-C3.2).
const REQUIRED_ASSETS = [
  "assets/readme/hero.svg",
  "assets/readme/hero-dark.svg",
  "assets/readme/architecture.svg",
  "assets/readme/topology.png",
  "assets/readme/platforms.png",
  "assets/readme/effective-config.png",
];
for (const pair of [["README.md", en], ["README_CN.md", cn]]) {
  const missing = REQUIRED_ASSETS.filter((a) => !pair[1].text.includes(a));
  record(
    pair[0] + " references all 6 readme assets (mirror coverage)",
    missing.length === 0,
    missing.length ? "missing: " + JSON.stringify(missing) : ""
  );
}

// 18 (ticket 06, spec IMP-6 #5). Fenced-code info-string mirror. The CN page
// is a translated mirror, so every fenced block must carry the SAME info
// string (mermaid / bash / ...), in the SAME order, on both sides - a
// translation that drops or renames a fence silently breaks Mermaid rendering
// and syntax highlighting. Unbalanced fences fail too: an unclosed fence
// swallows the rest of the page.
function fenceInfoStrings(lines) {
  const re = /^\s{0,3}([`~])\1{2,}\s*(\S*)/;
  const out = [];
  let open = null;
  for (const line of lines) {
    const m = line.match(re);
    if (!m) continue;
    if (open === null) {
      open = m[1];
      out.push(m[2]);
    } else if (m[1] === open && m[2] === "") {
      open = null;
    }
  }
  return { fences: out, balanced: open === null };
}
const enFences = fenceInfoStrings(en.lines);
const cnFences = fenceInfoStrings(cn.lines);
record(
  "code fence info strings mirror EN<->CN (same order, fences balanced)",
  enFences.balanced && cnFences.balanced &&
    JSON.stringify(enFences.fences) === JSON.stringify(cnFences.fences),
  "EN " + JSON.stringify(enFences.fences) + (enFences.balanced ? "" : " [unclosed]") +
    " vs CN " + JSON.stringify(cnFences.fences) + (cnFences.balanced ? "" : " [unclosed]")
);

// Report (style matches scripts/license-field-check.cjs).
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
  console.error("readme-lang-check: FAILED (" + failed + " error(s))");
  process.exit(1);
}
console.log("readme-lang-check: OK (" + checks.length + " checks - EN<->CN structure, License anchors, cross-links, D-05 tokens, sync comments, asset coverage, upstream-router pair coverage, code-fence info strings)");
